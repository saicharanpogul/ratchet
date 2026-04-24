//! `ratchet` — upgrade-safety checks for Solana programs.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ratchet_anchor::{
    fetch_account_data, fetch_idl_account, fetch_idl_for_program, load_idl_from_file, normalize,
    Cluster,
};
use ratchet_core::{
    check, default_preflight_rules, default_rules, preflight, CheckContext, ProgramSurface, Report,
    Severity,
};
use ratchet_lock::{Lockfile, DEFAULT_FILENAME};
use ratchet_observe::redact_rpc_url;
use ratchet_source::parse_dir;
use ratchet_squads::{decode_vault_transaction, ProposalKind};
use ratchet_svm::{
    fetch_program_accounts, validate_surface, verify_deploy, verify_sbf_program_file, DeployReport,
    SbfProgramInfo,
};

#[derive(Debug, Parser)]
#[command(
    name = "ratchet",
    version,
    about = "Upgrade-safety checks for Solana programs"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Emit JSON instead of human-readable output.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lint a single IDL for mainnet-readiness. Runs the P-series rules
    /// (version fields, reserved padding, explicit discriminators,
    /// name collisions, unsignered writes) on one program surface.
    /// Use before first deploy; use `check-upgrade` for upgrades.
    Readiness(ReadinessArgs),
    /// Compare a new program surface against the deployed one, a prior
    /// snapshot, or a committed `ratchet.lock`, and report every breaking
    /// or unsafe change.
    CheckUpgrade(CheckUpgradeArgs),
    /// Write a `ratchet.lock` snapshot from a program surface. The snapshot
    /// is what `check-upgrade --lock` later compares against.
    Lock(LockArgs),
    /// Sample live program-owned accounts and verify they match the new
    /// IDL's layout. Catches 'old-layout accounts never migrated' failures
    /// that static rules miss.
    Replay(ReplayArgs),
    /// Summarise a Squads V4 vault-transaction proposal. Fetches the
    /// account, classifies it (program upgrade / set-authority / other),
    /// and lists referenced pubkeys for signer triage.
    Squads(SquadsArgs),
    /// Observe a deployed program over a time window: per-instruction
    /// success rate + error distribution, CU percentiles, recent
    /// failures with decoded account inputs. The third lens after
    /// `readiness` (pre-deploy) and `check-upgrade` (pre-release).
    Observe(ObserveArgs),
    /// Start a Model Context Protocol server on stdio. Exposes every
    /// ratchet capability (readiness, check-upgrade, observe, rule
    /// catalogs) as MCP tools so an agent — Claude Code, Cursor,
    /// Windsurf, any MCP-aware client — can drive ratchet directly.
    Mcp,
    /// List every registered rule with its one-line description.
    ListRules,
}

#[derive(Debug, Args)]
struct CheckUpgradeArgs {
    /// Path to the new (candidate) IDL JSON. Typically
    /// `target/idl/<program>.json` from an Anchor build.
    #[arg(long)]
    new: PathBuf,

    /// Path to the old (deployed / baseline) IDL JSON.
    #[arg(long, group = "baseline")]
    old: Option<PathBuf>,

    /// Path to a committed `ratchet.lock` to use as the baseline.
    #[arg(long, group = "baseline")]
    lock: Option<PathBuf>,

    /// Program id whose on-chain IDL should be fetched as the baseline.
    /// `ratchet` derives the Anchor IDL account address from the program id
    /// (`create_with_seed(find_program_address(&[], pid).0, "anchor:idl", pid)`)
    /// and reads it over `--cluster`.
    #[arg(long, group = "baseline")]
    program: Option<String>,

    /// Explicit Anchor IDL account pubkey to fetch as the baseline.
    #[arg(long, group = "baseline")]
    idl_account: Option<String>,

    /// Cluster shorthand (mainnet, devnet, testnet) or an explicit RPC URL.
    #[arg(long, default_value = "mainnet")]
    cluster: String,

    /// Acknowledge an `unsafe-*` finding and demote it to additive. Repeat
    /// for multiple flags: `--unsafe allow-rename --unsafe allow-type-change`.
    #[arg(long = "unsafe", value_name = "FLAG")]
    unsafes: Vec<String>,

    /// Account name that has a declared migration (Anchor 1.0+
    /// `Migration<From, To>` or a manual `realloc` handler). Repeatable.
    #[arg(long = "migrated-account", value_name = "NAME")]
    migrated_accounts: Vec<String>,

    /// Account name whose Anchor definition carries a `realloc = ...`
    /// constraint, meaning every instruction that touches the account
    /// will automatically resize it. Demotes R005 appends to Additive
    /// with a realloc-specific message. Repeatable. Source parsing
    /// (`--new-source`) populates this automatically when it spots the
    /// attribute.
    #[arg(long = "realloc-account", value_name = "NAME")]
    realloc_accounts: Vec<String>,

    /// Anchor program source directory. When set, ratchet parses
    /// `#[account(seeds = [...])]` attributes and augments the new
    /// surface with seed components the IDL may have lost.
    #[arg(long = "new-source", value_name = "DIR")]
    new_source: Option<PathBuf>,

    /// Same, for the old surface. Only useful when the baseline comes
    /// from --old rather than a lock or RPC (locks capture source-augmented
    /// seeds when they were written).
    #[arg(long = "old-source", value_name = "DIR")]
    old_source: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ReplayArgs {
    /// Path to the new IDL JSON whose account layouts will validate samples.
    #[arg(long)]
    new: PathBuf,

    /// Program id to sample accounts from.
    #[arg(long)]
    program: String,

    /// Cluster shorthand (mainnet, devnet, testnet) or RPC URL.
    #[arg(long, default_value = "mainnet")]
    cluster: String,

    /// Maximum number of accounts to sample from the program.
    #[arg(long, default_value_t = 100)]
    limit: usize,

    /// Path to the candidate program `.so`. When provided, the file's
    /// ELF header is verified (magic, class, endianness, SBF machine
    /// type) before the account-sample replay runs.
    #[arg(long)]
    so: Option<PathBuf>,

    /// Also deploy `--so` into an in-process LiteSVM instance. Requires
    /// the `litesvm-deploy` build feature; without it the flag errors
    /// with a clear message pointing at the build invocation.
    #[arg(long, requires = "so")]
    deploy: bool,
}

#[derive(Debug, Args)]
struct ReadinessArgs {
    /// Path to the IDL JSON to lint. Typically `target/idl/<program>.json`
    /// from an Anchor build.
    #[arg(long)]
    new: PathBuf,

