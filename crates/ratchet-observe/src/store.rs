//! Persistent storage for watch mode + historical comparison.
//!
//! Backed by SQLite via `rusqlite` when the `store` feature is on.
//! Keeps the schema tight — one table per logical roll-up
//! (`observation`, `ix_metric`, `error_bucket`) — so queries stay
//! readable on the hosted-dashboard variant.
//!
//! Deliberately empty in this commit; the fetch → decode → aggregate
//! pipeline works end-to-end without it. Watch mode lands in a
//! follow-up milestone.
