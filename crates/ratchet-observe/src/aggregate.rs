//! Turn a stream of decoded transactions into the [`ObserveReport`]
//! shape. All numeric roll-ups live here so the CLI / UI / MCP renders
//! stay dumb.

use std::collections::BTreeMap;

use ratchet_anchor::AnchorIdl;
use serde::{Deserialize, Serialize};

use crate::decode::DecodedTx;
use crate::report::{ObserveReport, ObserveWindow};
use crate::ObserveOpts;

/// Per-instruction metrics. All percentile fields are `None` when the
/// underlying transactions didn't carry `meta.computeUnitsConsumed` —
/// older RPC providers and non-Anchor programs often omit it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IxMetrics {
    /// Kebab-case ix name from the IDL (`deposit`, `exchange`, …).
    pub name: String,
    /// Total observed calls.
    pub count: u64,
    /// Successful calls (transaction-level success; doesn't verify
    /// inner-ix CPI outcomes).
    pub success_count: u64,
    /// Failures — calls whose transaction's top-level `err` is set.
    pub error_count: u64,
    /// `success_count / count` in [0.0, 1.0]. `None` when `count == 0`
    /// (which can't happen today but stays defensive for future watch
    /// snapshots that may include zeroed-out buckets).
    pub success_rate: Option<f64>,
    /// Median compute units consumed. `None` when no tx in the bucket
    /// carried a CU measurement.
    pub cu_p50: Option<u64>,
    /// p95 compute units consumed.
    pub cu_p95: Option<u64>,
    /// p99 compute units consumed.
    pub cu_p99: Option<u64>,
}

/// Error-code bucket, aggregated across every instruction that fired
/// it. When the IDL has a human name for the code it's populated;
/// otherwise callers see just the numeric code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBucket {
    /// Raw Anchor / program error code (custom-error number).
    pub code: u32,
    /// Human name from the IDL's `errors` array. `None` when the code
    /// doesn't resolve — most commonly for CPI'd errors from other
    /// programs.
    pub name: Option<String>,
    /// Message from the IDL (same resolution story as `name`).
    pub message: Option<String>,
    /// Total occurrences in the window.
    pub count: u64,
    /// Instruction names that produced this error, sorted by frequency
    /// descending.
    pub ix_names: Vec<String>,
}

/// A single decoded failure — enough for a dev to reproduce / debug
/// without opening the explorer. Capped at a small N per report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFailure {
    /// Base58 transaction signature.
    pub signature: String,
    /// Unix seconds.
    pub block_time: Option<i64>,
    /// Ix name that failed (first ix of the program's ixs in the tx).
    pub ix_name: Option<String>,
    /// Resolved error code + name.
    pub error_code: Option<u32>,
    pub error_name: Option<String>,
    /// Base58 fee payer — useful for linking back to the user that hit
    /// the failure.
    pub fee_payer: Option<String>,
}

/// Compute the final report from a decoded tx stream.
pub fn summarise(opts: &ObserveOpts, idl: &AnchorIdl, txs: Vec<DecodedTx>) -> ObserveReport {
    let (earliest, latest) =
        txs.iter()
            .filter_map(|t| t.block_time)
            .fold((None, None), |(lo, hi), t| {
                (
                    Some(lo.map_or(t, |v: i64| v.min(t))),
                    Some(hi.map_or(t, |v: i64| v.max(t))),
                )
            });

    let window = ObserveWindow {
        seconds: opts.window_seconds,
        tx_count: txs.len(),
        earliest_block_time: earliest,
        latest_block_time: latest,
    };

    let instructions = roll_up_instructions(&txs);
    let errors = roll_up_errors(idl, &txs);
    let recent_failures = pick_recent_failures(&txs, 10);

    ObserveReport {
        program_id: opts.program_id.clone(),
        program_name: idl.metadata.as_ref().map(|m| m.name.clone()),
        window,
        instructions,
        errors,
        recent_failures,
        upgrade_history: None,
        account_counts: Vec::new(),
    }
}

fn roll_up_instructions(txs: &[DecodedTx]) -> Vec<IxMetrics> {
    // Bucket by ix name, dropping txs we couldn't decode (no matching
    // discriminator — typically a pre-IDL-account upgrade or a cpi-only
    // call that doesn't start with our program's ix).
    let mut by_name: BTreeMap<String, Vec<&DecodedTx>> = BTreeMap::new();
    for tx in txs {
        if let Some(name) = &tx.ix_name {
            by_name.entry(name.clone()).or_default().push(tx);
        }
    }

    let mut out: Vec<IxMetrics> = by_name
        .into_iter()
        .map(|(name, group)| {
            let count = group.len() as u64;
            let error_count = group.iter().filter(|t| t.error_code.is_some()).count() as u64;
            let success_count = count - error_count;
            let cus: Vec<u64> = group.iter().filter_map(|t| t.compute_units).collect();
            let (p50, p95, p99) = percentiles(&cus);

            IxMetrics {
                name,
                count,
                success_count,
                error_count,
                success_rate: if count == 0 {
                    None
                } else {
                    Some(success_count as f64 / count as f64)
                },
                cu_p50: p50,
                cu_p95: p95,
                cu_p99: p99,
            }
        })
        .collect();
    // Stable display order: highest-volume first, tie-break by name.
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    out
}