    /// Acknowledge a P-rule finding and demote it to additive. Repeat
    /// per flag: `--unsafe allow-no-version-field --unsafe allow-no-signer`.
    #[arg(long = "unsafe", value_name = "FLAG")]
    unsafes: Vec<String>,
}

#[derive(Debug, Args)]
struct SquadsArgs {
    /// Squads V4 `VaultTransaction` account pubkey.
    #[arg(long)]
    proposal: String,

    /// Cluster shorthand (mainnet, devnet, testnet) or RPC URL.
    #[arg(long, default_value = "mainnet")]
    cluster: String,

    /// After decoding the proposal, fetch the current on-chain IDL for
    /// the extracted program id and run `check-upgrade` against the
    /// local IDL path provided via `--new`. Only applies when the
    /// proposal is classified as a program upgrade.
    #[arg(long)]
    auto_diff: bool,

    /// Candidate IDL JSON used by `--auto-diff`.
    #[arg(long, requires = "auto_diff")]
    new: Option<PathBuf>,

    /// Acknowledge an `unsafe-*` finding during auto-diff. Same
    /// semantics as on `check-upgrade`.
    #[arg(long = "unsafe", value_name = "FLAG", requires = "auto_diff")]
    unsafes: Vec<String>,

    /// Account that has a declared migration, for auto-diff.
    #[arg(long = "migrated-account", value_name = "NAME", requires = "auto_diff")]
    migrated_accounts: Vec<String>,
}

#[derive(Debug, Args)]
struct ObserveArgs {
    /// Base58 program id to observe.
    #[arg(long)]
    program: String,

    /// Cluster shorthand (mainnet, devnet, testnet) or a full RPC URL.
    /// Helius / QuickNode URLs with a tier-appropriate API key produce
    /// the fastest results; stock public endpoints will work but may
    /// rate-limit on high-volume programs. The docs in the README call
    /// out when to reach for a paid tier.
    #[arg(long, default_value = "mainnet")]
    cluster: String,

    /// Time window to cover, as a `24h` / `7d` / `30m` string. Default
    /// 24h.
    #[arg(long = "since", default_value = "24h")]
    window: String,

    /// Cap transactions fetched. Guards against unbounded RPC cost on
    /// very busy programs — raise when your program's throughput
    /// justifies it. Default 1000.
    #[arg(long, default_value_t = 1000)]
    limit: usize,

    /// Path to a local IDL JSON to use instead of fetching from the
    /// program's on-chain IDL account. Useful when the program hasn't
    /// published its IDL on-chain, or when iterating against a local
    /// build.
    #[arg(long)]
    idl: Option<PathBuf>,

    /// Also run `getProgramAccounts` per account type in the IDL and
    /// report per-type counts. Off by default because the RPC call is
    /// expensive and often rate-limited on free tiers.
    #[arg(long)]
    account_counts: bool,

    /// Milliseconds to sleep between batched `getTransaction` calls.
    /// Defaults to 250ms which keeps a Helius Developer-tier paid key
    /// under the per-second method-credit ceiling. Crank up (e.g.
    /// 1000) for free tiers, or drop to 0 on a private RPC where
    /// throughput isn't rate-limited. If you see 429 warnings in the
    /// output, this is the knob.
    #[arg(long = "pace-ms", value_name = "MS", default_value_t = 250)]
    pace_ms: u64,

    /// Fail (exit 1) when any ix's error rate exceeds this percentage.
    /// Accepts a float: `--alert-error-rate 5` == 5%.
    #[arg(long = "alert-error-rate", value_name = "PCT")]
    alert_error_rate: Option<f64>,

    /// Limit `--alert-error-rate` to a single ix (optional). When
    /// omitted, the threshold applies to every ix in the report.
    #[arg(long = "alert-error-rate-ix", value_name = "IX")]
    alert_error_rate_ix: Option<String>,

    /// Fail when the observed tx count in the window drops below this
    /// floor — outage / dropped-traffic detection.
    #[arg(long = "alert-min-tx", value_name = "N")]
    alert_min_tx: Option<usize>,

    /// Fail when any ix's CU p99 exceeds this value. Catches
    /// post-deploy efficiency regressions.
    #[arg(long = "alert-cu-p99", value_name = "CU")]
    alert_cu_p99: Option<u64>,

    /// Limit `--alert-cu-p99` to a single ix.
    #[arg(long = "alert-cu-p99-ix", value_name = "IX")]
    alert_cu_p99_ix: Option<String>,

    /// Re-run the observation on a loop every `INTERVAL`
    /// (`--watch 5m`, `--watch 30s`). Each cycle persists a snapshot
    /// to `--db` and prints a delta summary against the previous
    /// snapshot. Stops on Ctrl-C.
    #[arg(long, value_name = "INTERVAL")]
    watch: Option<String>,

    /// SQLite path for persisted snapshots. Default
    /// `~/.ratchet/observe/<program>.db`. Use `:memory:` when you
    /// don't want a file on disk (each watch cycle still prints a
    /// delta against the previous in-process snapshot).
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,

    /// Also write a self-contained HTML report to this path. Static,
    /// single file, no external fetches at render time — safe to
    /// attach to a PR description or share in Slack. Runs alongside
    /// the normal human / JSON output.
    #[arg(long = "export-html", value_name = "PATH")]
    export_html: Option<PathBuf>,

    /// Serve a live dashboard on localhost. Pairs naturally with
    /// `--watch`: the background thread re-runs the observation on
    /// the watch interval and the page auto-polls `/api/report`.
    /// Default bind address `127.0.0.1:8787`.
    #[arg(long)]
    ui: bool,

    /// Override the UI bind address. Set to `0.0.0.0:8787` for
    /// self-hosted deployments where you want the dashboard
    /// reachable over the network. Be deliberate — no auth is
    /// shipped, so exposing it to the internet is on you.
    #[arg(
        long = "ui-addr",
        value_name = "ADDR",
        default_value = "127.0.0.1:8787"
    )]
    ui_addr: String,
}

