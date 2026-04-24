//! IDL-aware observability for deployed Solana programs.
//!
//! `solana-ratchet-observe` is the third lens on a program after the
//! diff-time and single-IDL checks the other ratchet crates cover.
//! The composition:
//!
//! - [`ratchet_core::preflight`] — is my program mainnet-shaped *before*
//!   first deploy?
//! - [`ratchet_core::check`] — will this upgrade corrupt state, clients,
//!   or PDAs?
//! - **this crate** — now that it's live, how is it actually doing?
//!
//! The engine (this crate) is UI-agnostic. It turns an RPC endpoint +
//! program id + time window into a typed [`ObserveReport`]. Front-ends
//! wrap it differently:
//!
//! - `ratchet observe` CLI → human or JSON stdout
//! - `ratchet observe --ui` → local HTTP server rendering the same data
//! - `ratchet observe --export html` → self-contained report file
//! - `ratchet mcp` → Model Context Protocol server for agent use
//!
//! The crate is split so each front-end only pulls what it needs —
//! `rpc` feature gates the HTTP layer, `store` gates SQLite persistence
//! for watch mode, and the core aggregation + report types are always
//! available.

#![deny(rustdoc::broken_intra_doc_links)]

pub mod aggregate;
pub mod alert;
pub mod decode;
pub mod export;
#[cfg(feature = "rpc")]
pub mod fetch;
pub mod redact;
pub mod report;
#[cfg(feature = "store")]
pub mod store;
#[cfg(feature = "ui")]
pub mod ui;
pub mod upgrade;

pub use aggregate::{ErrorBucket, IxMetrics, RecentFailure};
pub use alert::{evaluate as evaluate_alerts, AlertBreach, AlertConfig};
pub use export::render_html;
pub use redact::{redact_error_message, redact_rpc_url};
pub use report::{AccountCount, ObserveReport, ObserveWindow, UpgradeHistory};

/// Top-level configuration for a single [`observe`] run.
#[derive(Debug, Clone)]
pub struct ObserveOpts {
    /// Base58-encoded Solana program id to observe.
    pub program_id: String,
    /// Time window to cover, expressed as a Unix-duration in seconds.
    /// The engine pulls signatures newer than `now - window_seconds`.
    pub window_seconds: u64,
    /// Maximum number of transactions to pull. Protects against
    /// unbounded RPC cost on high-volume programs. Default: 1000.
    pub limit: usize,
    /// Optional pre-loaded Anchor IDL. When `None`, the engine fetches
    /// the IDL from the program's on-chain IDL account (rpc feature).
    pub idl_override: Option<ratchet_anchor::AnchorIdl>,
    /// When `true`, runs `getProgramAccounts` with a memcmp filter per
    /// account def in the IDL to populate per-type counts. Off by
    /// default because `getProgramAccounts` is expensive and often
    /// rate-limited on free RPC tiers — callers must opt in.
    pub include_account_counts: bool,
    /// Milliseconds to sleep between batched `getTransaction` calls.
    /// Most paid RPC tiers count each method in a JSON-RPC batch
    /// against a per-second credit budget; pacing keeps sustained
    /// pulls below the ceiling without requiring caller-side math.
    ///
    /// Default 250ms matches Helius Developer at 50-wide batches;
    /// set to 0 to disable, or raise for free tiers and crank
    /// higher for `Business+` when you know what you're doing.
    pub pace_ms: u64,
    /// Emit progress updates to stderr during the fetch phases (sig
    /// pagination + tx batches). Off by default so the library stays
    /// silent for programmatic consumers (MCP, hosted dashboards);
    /// the CLI flips it on for human output and off for `--json`.
    pub show_progress: bool,
}

impl Default for ObserveOpts {
    fn default() -> Self {
        Self {
            program_id: String::new(),
            window_seconds: 24 * 60 * 60,
            limit: 1000,
            idl_override: None,
            include_account_counts: false,
            pace_ms: 250,
            show_progress: false,
        }
    }
}

#[cfg(feature = "rpc")]
pub use fetch::{Cluster, FetchError};