fn roll_up_errors(idl: &AnchorIdl, txs: &[DecodedTx]) -> Vec<ErrorBucket> {
    let mut by_code: BTreeMap<u32, (u64, BTreeMap<String, u64>)> = BTreeMap::new();
    for tx in txs {
        if let Some(code) = tx.error_code {
            let (count, ix_counts) = by_code.entry(code).or_insert((0, BTreeMap::new()));
            *count += 1;
            if let Some(name) = &tx.ix_name {
                *ix_counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut out: Vec<ErrorBucket> = by_code
        .into_iter()
        .map(|(code, (count, ix_counts))| {
            let (name, message) = resolve_error(idl, code);
            let mut ix_names: Vec<(String, u64)> = ix_counts.into_iter().collect();
            ix_names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            ErrorBucket {
                code,
                name,
                message,
                count,
                ix_names: ix_names.into_iter().map(|(n, _)| n).collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.code.cmp(&b.code)));
    out
}

fn resolve_error(idl: &AnchorIdl, code: u32) -> (Option<String>, Option<String>) {
    for e in &idl.errors {
        if e.code == code {
            return (Some(e.name.clone()), e.msg.clone());
        }
    }
    (None, None)
}

fn pick_recent_failures(txs: &[DecodedTx], limit: usize) -> Vec<RecentFailure> {
    let mut failures: Vec<&DecodedTx> = txs.iter().filter(|t| t.error_code.is_some()).collect();
    // Newest first — block_time is best-effort (some legacy tx responses
    // don't carry it), so fall back to signature order.
    failures.sort_by(|a, b| b.block_time.cmp(&a.block_time));
    failures
        .into_iter()
        .take(limit)
        .map(|t| RecentFailure {
            signature: t.signature.clone(),
            block_time: t.block_time,
            ix_name: t.ix_name.clone(),
            error_code: t.error_code,
            error_name: t.error_name.clone(),
            fee_payer: t.fee_payer.clone(),
        })
        .collect()
}

/// Approximate percentile picker for a small sample. Linear in the
/// input size — we're dealing with ≤1000 transactions per observe
/// pass, so pulling in a stats crate would be overkill.
fn percentiles(values: &[u64]) -> (Option<u64>, Option<u64>, Option<u64>) {
    if values.is_empty() {
        return (None, None, None);
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let pick = |pct: f64| -> u64 {
        let idx = ((sorted.len() as f64 - 1.0) * pct).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    (Some(pick(0.50)), Some(pick(0.95)), Some(pick(0.99)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_tx(ix: Option<&str>, err: Option<u32>, cu: Option<u64>) -> DecodedTx {
        DecodedTx {
            signature: "sig".into(),
            block_time: Some(0),
            ix_name: ix.map(|s| s.to_string()),
            error_code: err,
            error_name: None,
            fee_payer: None,
            compute_units: cu,
        }
    }

    #[test]
    fn instruction_rollup_sorts_by_volume() {
        let txs = vec![
            mk_tx(Some("withdraw"), None, Some(1000)),
            mk_tx(Some("deposit"), None, Some(2000)),
            mk_tx(Some("deposit"), None, Some(2500)),
            mk_tx(Some("deposit"), Some(0x1770), Some(2200)),
        ];
        let rolled = roll_up_instructions(&txs);
        assert_eq!(rolled.len(), 2);
        assert_eq!(rolled[0].name, "deposit");
        assert_eq!(rolled[0].count, 3);
        assert_eq!(rolled[0].error_count, 1);
        assert!((rolled[0].success_rate.unwrap() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn percentiles_on_small_sample() {
        let (p50, p95, p99) = percentiles(&[10, 20, 30, 40, 50]);
        assert_eq!(p50, Some(30));
        assert_eq!(p95, Some(50));
        assert_eq!(p99, Some(50));
    }

    #[test]
    fn percentiles_on_empty_sample_are_none() {
        let (p50, p95, p99) = percentiles(&[]);
        assert!(p50.is_none() && p95.is_none() && p99.is_none());
    }
}
