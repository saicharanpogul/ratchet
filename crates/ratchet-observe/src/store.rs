//! SQLite-backed persistence for observe snapshots.
//!
//! Feature-gated (`store`) so a one-shot `ratchet observe` run still
//! has zero filesystem footprint. Watch mode + historical comparison
//! light this up.
//!
//! Schema intentionally minimal:
//!
//! ```sql
//! CREATE TABLE snapshot (
//!   id          INTEGER PRIMARY KEY AUTOINCREMENT,
//!   program_id  TEXT NOT NULL,
//!   taken_at    INTEGER NOT NULL,   -- unix seconds, when the run completed
//!   payload     TEXT NOT NULL       -- serde_json::to_string(&ObserveReport)
//! );
//! CREATE INDEX snapshot_program_taken ON snapshot(program_id, taken_at);
//! ```
//!
//! Storing the whole `ObserveReport` as JSON is deliberate — the shape
//! evolves faster than a relational schema can keep up, and individual
//! snapshots are small (< 100 KB on typical programs). A future
//! revision can normalise hot query paths into columns, but "is my
//! error rate trending up this week?" just reads the last N rows.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::report::ObserveReport;

/// Open (and initialise if needed) the SQLite store at `path`.
///
/// Callers typically pass `~/.ratchet/observe/<program_id>.db`, but
/// anything resolvable works — tests use `:memory:` for isolation.
pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path.as_ref())
            .with_context(|| format!("opening sqlite at {}", path.as_ref().display()))?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    /// In-memory store — only useful for tests and ephemeral MCP
    /// sessions that don't want a file on disk.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("opening in-memory sqlite")?;
        Self::ensure_schema(&conn)?;
        Ok(Self { conn })
    }

    fn ensure_schema(conn: &Connection) -> Result<()> {
        // Two tables:
        // - `snapshot`   — one row per `observe` cycle, the aggregated
        //                  ObserveReport. Used by the Δ-since-last
        //                  summary in watch mode.
        // - `transaction_cache` — one row per decoded tx. Supports
        //                  incremental fetches: the next cycle only
        //                  pulls sigs newer than the watermark
        //                  (`MAX(slot)`), reuses the rest.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshot (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                program_id TEXT NOT NULL,
                taken_at   INTEGER NOT NULL,
                payload    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS snapshot_program_taken
                ON snapshot(program_id, taken_at);

            CREATE TABLE IF NOT EXISTS transaction_cache (
                program_id TEXT NOT NULL,
                signature  TEXT NOT NULL,
                slot       INTEGER NOT NULL,
                block_time INTEGER,
                fetched_at INTEGER NOT NULL,
                payload    TEXT NOT NULL,
                PRIMARY KEY (program_id, signature)
            );
            CREATE INDEX IF NOT EXISTS tx_cache_program_slot
                ON transaction_cache(program_id, slot DESC);
            CREATE INDEX IF NOT EXISTS tx_cache_program_block_time
                ON transaction_cache(program_id, block_time DESC);",
        )
        .context("creating store schema")
    }

    /// Persist a completed report. Returns the newly-inserted row id
    /// so callers that want to link back to the snapshot (e.g. a
    /// hosted dashboard URL) can reference it.
    pub fn insert(&self, report: &ObserveReport, taken_at: i64) -> Result<i64> {
        let payload = serde_json::to_string(report).context("serialising report")?;
        let mut stmt = self
            .conn
            .prepare_cached(
                "INSERT INTO snapshot(program_id, taken_at, payload) VALUES (?1, ?2, ?3)",
            )
            .context("preparing insert")?;
        stmt.execute(params![report.program_id, taken_at, payload])
            .context("inserting snapshot")?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Return the most recent snapshot for `program_id` strictly
    /// older than `before_ts` (unix seconds). Used to find a baseline
    /// when diffing "then vs now" in watch mode. `None` when the
    /// program has no earlier snapshots.
    pub fn latest_before(
        &self,
        program_id: &str,
        before_ts: i64,
    ) -> Result<Option<(i64, ObserveReport)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT taken_at, payload FROM snapshot
                 WHERE program_id = ?1 AND taken_at < ?2
                 ORDER BY taken_at DESC LIMIT 1",
            )
            .context("preparing latest_before")?;
        let row = stmt
            .query_row(params![program_id, before_ts], |row| {
                let taken_at: i64 = row.get(0)?;
                let payload: String = row.get(1)?;
                Ok((taken_at, payload))
            })
            .optional()
            .context("querying latest_before")?;
        match row {
            Some((taken_at, payload)) => {
                let report: ObserveReport =
                    serde_json::from_str(&payload).context("deserialising snapshot payload")?;
                Ok(Some((taken_at, report)))
            }
            None => Ok(None),
        }
    }

    /// Return the N most recent snapshots for `program_id`, newest
    /// first. `limit` is caller-chosen so scripts can page without
    /// pulling unbounded history.
    pub fn recent(&self, program_id: &str, limit: usize) -> Result<Vec<(i64, ObserveReport)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT taken_at, payload FROM snapshot
                 WHERE program_id = ?1
                 ORDER BY taken_at DESC LIMIT ?2",
            )
            .context("preparing recent")?;
        let mut rows = stmt
            .query(params![program_id, limit as i64])
            .context("executing recent")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().context("fetching row")? {
            let taken_at: i64 = row.get(0)?;
            let payload: String = row.get(1)?;
            let report: ObserveReport = serde_json::from_str(&payload)?;
            out.push((taken_at, report));
        }
        Ok(out)
    }

    // -- Transaction cache ----------------------------------------------
    //
    // The cache powers incremental fetch: on every cycle after the
    // first, we only pull signatures newer than the cache's watermark,
    // decode them, and merge with the already-cached txs.
    //
    // The cache persists `DecodedTx` as JSON. Keeping the raw decoded
    // shape (not the report) means a future window-size change on the
    // same program reuses the history without re-fetching.

    /// Most recent cached signature for `program_id`, determined by
    /// highest slot. `None` when the cache is empty — callers treat
    /// that as "first run, do a full window pull."
    pub fn latest_cached_signature(&self, program_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT signature FROM transaction_cache
                 WHERE program_id = ?1
                 ORDER BY slot DESC LIMIT 1",
            )
            .context("preparing latest_cached_signature")?;
        stmt.query_row(params![program_id], |row| row.get::<_, String>(0))
            .optional()
    }

    /// Insert (or replace on conflict) a decoded tx. Returns the number
    /// of rows actually affected — callers can use this to report how
    /// many sigs were truly new vs already cached.
    pub fn insert_tx(
        &self,
        program_id: &str,
        signature: &str,
        slot: u64,
        block_time: Option<i64>,
        payload: &str,
        fetched_at: i64,
    ) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "INSERT OR REPLACE INTO transaction_cache
                    (program_id, signature, slot, block_time, fetched_at, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .context("preparing insert_tx")?;
        stmt.execute(params![
            program_id,
            signature,
            slot as i64,
            block_time,
            fetched_at,
            payload,
        ])
        .context("inserting tx row")
    }

    /// Pull every cached tx for `program_id` whose `block_time` lies
    /// within [`window_start_ts`, `window_end_ts`], newest first.
    /// Returns the raw JSON payloads; the caller deserialises into
    /// whatever shape it needs (we keep the cache decoupled from the
    /// `DecodedTx` type so schema evolution stays in one layer).
    pub fn txs_in_window(
        &self,
        program_id: &str,
        window_start_ts: i64,
        window_end_ts: i64,
        limit: usize,
    ) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT payload FROM transaction_cache
                 WHERE program_id = ?1
                   AND (block_time IS NULL OR block_time BETWEEN ?2 AND ?3)
                 ORDER BY slot DESC
                 LIMIT ?4",
            )
            .context("preparing txs_in_window")?;
        let mut rows = stmt.query(params![
            program_id,
            window_start_ts,
            window_end_ts,
            limit as i64
        ])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row.get::<_, String>(0)?);
        }
        Ok(out)
    }

    /// Drop cached txs older than `cutoff_block_time`. Keeps disk
    /// growth bounded for long-running watch loops; callers typically
    /// pass `now - 2 * window_seconds` so recent history survives for
    /// window-size changes while ancient history falls off.
    pub fn prune_txs_older_than(&self, program_id: &str, cutoff_block_time: i64) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "DELETE FROM transaction_cache
                 WHERE program_id = ?1
                   AND block_time IS NOT NULL
                   AND block_time < ?2",
            )
            .context("preparing prune_txs_older_than")?;
        stmt.execute(params![program_id, cutoff_block_time])
            .context("pruning old tx rows")
    }
}

