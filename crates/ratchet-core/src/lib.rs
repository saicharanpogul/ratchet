//! Core IR and rule engine for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! This crate holds the framework-agnostic representation of a Solana program's
//! public surface — accounts, instructions, enums, PDAs — and the rule engine
//! that diffs two versions of that surface and classifies each change as
//! `Additive`, `Breaking`, or `Unsafe`.
