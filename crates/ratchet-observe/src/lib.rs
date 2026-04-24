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
pub mod decode;
#[cfg(feature = "rpc")]
pub mod fetch;
pub mod report;
#[cfg(feature = "store")]
pub mod store;

pub use aggregate::{ErrorBucket, IxMetrics, RecentFailure};
pub use report::{ObserveReport, ObserveWindow};

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
}

impl Default for ObserveOpts {
    fn default() -> Self {
        Self {
            program_id: String::new(),
            window_seconds: 24 * 60 * 60,
            limit: 1000,
            idl_override: None,
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
/// 5. Aggregate into [`ObserveReport`].
///
/// The `rpc` feature gates this; consumers holding their own tx stream
/// can call the aggregator directly.
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

    let txs = fetch::fetch_transactions(cluster, &sigs).context("fetching transactions")?;

    let decoded = decode::decode_all(&idl, &txs);
    Ok(aggregate::summarise(opts, &idl, decoded))
}
