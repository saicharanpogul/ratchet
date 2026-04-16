//! `ratchet` — upgrade-safety checks for Solana programs.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ratchet_anchor::{
    fetch_account_data, fetch_idl_account, fetch_idl_for_program, load_idl_from_file, normalize,
    Cluster,
};
use ratchet_core::{check, default_rules, CheckContext, ProgramSurface, Report, Severity};
use ratchet_lock::{Lockfile, DEFAULT_FILENAME};
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
        Command::CheckUpgrade(args) => check_upgrade(args, cli.json),
        Command::Lock(args) => lock(args, cli.json),
        Command::Replay(args) => replay(args, cli.json),
        Command::Squads(args) => squads(args, cli.json),
        Command::ListRules => {
            list_rules(cli.json);
            Ok(0)
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
        println!(
            "referenced pubkeys ({}):",
            summary.referenced_pubkeys.len()
        );
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
        let bytes = std::fs::read(so_path)
            .with_context(|| format!("reading {}", so_path.display()))?;
        Some(
            verify_deploy(&args.program, &bytes)
                .context("running LiteSVM deploy smoke test")?,
        )
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
    Ok(if report.is_clean() && !deploy_failed { 0 } else { 1 })
}

fn render_deploy(d: &DeployReport) {
    if d.deploy_succeeded {
        println!("deploy ok: {} loaded into LiteSVM successfully", d.program_id);
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
        "binary ok: {} ({} bytes, machine={:#x}, {}-bit, {}-endian, {})",
        path.display(),
        info.size_bytes,
        info.machine,
        if info.elf_class_64 { 64 } else { 32 },
        if info.little_endian { "little" } else { "big" },
        if info.is_shared_object { "shared-object" } else { "not shared" },
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

    if let Some(dir) = &args.new_source {
        augment_from_source(&mut new, dir, "new")?;
    }
    if let Some(dir) = &args.old_source {
        augment_from_source(&mut old, dir, "old")?;
    }

    let mut ctx = CheckContext::new();
    for flag in &args.unsafes {
        ctx = ctx.with_allow(flag.trim_start_matches("--"));
    }
    for name in &args.migrated_accounts {
        ctx = ctx.with_migration(name);
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
        augment_from_source(&mut surface, dir, "lock")?;
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
) -> Result<()> {
    let scan =
        parse_dir(dir).with_context(|| format!("scanning {side} source at {}", dir.display()))?;
    let applied = scan.patch.apply_to(surface);
    eprintln!(
        "ratchet: parsed {} .rs file(s) in {side} source, filled {} PDA slot(s){}",
        scan.files_parsed,
        applied,
        if scan.unresolved_structs.is_empty() {
            "".into()
        } else {
            format!(
                " ({} struct(s) had no Context<_> binding)",
                scan.unresolved_structs.len()
            )
        }
    );
    Ok(())
}

fn render_human(report: &Report) {
    if report.findings.is_empty() {
        println!("no findings — upgrade is safe");
        return;
    }

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
