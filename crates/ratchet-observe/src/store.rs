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
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshot (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                program_id TEXT NOT NULL,
                taken_at   INTEGER NOT NULL,
                payload    TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS snapshot_program_taken
                ON snapshot(program_id, taken_at);",
        )
        .context("creating snapshot schema")
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
}
