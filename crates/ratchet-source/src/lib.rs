//! Rust source parser for ratchet.
//!
//! Anchor IDLs from 0.30+ carry PDA seed information, but they don't
//! capture the full expression — things like
//! `&vault.config.admin.to_bytes()` land in the IDL with only the account
//! path, losing the field reference. When the user runs `ratchet` against
//! their own repo we can do better by parsing the source directly.
//!
//! This crate walks a directory of Rust files, finds `#[derive(Accounts)]`
//! structs, extracts `#[account(seeds = [...])]` expressions, and emits a
//! [`SourcePatch`] that the CLI merges into a [`ProgramSurface`] before
//! running rules.

pub mod parse;
pub mod patch;
pub mod seeds;

pub use parse::{parse_dir, parse_file, SourceScan};
pub use patch::SourcePatch;
pub use seeds::{parse_seed_expr, SeedExpr};