#[derive(Debug, Args)]
struct LockArgs {
    /// Source IDL path.
    #[arg(long, group = "source")]
    from_idl: Option<PathBuf>,

    /// IDL account pubkey to fetch.
    #[arg(long, group = "source")]
    idl_account: Option<String>,

    /// Program id; IDL account is derived from it and fetched over --cluster.
    #[arg(long, group = "source")]
    program: Option<String>,

    /// Cluster shorthand (mainnet, devnet, testnet) or RPC URL.
    #[arg(long, default_value = "mainnet")]
    cluster: String,

    /// Optional Anchor program source directory. When provided, the
    /// locked surface is augmented with richer PDA seed info parsed
    /// from source.
    #[arg(long = "source-dir", value_name = "DIR")]
    source_dir: Option<PathBuf>,

    /// Output path for the lockfile.
    #[arg(long, default_value = DEFAULT_FILENAME)]
    out: PathBuf,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("ratchet: {e:#}");
            ExitCode::from(3)
        }
    }
}

fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Readiness(args) => readiness(args, cli.json),
        Command::CheckUpgrade(args) => check_upgrade(args, cli.json),
        Command::Lock(args) => lock(args, cli.json),
        Command::Replay(args) => replay(args, cli.json),
        Command::Squads(args) => squads(args, cli.json),
        Command::Observe(args) => observe(args, cli.json),
        Command::Mcp => {
            // The MCP server owns stdin+stdout — cli.json is meaningless
            // here. Log banner to stderr so the protocol stream stays
            // parseable.
            eprintln!("ratchet mcp: serving on stdio");
            ratchet_mcp::run(std::io::stdin().lock(), std::io::stdout().lock())?;
            Ok(0)
        }
        Command::ListRules => {
            list_rules(cli.json);
            Ok(0)
        }
    }
}

fn readiness(args: ReadinessArgs, as_json: bool) -> Result<i32> {
    let surface = normalize(&load_idl_from_file(&args.new)?)?;

    let mut ctx = CheckContext::new();
    for flag in &args.unsafes {
        ctx = ctx.with_allow(flag.trim_start_matches("--"));
    }

    let rules = default_preflight_rules();
    let report = preflight(&surface, &ctx, &rules);

    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_readiness_human(&report, &surface.name);
    }

    Ok(report.exit_code())
}

fn render_readiness_human(report: &Report, program_name: &str) {
    if report.findings.is_empty() {
        println!(
            "no readiness findings — `{program_name}` looks mainnet-shaped against the 6 P-rules"
        );
        return;
    }

    render_findings(report);
    println!();
    match report.max_severity() {
        Some(Severity::Breaking) => {
            println!(
                "verdict: BREAKING — `{program_name}` has issues that will cause problems on mainnet"
            );
        }
        Some(Severity::Unsafe) => {
            println!(
                "verdict: UNSAFE — `{program_name}` has future-upgrade concerns; review each finding and either fix or acknowledge with --unsafe <flag>"
            );
        }
        Some(Severity::Additive) | None => {
            println!("verdict: ready — only informational findings");
        }
    }
}

fn squads(args: SquadsArgs, as_json: bool) -> Result<i32> {
    let cluster = Cluster::parse(&args.cluster);
    let data = fetch_account_data(&cluster, &args.proposal)
        .with_context(|| format!("fetching Squads proposal {}", args.proposal))?;
    let summary = decode_vault_transaction(&data)?;

    let auto_diff_report = if args.auto_diff {
        Some(run_auto_diff(&args, &summary, &cluster)?)
    } else {
        None
    };

    if as_json {
        let payload = serde_json::json!({
            "summary": summary,
            "check_upgrade": auto_diff_report,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        render_squads_human(&args, &summary);
        if let Some(report) = &auto_diff_report {
            println!();
            render_human(report);
        }
    }

    if let Some(report) = &auto_diff_report {
        return Ok(report.exit_code());
    }
    Ok(0)
}

fn render_squads_human(args: &SquadsArgs, summary: &ratchet_squads::VaultTransactionSummary) {
    let label = match &summary.kind {
        ProposalKind::ProgramUpgrade { .. } => "PROGRAM UPGRADE",
        ProposalKind::SetUpgradeAuthority => "SET UPGRADE AUTHORITY",
        ProposalKind::Other => "other / unrecognised",
    };
    println!("proposal: {}", args.proposal);
    println!("kind:     {label}");
    println!("size:     {} bytes", summary.account_size);
    if let ProposalKind::ProgramUpgrade { program_id, buffer } = &summary.kind {
        if let Some(p) = program_id {
            println!("program:  {p}");
        }
        if let Some(b) = buffer {
            println!("buffer:   {b}");
        }
    }
    if !summary.referenced_pubkeys.is_empty() {
        println!("referenced pubkeys ({}):", summary.referenced_pubkeys.len());
        for k in &summary.referenced_pubkeys {
            println!("  {k}");
        }
    }
    if matches!(summary.kind, ProposalKind::ProgramUpgrade { .. }) && !args.auto_diff {
        println!(
            "\nhint: rerun with --auto-diff --new <IDL> to have ratchet fetch the current\n\
             on-chain IDL and diff it against your candidate."
        );
    }
}

fn run_auto_diff(
    args: &SquadsArgs,
    summary: &ratchet_squads::VaultTransactionSummary,
    cluster: &Cluster,
) -> Result<Report> {
    let new_path = args
        .new
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--auto-diff requires --new <IDL_PATH>"))?;
    let program_id = match &summary.kind {
        ProposalKind::ProgramUpgrade { program_id, .. } => program_id.as_deref(),
        ProposalKind::SetUpgradeAuthority => {
            bail!("proposal is a set-upgrade-authority change, not a program upgrade; --auto-diff does not apply")
        }
        ProposalKind::Other => {
            bail!("proposal is not recognised as a BPF loader operation; --auto-diff cannot run")
        }
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "program_id could not be extracted from the proposal — fall back to running \
             `ratchet check-upgrade --program <PID>` manually"
        )
    })?;

    let old_idl = fetch_idl_for_program(cluster, program_id)
        .with_context(|| format!("fetching current IDL for program {program_id}"))?;
    let old = normalize(&old_idl)?;
    let new = normalize(&load_idl_from_file(new_path)?)?;

    let mut ctx = CheckContext::new();
    for flag in &args.unsafes {
        ctx = ctx.with_allow(flag.trim_start_matches("--"));
    }
    for name in &args.migrated_accounts {
        ctx = ctx.with_migration(name);
    }

    Ok(check(&old, &new, &ctx, &default_rules()))
}

