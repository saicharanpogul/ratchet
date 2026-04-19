//! P005 — account-name-collision.
//!
//! Account struct named after a well-known Solana type (`State`,
//! `Config`, `Account`, `Token`, `Mint`, `System`, `Program`, `Data`).
//! Not a correctness issue, but generic names make log-grepping,
//! discriminator-reverse-lookup, and docs confusing — and sometimes
//! clash with user expectations about what the type represents.

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::ProgramSurface;

pub const ID: &str = "P005";
pub const NAME: &str = "account-name-collision";
pub const DESCRIPTION: &str =
    "Account struct is named after a well-known Solana type, making tooling and docs ambiguous.";

pub struct AccountNameCollision;

const RESERVED: &[&str] = &[
    "State",
    "Config",
    "Account",
    "Token",
    "Mint",
    "System",
    "Program",
    "Data",
    "Instruction",
];

impl PreflightRule for AccountNameCollision {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(&self, surface: &ProgramSurface, _ctx: &CheckContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        for name in surface.accounts.keys() {
            if !RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name)) {
                continue;
            }
            findings.push(
                self.finding(Severity::Additive)
                    .at([format!("account:{name}")])
                    .message(format!(
                        "account struct `{name}` collides with a generic Solana type name; consider namespacing (e.g. `{}State`)",
                        surface.name.split('_').next().unwrap_or("Program")
                    ))
                    .suggestion(
                        "Prefix the name with the program's domain (e.g. `VaultState`, `SwapConfig`) so discriminator search and logs stay unambiguous.",
                    ),
            );
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, ProgramSurface};

    fn surface_with(name: &str) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "vault".into(),
            ..Default::default()
        };
        s.accounts.insert(
            name.into(),
            AccountDef {
                name: name.into(),
                discriminator: [0; 8],
                fields: vec![],
                size: None,
            },
        );
        s
    }

    #[test]
    fn reserved_name_is_flagged() {
        let s = surface_with("State");
        let findings = AccountNameCollision.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
    }

    #[test]
    fn reserved_name_case_insensitive() {
        let s = surface_with("config");
        assert_eq!(
            AccountNameCollision.check(&s, &CheckContext::new()).len(),
            1
        );
    }

    #[test]
    fn domain_prefixed_name_is_not_flagged() {
        let s = surface_with("VaultState");
        assert!(AccountNameCollision
            .check(&s, &CheckContext::new())
            .is_empty());
    }
}
