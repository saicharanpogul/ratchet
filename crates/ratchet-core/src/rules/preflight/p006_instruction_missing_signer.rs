//! P006 — instruction-missing-signer.
//!
//! An instruction with at least one writable account but no signer
//! accounts is suspicious — anyone can call it and mutate state.
//! Read-only instructions legitimately have no signer (a lot of view
//! functions fit this), so the rule only fires on instructions that
//! write.
//!
//! Emitted as `Unsafe` with `allow-no-signer` because there are
//! legitimate write-without-signer cases (e.g. publicly-callable
//! settlement crankers, accounts that rely on CPI authority checks
//! elsewhere). The default flags them so developers at least
//! consider whether the authorization model is what they intended.

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::ProgramSurface;

pub const ID: &str = "P006";
pub const NAME: &str = "instruction-missing-signer";
pub const DESCRIPTION: &str =
    "Instruction writes to accounts with no signer slot — anyone can mutate state unless CPI authority is checked elsewhere.";

pub struct InstructionMissingSigner;

impl PreflightRule for InstructionMissingSigner {
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
        for (ix_name, ix) in &surface.instructions {
            let has_writable = ix.accounts.iter().any(|a| a.is_writable);
            let has_signer = ix.accounts.iter().any(|a| a.is_signer);
            if !has_writable || has_signer {
                continue;
            }
            findings.push(
                self.finding(Severity::Unsafe)
                    .at([format!("ix:{ix_name}")])
                    .message(format!(
                        "instruction `{ix_name}` writes to accounts but has no signer slot",
                    ))
                    .suggestion(
                        "Add a signer account to the Accounts struct, or verify the authorization story (CPI authority check, permissionless crank, etc.) and acknowledge with --unsafe allow-no-signer.",
                    )
                    .allow_flag("allow-no-signer"),
            );
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountInput, InstructionDef, ProgramSurface};

    fn acc(name: &str, signer: bool, writable: bool) -> AccountInput {
        AccountInput {
            name: name.into(),
            is_signer: signer,
            is_writable: writable,
            is_optional: false,
            pda: None,
        }
    }

    fn surface_with(ix: InstructionDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(ix.name.clone(), ix);
        s
    }

    fn ix(name: &str, accounts: Vec<AccountInput>) -> InstructionDef {
        InstructionDef {
            name: name.into(),
            discriminator: [0; 8],
            args: vec![],
            accounts,
        }
    }

    #[test]
    fn write_with_signer_is_fine() {
        let s = surface_with(ix(
            "deposit",
            vec![acc("user", true, true), acc("vault", false, true)],
        ));
        assert!(InstructionMissingSigner
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn write_without_signer_is_flagged() {
        let s = surface_with(ix(
            "crank",
            vec![acc("vault", false, true), acc("payer", false, false)],
        ));
        let findings = InstructionMissingSigner.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Unsafe);
        assert_eq!(findings[0].allow_flag.as_deref(), Some("allow-no-signer"));
    }

    #[test]
    fn readonly_without_signer_is_fine() {
        let s = surface_with(ix("get_balance", vec![acc("vault", false, false)]));
        assert!(InstructionMissingSigner
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn no_accounts_is_fine() {
        let s = surface_with(ix("noop", vec![]));
        assert!(InstructionMissingSigner
            .check(&s, &CheckContext::new())
            .is_empty());
    }
}