fn replay(args: ReplayArgs, as_json: bool) -> Result<i32> {
    let surface = normalize(&load_idl_from_file(&args.new)?)?;

    let binary_info = if let Some(so_path) = &args.so {
        Some(
            verify_sbf_program_file(so_path)
                .with_context(|| format!("verifying program binary at {}", so_path.display()))?,
        )
    } else {
        None
    };

    let deploy_report = if args.deploy {
        let so_path = args.so.as_ref().expect("clap enforces --so with --deploy");
        let bytes =
            std::fs::read(so_path).with_context(|| format!("reading {}", so_path.display()))?;
        Some(verify_deploy(&args.program, &bytes).context("running LiteSVM deploy smoke test")?)
    } else {
        None
    };

    let cluster = Cluster::parse(&args.cluster);
    let samples = fetch_program_accounts(&cluster, &args.program, args.limit)
        .with_context(|| format!("sampling accounts from program {}", args.program))?;
    let report = validate_surface(&surface, &samples);

    if as_json {
        let payload = serde_json::json!({
            "binary": binary_info,
            "deploy": deploy_report,
            "report": report,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        if let Some(info) = &binary_info {
            render_binary_info(info, args.so.as_ref().unwrap());
        }
        if let Some(d) = &deploy_report {
            render_deploy(d);
        }
        render_replay(&report);
    }

    let deploy_failed = deploy_report
        .as_ref()
        .map(|d| !d.deploy_succeeded)
        .unwrap_or(false);
    Ok(if report.is_clean() && !deploy_failed {
        0
    } else {
        1
    })
}

fn render_deploy(d: &DeployReport) {
    if d.deploy_succeeded {
        println!(
            "deploy ok: {} loaded into LiteSVM successfully",
            d.program_id
        );
    } else {
        println!(
            "deploy FAILED: {} rejected by LiteSVM{}",
            d.program_id,
            d.error
                .as_ref()
                .map(|e| format!(" — {e}"))
                .unwrap_or_default()
        );
    }
}

fn render_binary_info(info: &SbfProgramInfo, path: &std::path::Path) {
    println!(
        "binary ok: {} ({} bytes, machine={:#x}, e_flags={:#x} [{}], {}-bit, {}-endian, {})",
        path.display(),
        info.size_bytes,
        info.machine,
        info.e_flags,
        ratchet_svm::sbpf_version_hint(info.e_flags),
        if info.elf_class_64 { 64 } else { 32 },
        if info.little_endian { "little" } else { "big" },
        if info.is_shared_object {
            "shared-object"
        } else {
            "not shared"
        },
    );
}

fn render_replay(report: &ratchet_svm::ReplayReport) {
    println!(
        "sampled {} accounts; {} matched cleanly",
        report.total_samples,
        report.total_samples - report.failing()
    );
    for (ty, tally) in &report.tallies_by_type {
        println!(
            "  {ty}: {ok} ok, {under} undersized, {unk} unknown",
            ok = tally.ok,
            under = tally.undersized,
            unk = tally.unknown,
        );
    }
    let failures: Vec<_> = report
        .verdicts
        .iter()
        .filter(|v| !matches!(v, ratchet_svm::AccountVerdict::Ok { .. }))
        .collect();
    if failures.is_empty() {
        return;
    }
    println!("\nfailing accounts (showing up to 20):");
    for f in failures.iter().take(20) {
        match f {
            ratchet_svm::AccountVerdict::Undersized {
                pubkey,
                account_type,
                actual,
                expected_min,
            } => {
                println!(
                    "  UNDERSIZED {pubkey}  type={account_type}  got {actual}B, expected >= {expected_min}B"
                );
            }
            ratchet_svm::AccountVerdict::UnknownDiscriminator {
                pubkey,
                discriminator,
            } => {
                let hex: String = discriminator.iter().map(|b| format!("{b:02x}")).collect();
                println!("  UNKNOWN    {pubkey}  disc=0x{hex}");
            }
            ratchet_svm::AccountVerdict::Malformed { pubkey, reason } => {
                println!("  MALFORMED  {pubkey}  {reason}");
            }
            ratchet_svm::AccountVerdict::Ok { .. } => {}
        }
    }
}

fn list_rules(as_json: bool) {
    let rules = default_rules();
    if as_json {
        let entries: Vec<_> = rules
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id(),
                    "name": r.name(),
                    "description": r.description(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".into())
        );
        return;
    }
    if rules.is_empty() {
        println!("(no rules registered yet)");
        return;
    }
    for r in &rules {
        println!("{}  {}  {}", r.id(), r.name(), r.description());
    }
}

fn check_upgrade(args: CheckUpgradeArgs, as_json: bool) -> Result<i32> {
    let mut new = load_new(&args)?;
    let mut old = load_old(&args)?;

    let mut ctx = CheckContext::new();

    if let Some(dir) = &args.new_source {
        augment_from_source(&mut new, dir, "new", as_json, &mut ctx)?;
    }
    if let Some(dir) = &args.old_source {
        augment_from_source(&mut old, dir, "old", as_json, &mut ctx)?;
    }

    for flag in &args.unsafes {
        ctx = ctx.with_allow(flag.trim_start_matches("--"));
    }
    for name in &args.migrated_accounts {
        ctx = ctx.with_migration(name);
    }
    for name in &args.realloc_accounts {
        ctx = ctx.with_realloc(name);
    }

    let rules = default_rules();
    let report = check(&old, &new, &ctx, &rules);

    if as_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        render_human(&report);
    }

    Ok(report.exit_code())
}

fn load_new(args: &CheckUpgradeArgs) -> Result<ProgramSurface> {
    let idl = load_idl_from_file(&args.new)?;
    normalize(&idl)
}

fn load_old(args: &CheckUpgradeArgs) -> Result<ProgramSurface> {
    if let Some(path) = &args.old {
        let idl = load_idl_from_file(path)?;
        return normalize(&idl);
    }
    if let Some(path) = &args.lock {
        let lock =
            Lockfile::read(path).with_context(|| format!("reading lockfile {}", path.display()))?;
        // Tamper / mismatch check: lock's bound program identity must
        // agree with the candidate's.
        let new = load_idl_from_file(&args.new)
            .and_then(|idl| normalize(&idl))
            .with_context(|| format!("reading --new {}", args.new.display()))?;
        lock.ensure_matches(&new)
            .with_context(|| format!("comparing lockfile {} against --new", path.display()))?;
        return Ok(lock.surface);
    }
    if let Some(pubkey) = &args.idl_account {
        let cluster = Cluster::parse(&args.cluster);
        let idl = fetch_idl_account(&cluster, pubkey)?;
        return normalize(&idl);
    }
    if let Some(program_id) = &args.program {
        let cluster = Cluster::parse(&args.cluster);
        let idl = fetch_idl_for_program(&cluster, program_id)?;
        return normalize(&idl);
    }
    bail!("need one of --old <PATH>, --lock <PATH>, --idl-account <PUBKEY>, or --program <PID>")
}

fn lock(args: LockArgs, as_json: bool) -> Result<i32> {
    let mut surface = if let Some(path) = &args.from_idl {
        normalize(&load_idl_from_file(path)?)?
    } else if let Some(pubkey) = &args.idl_account {
        let cluster = Cluster::parse(&args.cluster);
        normalize(&fetch_idl_account(&cluster, pubkey)?)?
    } else if let Some(program_id) = &args.program {
        let cluster = Cluster::parse(&args.cluster);
        normalize(&fetch_idl_for_program(&cluster, program_id)?)?
    } else {
        bail!("need one of --from-idl <PATH>, --idl-account <PUBKEY>, or --program <PID>");
    };

    if let Some(dir) = &args.source_dir {
        // lock uses a throwaway ctx since realloc info doesn't affect what
        // we write — only R005 cares about it at check time.
        let mut throwaway_ctx = CheckContext::new();
        augment_from_source(&mut surface, dir, "lock", as_json, &mut throwaway_ctx)?;
    }

    let lockfile = Lockfile::of(surface);
    lockfile
        .write(&args.out)
        .with_context(|| format!("writing {}", args.out.display()))?;

    if as_json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "wrote": args.out.display().to_string(),
                "name": lockfile.surface.name,
                "program_id": lockfile.surface.program_id,
            })
        );
    } else {
        println!(
            "wrote {} (program `{}`{}{} accounts, {} instructions)",
            args.out.display(),
            lockfile.surface.name,
            match &lockfile.surface.program_id {
                Some(pid) => format!(", {pid}, "),
                None => ", ".into(),
            },
            lockfile.surface.accounts.len(),
            lockfile.surface.instructions.len()
        );
    }

    Ok(0)
}