/// Orchestrate one end-to-end observe pass:
///
/// 1. Resolve the IDL (from override or on-chain).
/// 2. Page signatures for the program over the window.
/// 3. Batch-fetch transactions for each signature.
/// 4. Decode each tx against the IDL (ix name, error name).
/// 5. Fetch BPF-loader upgrade metadata (best-effort).
/// 6. Optionally count accounts by discriminator (opt-in).
/// 7. Aggregate into [`ObserveReport`].
///
/// Upgrade-metadata and account-count fetches are best-effort —
/// failures log to `eprintln!` and leave the corresponding report
/// fields empty rather than killing the whole run. That matters for
/// CI: a dev pointing `ratchet observe` at a still-deploying program
/// shouldn't get an exit-3 when the ProgramData account temporarily
/// fails to resolve.
#[cfg(feature = "rpc")]
pub fn observe(cluster: &Cluster, opts: &ObserveOpts) -> anyhow::Result<ObserveReport> {
    use anyhow::Context;

    let idl = match &opts.idl_override {
        Some(idl) => idl.clone(),
        None => ratchet_anchor::fetch_idl_for_program(cluster, &opts.program_id)
            .with_context(|| format!("fetching IDL for {}", opts.program_id))?,
    };

    let sigs = fetch::signatures_within_window(cluster, opts)
        .context("fetching transaction signatures")?;

    let txs = fetch::fetch_transactions(cluster, &sigs, opts.pace_ms, opts.show_progress)
        .context("fetching transactions")?;

    let decoded = decode::decode_all(&idl, &txs);
    let mut report = aggregate::summarise(opts, &idl, decoded);

    report.upgrade_history = fetch_upgrade_history(cluster, &opts.program_id);

    if opts.include_account_counts {
        report.account_counts = fetch_account_counts(cluster, &opts.program_id, &idl);
    }

    Ok(report)
}

/// Best-effort upgrade metadata fetch. Logs and returns `None` on any
/// RPC / shape failure so the rest of the report still renders.
#[cfg(feature = "rpc")]
fn fetch_upgrade_history(cluster: &Cluster, program_id: &str) -> Option<UpgradeHistory> {
    match fetch_upgrade_history_inner(cluster, program_id) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("warn: upgrade history unavailable: {e:#}");
            None
        }
    }
}

#[cfg(feature = "rpc")]
fn fetch_upgrade_history_inner(
    cluster: &Cluster,
    program_id: &str,
) -> anyhow::Result<UpgradeHistory> {
    use anyhow::Context;

    let program_bytes = fetch::fetch_account_bytes(cluster, program_id)
        .with_context(|| format!("getAccountInfo({program_id})"))?
        .ok_or_else(|| anyhow::anyhow!("program account {program_id} not found"))?;
    let rec = upgrade::parse_program_record(&program_bytes).context("decoding program record")?;
    let programdata_b58 = bs58::encode(rec.programdata_address).into_string();

    let data_bytes = fetch::fetch_account_bytes(cluster, &programdata_b58)
        .with_context(|| format!("getAccountInfo({programdata_b58})"))?
        .ok_or_else(|| anyhow::anyhow!("programdata account {programdata_b58} not found"))?;
    let hdr =
        upgrade::parse_program_data_header(&data_bytes).context("decoding programdata header")?;

    let last_deploy_time = fetch::fetch_block_time(cluster, hdr.last_deploy_slot)
        .ok()
        .flatten();

    Ok(UpgradeHistory {
        authority: hdr.upgrade_authority.map(|k| bs58::encode(k).into_string()),
        last_deploy_slot: Some(hdr.last_deploy_slot),
        last_deploy_time,
        // Wire accurate counts in a follow-up — MVP ships the current
        // snapshot, not the history count.
        upgrades_in_window: 0,
    })
}

/// Run `getProgramAccounts` per account def in the IDL. Failures on
/// individual accounts are logged and skipped — we'd rather ship a
/// partial count than kill the whole report because one
/// discriminator timed out.
#[cfg(feature = "rpc")]
fn fetch_account_counts(
    cluster: &Cluster,
    program_id: &str,
    idl: &ratchet_anchor::AnchorIdl,
) -> Vec<report::AccountCount> {
    use sha2::{Digest, Sha256};

    let mut out = Vec::with_capacity(idl.accounts.len());
    for acc in &idl.accounts {
        let disc = acc.discriminator.unwrap_or_else(|| {
            // Anchor's canonical default: `sha256("account:<Name>")[..8]`.
            let mut hasher = Sha256::new();
            hasher.update(b"account:");
            hasher.update(acc.name.as_bytes());
            let digest: [u8; 32] = hasher.finalize().into();
            let mut prefix = [0u8; 8];
            prefix.copy_from_slice(&digest[..8]);
            prefix
        });
        match fetch::count_accounts_by_discriminator(cluster, program_id, &disc) {
            Ok(count) => out.push(report::AccountCount {
                name: acc.name.clone(),
                count,
            }),
            Err(e) => eprintln!("warn: account count for {} failed: {e}", acc.name),
        }
    }
    out
}
