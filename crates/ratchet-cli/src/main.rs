//! `ratchet` — upgrade-safety checks for Solana programs.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use ratchet_anchor::{
    fetch_idl_account, fetch_idl_for_program, load_idl_from_file, normalize, Cluster,
};
use ratchet_core::{check, default_rules, CheckContext, ProgramSurface, Report, Severity};
use ratchet_lock::{Lockfile, DEFAULT_FILENAME};
use ratchet_source::parse_dir;

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
        Command::ListRules => {
            list_rules(cli.json);
            Ok(0)
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