fn augment_from_source(
    surface: &mut ProgramSurface,
    dir: &std::path::Path,
    side: &str,
    quiet: bool,
    ctx: &mut CheckContext,
) -> Result<()> {
    let scan =
        parse_dir(dir).with_context(|| format!("scanning {side} source at {}", dir.display()))?;
    let applied = scan.patch.apply_to(surface);

    // Auto-populate realloc-aware demotion for R005. We intentionally
    // only touch the "new" side — the old surface's realloc attributes
    // don't change the forward-compatibility verdict.
    let mut realloc_added = 0usize;
    if side == "new" {
        for name in &scan.realloc_accounts {
            if !ctx.has_realloc(name) {
                *ctx = std::mem::take(ctx).with_realloc(name);
                realloc_added += 1;
            }
        }
    }

    if !quiet {
        let unresolved = if scan.unresolved_structs.is_empty() {
            String::new()
        } else {
            format!(
                " ({} struct(s) had no Context<_> binding)",
                scan.unresolved_structs.len()
            )
        };
        let realloc = if realloc_added > 0 {
            format!(", auto-declared realloc for {realloc_added} account(s)")
        } else {
            String::new()
        };
        eprintln!(
            "ratchet: parsed {} .rs file(s) in {side} source, filled {} PDA slot(s){}{}",
            scan.files_parsed, applied, unresolved, realloc,
        );
    }
    Ok(())
}

/// Print every finding in the report. Caller is responsible for the
/// verdict banner and any "no findings" message — that copy differs
/// between `check-upgrade` and `readiness`.
fn render_findings(report: &Report) {
    for f in &report.findings {
        let label = match f.severity {
            Severity::Breaking => "BREAKING",
            Severity::Unsafe => "UNSAFE  ",
            Severity::Additive => "additive",
        };
        println!(
            "{label}  {}  {}  {}",
            f.rule_id,
            f.rule_name,
            f.path.join("/")
        );
        println!("          {}", f.message);
        if let Some(old) = &f.old {
            println!("          - {old}");
        }
        if let Some(new) = &f.new {
            println!("          + {new}");
        }
        if let Some(s) = &f.suggestion {
            println!("          hint: {s}");
        }
        if let Some(flag) = &f.allow_flag {
            println!("          (acknowledge with --unsafe {flag})");
        }
    }
}

fn render_human(report: &Report) {
    if report.findings.is_empty() {
        println!("no findings — upgrade is safe");
        return;
    }
    render_findings(report);
    println!();
    match report.max_severity() {
        Some(Severity::Breaking) => {
            println!("verdict: BREAKING — upgrade will corrupt data or break clients");
        }
        Some(Severity::Unsafe) => {
            println!("verdict: UNSAFE — upgrade needs a declared migration or --unsafe flag");
        }
        Some(Severity::Additive) | None => {
            println!("verdict: safe");
        }
    }
}

