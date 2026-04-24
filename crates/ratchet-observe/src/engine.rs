//! Incremental observe engine — stateful companion to the stateless
//! [`crate::observe`] entry point.
//!
//! The engine wraps a persistent SQLite [`crate::store::Store`] and a
//! fixed `program_id`. Each call to [`ObserveEngine::observe`]:
//!
//! 1. Looks up the cache watermark (newest cached signature).
//! 2. Fetches only signatures newer than the watermark, via
//!    `getSignaturesForAddress(until = watermark)`. First run (no
//!    watermark) falls back to the full-window pull the stateless
//!    entry point uses.
//! 3. Batch-fetches the new transactions and decodes them.
//! 4. Inserts the decoded rows into the cache.
//! 5. Prunes anything older than `2 × window_seconds` to keep disk
//!    growth bounded.
//! 6. Queries the cache for every row inside the current window and
//!    aggregates into an [`ObserveReport`].
//!
//! The net effect: first run takes the usual full-window pull, every
//! subsequent run pays only for traffic that arrived since the last
//! cycle. On a program doing 300 tx/hour with a 5m cycle that's
//! roughly 25 new txs per refresh — sub-second against a paid RPC
//! tier.
//!
//! Feature-gated on `store` + `rpc`: without persistence there's no
//! cache to hit, without RPC there's no way to fill it.

use anyhow::{Context, Result};
use ratchet_anchor::AnchorIdl;

use crate::aggregate::summarise;
use crate::decode::{decode_all, DecodedTx};
use crate::fetch::{self, Cluster};
use crate::report::ObserveReport;
use crate::store::Store;
use crate::ObserveOpts;

/// Unix seconds the engine uses to tag cache rows and drive window
/// filtering. Pulled out as a function so tests can inject a clock.
fn now_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Stateful observer tied to a single program's SQLite cache.
pub struct ObserveEngine {
    store: Store,
    program_id: String,
}

impl ObserveEngine {
    /// Create an engine against an existing store. Callers typically
    /// open the store once (at `~/.ratchet/observe/<pid>.db`) and
    /// hand it to the engine for the session.
    pub fn new(store: Store, program_id: impl Into<String>) -> Self {
        Self {
            store,
            program_id: program_id.into(),
        }
    }

    /// Run one incremental cycle and return the aggregated report.
    ///
    /// `idl` is supplied by the caller so the engine doesn't re-fetch
    /// the IDL on every cycle (it only ever changes on a program
    /// upgrade, which the surrounding watch / UI loop already tracks
    /// via the upgrade-history panel).
    pub fn observe(
        &self,
        cluster: &Cluster,
        idl: &AnchorIdl,
        opts: &ObserveOpts,
    ) -> Result<ObserveReport> {
        let watermark = self
            .store
            .latest_cached_signature(&self.program_id)
            .context("reading tx-cache watermark")?;
        let new_sigs = fetch::signatures_within_window_until(cluster, opts, watermark.as_deref())
            .context("fetching new signatures")?;

        if !new_sigs.is_empty() {
            let raw_txs =
                fetch::fetch_transactions(cluster, &new_sigs, opts.pace_ms, opts.show_progress)
                    .context("fetching new transactions")?;
            let decoded = decode_all(idl, &raw_txs);
            let fetched_at = now_seconds();
            for (sig_info, tx) in new_sigs.iter().zip(decoded.iter()) {
                let payload =
                    serde_json::to_string(tx).context("serialising decoded tx for cache")?;
                self.store
                    .insert_tx(
                        &self.program_id,
                        &sig_info.signature,
                        sig_info.slot,
                        sig_info.block_time,
                        &payload,
                        fetched_at,
                    )
                    .context("inserting tx into cache")?;
            }
        }

        // Prune anything twice-the-window old. Keeps disk growth
        // bounded without throwing away history a window-size bump
        // might want to reuse.
        let now = now_seconds();
        let prune_cutoff = now.saturating_sub(opts.window_seconds.saturating_mul(2) as i64);
        let _ = self
            .store
            .prune_txs_older_than(&self.program_id, prune_cutoff);

        // Query the cache for the current window.
        let window_start = now.saturating_sub(opts.window_seconds as i64);
        let payloads = self
            .store
            .txs_in_window(&self.program_id, window_start, now, opts.limit)
            .context("reading window from cache")?;
        let mut cached_txs: Vec<DecodedTx> = Vec::with_capacity(payloads.len());
        for p in payloads {
            cached_txs.push(serde_json::from_str(&p).context("deserialising cached tx payload")?);
        }

        Ok(summarise(opts, idl, cached_txs))
    }

    /// Expose the underlying store for callers that also want to do
    /// the Δ-since-last snapshot / historical comparison reads the
    /// stateless entry point already covers.
    pub fn store(&self) -> &Store {
        &self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::IxMetrics;
    use crate::report::{ObserveReport, ObserveWindow};

    fn empty_report(pid: &str) -> ObserveReport {
        ObserveReport {
            program_id: pid.into(),
            program_name: None,
            window: ObserveWindow {
                seconds: 3600,
                tx_count: 0,
                earliest_block_time: None,
                latest_block_time: None,
            },
            instructions: vec![IxMetrics {
                name: "noop".into(),
                count: 0,
                success_count: 0,
                error_count: 0,
                success_rate: None,
                cu_p50: None,
                cu_p95: None,
                cu_p99: None,
            }],
            errors: vec![],
            recent_failures: vec![],
            upgrade_history: None,
            account_counts: vec![],
        }
    }

    #[test]
    fn engine_can_wrap_store_and_read_back() {
        // This is a shape test — the real RPC path is exercised by
        // integration runs. Here we just lock the library surface:
        // ObserveEngine wraps Store, exposes it, and accepts a report
        // from the snapshot side.
        let store = Store::in_memory().unwrap();
        let engine = ObserveEngine::new(store, "pid");
        engine.store().insert(&empty_report("pid"), 100).unwrap();
        let recent = engine.store().recent("pid", 10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].0, 100);
    }
}
