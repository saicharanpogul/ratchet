//! Core IR and rule engine for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! This crate holds the framework-agnostic representation of a Solana program's
//! public surface — accounts, instructions, enums, PDAs — and the rule engine
//! that diffs two versions of that surface and classifies each change as
//! [`Severity::Additive`], [`Severity::Unsafe`], or [`Severity::Breaking`].

pub mod diagnostics;
pub mod engine;
pub mod preflight;
pub mod rule;
pub mod rules;
pub mod surface;

pub use diagnostics::{Finding, Path, Report, Severity};
pub use engine::{check, default_rules};
pub use preflight::{default_preflight_rules, preflight, PreflightRule};
pub use rule::{CheckContext, Rule};
pub use surface::{
    AccountDef, AccountInput, ArgDef, Discriminator, EnumVariant, EnumVariantFields, ErrorDef,
    EventDef, FieldDef, InstructionDef, PdaSpec, PrimitiveType, ProgramSurface, Seed, TypeDef,
    TypeRef,
};