fn observe(args: ObserveArgs, as_json: bool) -> Result<i32> {
    let cluster = Cluster::parse(&args.cluster);
    let window_seconds = parse_duration(&args.window)
        .with_context(|| format!("parsing --since {:?}", args.window))?;

    let idl_override =
        match &args.idl {
            Some(path) => Some(load_idl_from_file(path).with_context(|| {
                format!("loading --idl from {} (fallback path)", path.display())
            })?),
            None => None,
        };

    let opts = ratchet_observe::ObserveOpts {
        program_id: args.program.clone(),
        window_seconds,
        limit: args.limit,
        idl_override,
        include_account_counts: args.account_counts,
        pace_ms: args.pace_ms,
        // Live progress goes to stderr; suppress on --json so the
        // pipe-able output channel stays untouched. Watch / UI modes
        // inherit this: stderr frames during the first prime, silent
        // during subsequent background cycles keeps the dashboard
        // lifecycle quiet.
        show_progress: !as_json,
    };

    let alert_config = ratchet_observe::AlertConfig {
        max_error_rate_pct: args.alert_error_rate,
        error_rate_ix: args.alert_error_rate_ix.clone(),
        min_tx_count: args.alert_min_tx,
        max_cu_p99: args.alert_cu_p99,
        cu_p99_ix: args.alert_cu_p99_ix.clone(),
    };

    let store = observe_store(&args)?;

    let export_html = args.export_html.clone();

    if args.ui {
        let addr: std::net::SocketAddr = args
            .ui_addr
            .parse()
            .with_context(|| format!("parsing --ui-addr {:?}", args.ui_addr))?;
        let interval_secs = match &args.watch {
            Some(i) => parse_duration(i).with_context(|| format!("parsing --watch {i:?}"))?,
            None => 60, // sensible default for a live dashboard
        };
        return observe_ui(
            cluster,
            opts,
            alert_config,
            store,
            export_html,
            addr,
            interval_secs,
        );
    }

    match &args.watch {
        None => observe_one_shot(
            &cluster,
            &opts,
            &alert_config,
            store.as_ref(),
            export_html.as_deref(),
            as_json,
        ),
        Some(interval) => {
            let interval_secs = parse_duration(interval)
                .with_context(|| format!("parsing --watch {interval:?}"))?;
            observe_watch(
                &cluster,
                &opts,
                &alert_config,
                store.as_ref(),
                export_html.as_deref(),
                interval_secs,
                as_json,
            )
        }
    }
}

/// Serve a live dashboard. Runs the observe loop on a background
/// thread so the HTTP server can block on the main thread (keeps the
/// process structure small — no tokio runtime needed). Returns only
/// when the server exits (Ctrl-C / upstream kill), so the exit code
/// is effectively 0 on clean shutdown.
fn observe_ui(
    cluster: Cluster,
    opts: ratchet_observe::ObserveOpts,
    alert_config: ratchet_observe::AlertConfig,
    store: Option<ratchet_observe::store::Store>,
    export_html: Option<PathBuf>,
    addr: std::net::SocketAddr,
    interval_seconds: u64,
) -> Result<i32> {
    use std::sync::{Arc, RwLock};
    use std::thread;
    use std::time::Duration;

    // First observation blocks the server start — better a 2-second
    // wait at launch than a blank dashboard on first paint.
    eprintln!("ratchet observe --ui: priming first observation...");
    let initial = ratchet_observe::observe(&cluster, &opts)
        .with_context(|| format!("priming observation for {}", opts.program_id))?;
    if let Some(s) = &store {
        let _ = s.insert(&initial, i64_now());
    }
    if let Some(path) = &export_html {
        if let Err(e) = write_html_export(path, &initial) {
            eprintln!("warn: initial html export failed: {e:#}");
        }
    }

    let slot: ratchet_observe::ui::ReportSlot = Arc::new(RwLock::new(initial));
    let worker_slot = Arc::clone(&slot);
    // Silence the stderr progress frames on background cycles — the
    // prime run already showed them during initial load, and in UI
    // mode the user is watching the browser, not the terminal. A
    // continuously-updating progress line from every watch cycle
    // competes with whatever else they're doing in that shell.
    let worker_opts = ratchet_observe::ObserveOpts {
        show_progress: false,
        ..opts.clone()
    };
    let worker_cluster = cluster.clone();
    let worker_alert = alert_config.clone();
    let worker_store_is_some = store.is_some();
    let worker_store = store;
    let worker_export = export_html;

    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(interval_seconds));
        match ratchet_observe::observe(&worker_cluster, &worker_opts) {
            Ok(report) => {
                let breaches = ratchet_observe::evaluate_alerts(&report, &worker_alert);
                for b in &breaches {
                    eprintln!("alert [{}] {}", b.rule, b.message);
                }
                if worker_store_is_some {
                    if let Some(s) = &worker_store {
                        if let Err(e) = s.insert(&report, i64_now()) {
                            eprintln!("warn: store insert failed: {e:#}");
                        }
                    }
                }
                if let Some(path) = &worker_export {
                    if let Err(e) = write_html_export(path, &report) {
                        eprintln!("warn: html export failed: {e:#}");
                    }
                }
                if let Ok(mut slot) = worker_slot.write() {
                    *slot = report;
                }
            }
            Err(e) => eprintln!("warn: observation failed: {e:#}"),
        }
    });

    ratchet_observe::ui::serve(addr, slot)?;
    Ok(0)
}

fn observe_store(args: &ObserveArgs) -> Result<Option<ratchet_observe::store::Store>> {
    let path = resolve_store_path(args)?;
    match path {
        Some(p) if p.as_os_str() == ":memory:" => {
            Ok(Some(ratchet_observe::store::Store::in_memory()?))
        }
        Some(p) => {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating store directory {}", parent.display()))?;
            }
            Ok(Some(ratchet_observe::store::Store::open(&p)?))
        }
        None => Ok(None),
    }
}

/// Default snapshot path unless the dev overrides with `--db`.
/// Watch mode and UI mode both want persistence automatically —
/// watch for the Δ-since-last summary, UI so the first dashboard
/// refresh has a baseline to diff against. One-shot runs stay
/// zero-filesystem-footprint unless the dev opts in.
fn resolve_store_path(args: &ObserveArgs) -> Result<Option<PathBuf>> {
    if let Some(p) = &args.db {
        return Ok(Some(p.clone()));
    }
    if args.watch.is_some() || args.ui {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| anyhow::anyhow!("$HOME not set; pass --db explicitly"))?;
        let mut path = PathBuf::from(home);
        path.push(".ratchet");
        path.push("observe");
        path.push(format!("{}.db", args.program));
        return Ok(Some(path));
    }
    Ok(None)
}