/// Small helper for the rusqlite `.optional()` extension without
/// pulling the full `OptionalExtension` import at every call site.
trait OptionalQuery<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalQuery<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::IxMetrics;
    use crate::report::{ObserveReport, ObserveWindow};

    fn sample(program_id: &str, tx_count: usize) -> ObserveReport {
        ObserveReport {
            program_id: program_id.into(),
            program_name: Some("demo".into()),
            window: ObserveWindow {
                seconds: 3600,
                tx_count,
                earliest_block_time: None,
                latest_block_time: None,
            },
            instructions: vec![IxMetrics {
                name: "deposit".into(),
                count: tx_count as u64,
                success_count: tx_count as u64,
                error_count: 0,
                success_rate: Some(1.0),
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
    fn schema_bootstraps_on_open_and_is_idempotent() {
        let s = Store::in_memory().unwrap();
        // Re-running ensure_schema twice must be a no-op.
        Store::ensure_schema(&s.conn).unwrap();
        assert!(s.recent("noone", 10).unwrap().is_empty());
    }

    #[test]
    fn insert_and_recent_roundtrip() {
        let s = Store::in_memory().unwrap();
        s.insert(&sample("p1", 10), 100).unwrap();
        s.insert(&sample("p1", 20), 200).unwrap();
        s.insert(&sample("p2", 30), 150).unwrap(); // unrelated program

        let recent = s.recent("p1", 10).unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first.
        assert_eq!(recent[0].0, 200);
        assert_eq!(recent[0].1.window.tx_count, 20);
        assert_eq!(recent[1].0, 100);
    }

    #[test]
    fn latest_before_skips_future_snapshots_and_other_programs() {
        let s = Store::in_memory().unwrap();
        s.insert(&sample("p1", 10), 100).unwrap();
        s.insert(&sample("p1", 20), 300).unwrap();
        s.insert(&sample("p2", 999), 200).unwrap();

        let before = s.latest_before("p1", 250).unwrap();
        assert!(before.is_some());
        let (taken_at, report) = before.unwrap();
        assert_eq!(taken_at, 100);
        assert_eq!(report.window.tx_count, 10);
    }

    #[test]
    fn latest_before_returns_none_when_no_earlier_snapshot() {
        let s = Store::in_memory().unwrap();
        s.insert(&sample("p1", 10), 100).unwrap();
        assert!(s.latest_before("p1", 50).unwrap().is_none());
    }

    #[test]
    fn tx_cache_watermark_tracks_highest_slot() {
        let s = Store::in_memory().unwrap();
        s.insert_tx("p1", "sig_old", 100, Some(1000), "{}", 0)
            .unwrap();
        s.insert_tx("p1", "sig_new", 200, Some(2000), "{}", 0)
            .unwrap();
        s.insert_tx("p1", "sig_mid", 150, Some(1500), "{}", 0)
            .unwrap();
        // Other program shouldn't interfere.
        s.insert_tx("p2", "sig_p2", 9999, Some(9999), "{}", 0)
            .unwrap();

        assert_eq!(
            s.latest_cached_signature("p1").unwrap().as_deref(),
            Some("sig_new")
        );
        assert_eq!(
            s.latest_cached_signature("p2").unwrap().as_deref(),
            Some("sig_p2")
        );
    }

    #[test]
    fn tx_cache_returns_empty_when_program_never_seen() {
        let s = Store::in_memory().unwrap();
        assert!(s.latest_cached_signature("nobody").unwrap().is_none());
    }

    #[test]
    fn tx_cache_window_filters_by_block_time() {
        let s = Store::in_memory().unwrap();
        s.insert_tx("p1", "a", 100, Some(1_000), "{\"i\":1}", 0)
            .unwrap();
        s.insert_tx("p1", "b", 200, Some(2_000), "{\"i\":2}", 0)
            .unwrap();
        s.insert_tx("p1", "c", 300, Some(3_000), "{\"i\":3}", 0)
            .unwrap();

        // Window covers only `b`.
        let window = s.txs_in_window("p1", 1_500, 2_500, 100).unwrap();
        assert_eq!(window.len(), 1);
        assert_eq!(window[0], "{\"i\":2}");

        // Wider window, respects limit.
        let window = s.txs_in_window("p1", 0, 5_000, 2).unwrap();
        assert_eq!(window.len(), 2);
        // Newest first — slot 300 = payload {"i":3}.
        assert_eq!(window[0], "{\"i\":3}");
        assert_eq!(window[1], "{\"i\":2}");
    }

    #[test]
    fn tx_cache_insert_replace_allows_backfill_corrections() {
        let s = Store::in_memory().unwrap();
        s.insert_tx("p1", "a", 100, Some(1_000), "{\"v\":1}", 0)
            .unwrap();
        // Re-insert with updated payload — simulates a decoder fix.
        s.insert_tx("p1", "a", 100, Some(1_000), "{\"v\":2}", 1)
            .unwrap();
        let window = s.txs_in_window("p1", 0, 5_000, 10).unwrap();
        assert_eq!(window.len(), 1);
        assert_eq!(window[0], "{\"v\":2}");
    }

    #[test]
    fn tx_cache_prune_drops_older_rows_only() {
        let s = Store::in_memory().unwrap();
        s.insert_tx("p1", "a", 100, Some(1_000), "{}", 0).unwrap();
        s.insert_tx("p1", "b", 200, Some(2_000), "{}", 0).unwrap();
        s.insert_tx("p1", "c", 300, Some(3_000), "{}", 0).unwrap();

        let dropped = s.prune_txs_older_than("p1", 2_500).unwrap();
        assert_eq!(dropped, 2);
        let window = s.txs_in_window("p1", 0, 10_000, 10).unwrap();
        assert_eq!(window.len(), 1);
    }
}
