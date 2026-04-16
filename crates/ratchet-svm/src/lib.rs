//! Runtime-replay verification for ratchet.
//!
//! Static rules catch most schema-level breaks, but a few classes of
//! corruption only show up when the new binary actually looks at real
//! on-chain bytes — e.g. an account that was written with an older layout
//! and never re-serialized, or a data size that would fit the old struct
//! but overflow the new one's minimum.
//!
//! This crate samples live program-owned accounts via
//! `getProgramAccounts`, groups them by discriminator, and validates each
//! sample against the corresponding account definition in the new
//! [`ProgramSurface`]. When it finds mismatches it reports them as
//! concrete failure categories so a developer can chase down the
//! offending pubkeys.
//!
//! Real in-process SVM execution (litesvm / Surfpool) is a future
//! enhancement; checking discriminator + byte length is enough to catch
//! the common failure modes without pulling in a full runtime.

pub mod fetch;
pub mod report;
pub mod validate;

pub use fetch::{fetch_program_accounts, ProgramAccount};
pub use report::{AccountVerdict, ReplayReport, TypeTally};
pub use validate::{min_account_size, validate_surface};