fn observe_one_shot(
    cluster: &Cluster,
    opts: &ratchet_observe::ObserveOpts,
    alert_config: &ratchet_observe::AlertConfig,
    store: Option<&ratchet_observe::store::Store>,
    export_html: Option<&std::path::Path>,
    as_json: bool,
) -> Result<i32> {
    let previous = store.and_then(|s| s.latest_before(&opts.program_id, i64_now()).ok().flatten());
    let report = ratchet_observe::observe(cluster, opts)
        .with_context(|| format!("observing program {}", opts.program_id))?;
    let breaches = ratchet_observe::evaluate_alerts(&report, alert_config);

    if let Some(s) = store {
        if let Err(e) = s.insert(&report, i64_now()) {
            eprintln!("warn: store insert failed: {e:#}");
        }
    }

    if let Some(path) = export_html {
        write_html_export(path, &report)?;
    }

    emit_observe_result(
        cluster.url(),
        &report,
        &breaches,
        previous.as_ref().map(|(_, r)| r),
        as_json,
    )?;

    Ok(if breaches.is_empty() { 0 } else { 1 })
}

fn observe_watch(
    cluster: &Cluster,
    opts: &ratchet_observe::ObserveOpts,
    alert_config: &ratchet_observe::AlertConfig,
    store: Option<&ratchet_observe::store::Store>,
    export_html: Option<&std::path::Path>,
    interval_seconds: u64,
    as_json: bool,
) -> Result<i32> {
    // Watch semantically requires a store: the delta-vs-previous column
    // is what makes the loop interesting. Keep the `:memory:` fallback
    // for dry-run / test use; persistent store is the normal path.
    let store = store.ok_or_else(|| {
        anyhow::anyhow!("--watch requires a store; pass --db <path> or set $HOME for the default")
    })?;

    eprintln!(
        "ratchet observe --watch {}: press Ctrl-C to stop",
        fmt_seconds(interval_seconds)
    );
    let mut cycle = 0u64;
    loop {
        let previous = store
            .latest_before(&opts.program_id, i64_now())
            .ok()
            .flatten();

        let report = match ratchet_observe::observe(cluster, opts) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warn: observe cycle {cycle} failed: {e:#}");
                std::thread::sleep(std::time::Duration::from_secs(interval_seconds));
                cycle += 1;
                continue;
            }
        };
        let breaches = ratchet_observe::evaluate_alerts(&report, alert_config);
        if let Err(e) = store.insert(&report, i64_now()) {
            eprintln!("warn: store insert failed: {e:#}");
        }

        if let Some(path) = export_html {
            if let Err(e) = write_html_export(path, &report) {
                eprintln!("warn: html export failed: {e:#}");
            }
        }

        emit_observe_result(
            cluster.url(),
            &report,
            &breaches,
            previous.as_ref().map(|(_, r)| r),
            as_json,
        )?;

        cycle += 1;
        std::thread::sleep(std::time::Duration::from_secs(interval_seconds));
    }
}

fn write_html_export(
    path: &std::path::Path,
    report: &ratchet_observe::ObserveReport,
) -> Result<()> {
    let html = ratchet_observe::render_html(report).context("rendering HTML export")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating export parent directory {}", parent.display()))?;
    }
    std::fs::write(path, html)
        .with_context(|| format!("writing HTML export to {}", path.display()))?;
    eprintln!("ratchet observe: wrote {}", path.display());
    Ok(())
}

fn emit_observe_result(
    cluster_url: &str,
    report: &ratchet_observe::ObserveReport,
    breaches: &[ratchet_observe::AlertBreach],
    previous: Option<&ratchet_observe::ObserveReport>,
    as_json: bool,
) -> Result<()> {
    if as_json {
        let envelope = serde_json::json!({
            "report": report,
            "alerts": breaches,
            "previous_tx_count": previous.map(|r| r.window.tx_count),
        });
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        render_observe_human(report, cluster_url);
        if let Some(prev) = previous {
            render_delta(prev, report);
        }
        if !breaches.is_empty() {
            render_alert_breaches(breaches);
        }
    }
    Ok(())
}

fn render_delta(prev: &ratchet_observe::ObserveReport, cur: &ratchet_observe::ObserveReport) {
    println!();
    println!("Δ since last snapshot");
    println!("────────────────────────────────────────────────────────────────────");
    let tx_delta = cur.window.tx_count as i64 - prev.window.tx_count as i64;
    println!(
        "tx count:        {} → {} ({:+})",
        prev.window.tx_count, cur.window.tx_count, tx_delta
    );

    use std::collections::HashMap;
    let prev_by_name: HashMap<&str, f64> = prev
        .instructions
        .iter()
        .filter_map(|ix| ix.success_rate.map(|r| (ix.name.as_str(), r)))
        .collect();
    for ix in &cur.instructions {
        if let (Some(prev_rate), Some(cur_rate)) =
            (prev_by_name.get(ix.name.as_str()), ix.success_rate)
        {
            let delta_pp = (cur_rate - prev_rate) * 100.0;
            if delta_pp.abs() >= 0.1 {
                println!(
                    "{:<24} success {:>5.1}% → {:>5.1}% ({:+.1} pp)",
                    ix.name,
                    prev_rate * 100.0,
                    cur_rate * 100.0,
                    delta_pp
                );
            }
        }
    }
}

fn i64_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn render_alert_breaches(breaches: &[ratchet_observe::AlertBreach]) {
    println!();
    println!("Alerts");
    println!("────────────────────────────────────────────────────────────────────");
    for b in breaches {
        println!("[{}] {}", b.rule, b.message);
    }
}

/// Parse a `24h` / `7d` / `30m` / `600s` duration into seconds. Accepts
/// a bare integer as seconds for explicit callers. The set of suffixes
/// is deliberately small so CI pipelines don't accidentally get cute.
fn parse_duration(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty duration");
    }
    let (num, unit) = match s.chars().last().unwrap() {
        c if c.is_ascii_digit() => (s, "s"),
        'h' | 'd' | 'm' | 's' => s.split_at(s.len() - 1),
        other => bail!("unknown duration unit {other:?} (use h/d/m/s)"),
    };
    let n: u64 = num
        .parse()
        .with_context(|| format!("parsing numeric part of duration {s:?}"))?;
    let secs = match unit {
        "s" | "" => n,
        "m" => n * 60,
        "h" => n * 60 * 60,
        "d" => n * 60 * 60 * 24,
        _ => bail!("unreachable unit"),
    };
    Ok(secs)
}

