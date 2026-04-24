//! The public report shape — what `observe` returns and what every
//! front-end (CLI, HTML export, local server, MCP) renders.
//!
//! Kept flat and small so it serializes cleanly and stays stable across
//! rule / engine changes. Every new datum needs a new field on a named
//! struct; avoid ad-hoc `serde_json::Value` grab-bags.

use serde::{Deserialize, Serialize};

use crate::aggregate::{ErrorBucket, IxMetrics, RecentFailure};

/// The window an observe pass covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveWindow {
    /// Requested window in seconds (caller-facing; e.g. 86400 for 24h).
    pub seconds: u64,
    /// Transactions pulled. May be fewer than the limit when the
    /// program's tx volume in the window is below the limit.
    pub tx_count: usize,
    /// Earliest signature block-time in the sampled set (unix seconds).
    /// `None` when no transactions were fetched.
    pub earliest_block_time: Option<i64>,
    /// Most recent signature block-time (unix seconds). Lets callers
    /// compute the actual covered window if the program was idle for
    /// part of the requested range.
    pub latest_block_time: Option<i64>,
}

/// Upgrade-authority and upgrade-history signal lifted from the
/// BPF-loader-upgradeable program-data account. All fields are
/// optional because older / non-upgradeable programs don't carry
/// some of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpgradeHistory {
    /// Current upgrade authority (base58). `None` if the program is
    /// final / immutable.
    pub authority: Option<String>,
    /// Slot of the last deploy. `None` when the ProgramData account
    /// hasn't been decoded yet — MVP leaves this unfilled and later
    /// releases populate it.
    pub last_deploy_slot: Option<u64>,
    /// Unix seconds for the last deploy, resolved via `getBlockTime`.
    pub last_deploy_time: Option<i64>,
    /// Total number of observed upgrade transactions in the sampled
    /// window. Cheap proxy for "how often do you deploy?"
    pub upgrades_in_window: u64,
}

/// Per-account-type count in the current program state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCount {
    /// Account struct name (from the IDL).
    pub name: String,
    /// Total accounts owned by the program with this discriminator.
    pub count: u64,
}

/// The full observe output. Stable shape across CLI / export / MCP.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveReport {
    /// Base58 program id the report covers.
    pub program_id: String,
    /// Human-readable program name from the IDL metadata.
    pub program_name: Option<String>,
    /// Window + counts.
    pub window: ObserveWindow,
    /// Per-instruction metrics, sorted by call volume descending.
    pub instructions: Vec<IxMetrics>,
    /// Error-code buckets aggregated across all instructions, sorted
    /// by occurrence count descending.
    pub errors: Vec<ErrorBucket>,
    /// Most recent failures with decoded account inputs + args. Capped
    /// by the caller via `ObserveOpts::recent_failures_limit` (MVP
    /// hard-codes 10 until watch mode lands).
    pub recent_failures: Vec<RecentFailure>,
    /// Upgrade history signal. Absent on MVP runs; populated in a
    /// follow-up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upgrade_history: Option<UpgradeHistory>,
    /// Per-account-type counts. Absent on MVP runs; populated when
    /// `--account-counts` is passed and the RPC tier supports
    /// `getProgramAccounts` with memcmp.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub account_counts: Vec<AccountCount>,
}