fn render_observe_human(report: &ratchet_observe::ObserveReport, cluster_url: &str) {
    let name = report.program_name.as_deref().unwrap_or("<unnamed>");
    println!("ratchet observe — {}", name);
    println!("PID:      {}", report.program_id);
    println!("cluster:  {}", redact_rpc_url(cluster_url));
    println!(
        "window:   {}  ({} transactions)",
        fmt_seconds(report.window.seconds),
        report.window.tx_count
    );
    println!();

    if report.instructions.is_empty() {
        println!("No instructions decoded in window. The program may not have seen");
        println!("traffic from this account, or the IDL's instruction discriminators");
        println!("don't match any of the observed transactions.");
        return;
    }

    println!("Instructions");
    println!("────────────────────────────────────────────────────────────────────");
    println!(
        "{:<20} {:>8} {:>8}   {:>8}   {:>8}   {:>6}",
        "ix", "count", "✓ %", "CU p50", "CU p99", "errors"
    );
    for ix in &report.instructions {
        let rate = ix
            .success_rate
            .map(|r| format!("{:>6.1}%", r * 100.0))
            .unwrap_or_else(|| "    — ".into());
        let p50 = ix
            .cu_p50
            .map(|v| format!("{v:>8}"))
            .unwrap_or_else(|| "       —".into());
        let p99 = ix
            .cu_p99
            .map(|v| format!("{v:>8}"))
            .unwrap_or_else(|| "       —".into());
        println!(
            "{:<20} {:>8} {:>8}   {}   {}   {:>6}",
            ix.name, ix.count, rate, p50, p99, ix.error_count
        );
    }

    if !report.errors.is_empty() {
        println!();
        println!("Errors");
        println!("────────────────────────────────────────────────────────────────────");
        for e in &report.errors {
            let label = e
                .name
                .as_deref()
                .map(|n| format!("{n} (0x{:04x})", e.code))
                .unwrap_or_else(|| format!("0x{:04x}", e.code));
            let from = if e.ix_names.is_empty() {
                String::from("(unknown ix)")
            } else {
                format!("from: {}", e.ix_names.join(", "))
            };
            println!("{:<36} {:>5}   {}", label, e.count, from);
        }
    }

    if !report.recent_failures.is_empty() {
        println!();
        println!("Recent failures");
        println!("────────────────────────────────────────────────────────────────────");
        for f in &report.recent_failures {
            let ix = f.ix_name.as_deref().unwrap_or("<unknown ix>");
            let err = f
                .error_name
                .as_deref()
                .map(|n| format!("{n} (0x{:04x})", f.error_code.unwrap_or(0)))
                .unwrap_or_else(|| {
                    f.error_code
                        .map(|c| format!("0x{c:04x}"))
                        .unwrap_or_else(|| "<unresolved>".into())
                });
            println!("{}  →  {}", ix, err);
            if let Some(fp) = &f.fee_payer {
                println!("    user:  {}", fp);
            }
            println!("    sig:   {}", f.signature);
        }
    }

    if let Some(hist) = &report.upgrade_history {
        println!();
        println!("Upgrade history");
        println!("────────────────────────────────────────────────────────────────────");
        let auth = hist.authority.as_deref().unwrap_or("<immutable>");
        println!("authority:    {}", auth);
        if let Some(slot) = hist.last_deploy_slot {
            println!("last slot:    {}", slot);
        }
        if let Some(ts) = hist.last_deploy_time {
            println!("last deploy:  {}", fmt_relative_time(ts));
        }
    }

    if !report.account_counts.is_empty() {
        println!();
        println!("Accounts");
        println!("────────────────────────────────────────────────────────────────────");
        for a in &report.account_counts {
            println!("{:<24} {:>8}", a.name, a.count);
        }
    }
}

fn fmt_relative_time(unix_seconds: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let diff = now.saturating_sub(unix_seconds);
    if diff < 60 {
        format!("{diff}s ago")
    } else if diff < 60 * 60 {
        format!("{}m ago", diff / 60)
    } else if diff < 60 * 60 * 24 {
        format!("{}h ago", diff / (60 * 60))
    } else {
        format!("{}d ago", diff / (60 * 60 * 24))
    }
}

fn fmt_seconds(s: u64) -> String {
    if s % (60 * 60 * 24) == 0 {
        format!("{}d", s / (60 * 60 * 24))
    } else if s % (60 * 60) == 0 {
        format!("{}h", s / (60 * 60))
    } else if s % 60 == 0 {
        format!("{}m", s / 60)
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parse_duration_accepts_all_units() {
        assert_eq!(parse_duration("30s").unwrap(), 30);
        assert_eq!(parse_duration("10m").unwrap(), 600);
        assert_eq!(parse_duration("24h").unwrap(), 86_400);
        assert_eq!(parse_duration("7d").unwrap(), 7 * 86_400);
    }

    #[test]
    fn parse_duration_accepts_bare_seconds() {
        assert_eq!(parse_duration("600").unwrap(), 600);
    }

    #[test]
    fn parse_duration_rejects_unknown_unit() {
        assert!(parse_duration("5y").is_err());
    }

    #[test]
    fn fmt_seconds_picks_sensible_unit() {
        assert_eq!(fmt_seconds(86_400), "1d");
        assert_eq!(fmt_seconds(3600), "1h");
        assert_eq!(fmt_seconds(600), "10m");
        assert_eq!(fmt_seconds(45), "45s");
    }

    // URL redaction is now implemented and tested in ratchet-observe
    // (`crates/ratchet-observe/src/redact.rs`) — one source of truth
    // so the fetch layer, CLI render, and any future hosted surface
    // share the same scrubber. The re-export is covered by a single
    // smoke test here to lock the CLI consumer path.
    #[test]
    fn cli_reexports_the_shared_redactor() {
        assert_eq!(
            redact_rpc_url("https://mainnet.helius-rpc.com/?api-key=placeholder"),
            "https://mainnet.helius-rpc.com/?api-key=***"
        );
    }
}
