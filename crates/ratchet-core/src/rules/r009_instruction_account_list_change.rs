//! R009 — instruction-account-list-change.
//!
//! An instruction's account list changed: accounts added, removed, or
//! reordered. The Solana runtime dispatches by index, so any shift in the
//! account list means callers pass the wrong `AccountInfo` at a given
//! position — PDA derivations, signer checks, and mut-ability checks all
//! silently target the wrong account.
//!
//! This rule only considers the sequence of account *names*. Signer and
//! writable flips on an identical-name slot are covered by R010.
//!
//! Breaking with `allow-ix-account-change` escape hatch.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{AccountInput, ProgramSurface};

pub const ID: &str = "R009";
pub const NAME: &str = "instruction-account-list-change";
pub const DESCRIPTION: &str =
    "An instruction's account list added, removed, or reordered slots; callers pass accounts at the wrong index.";

pub struct InstructionAccountListChange;

impl Rule for InstructionAccountListChange {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        NAME
    }
    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn check(
        &self,
        old: &ProgramSurface,
        new: &ProgramSurface,
        _ctx: &CheckContext,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (name, old_ix) in &old.instructions {
            let Some(new_ix) = new.instructions.get(name) else {
                continue;
            };
            let old_names: Vec<&str> = old_ix.accounts.iter().map(|a| a.name.as_str()).collect();
            let new_names: Vec<&str> = new_ix.accounts.iter().map(|a| a.name.as_str()).collect();
            if old_names == new_names {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("ix:{name}"), "accounts".into()])
                    .message(format!(
                        "account list of `{name}` changed: [{}] → [{}]",
                        old_names.join(", "),
                        new_names.join(", ")
                    ))
                    .old(render(&old_ix.accounts))
                    .new_value(render(&new_ix.accounts))
                    .allow_flag("allow-ix-account-change")
                    .suggestion(
                        "The Solana runtime dispatches accounts by index. Append new accounts \
                         at the end if possible; if the order must change, publish a new \
                         instruction with a distinct name.",
                    ),
            );
        }
        findings
    }
}

fn render(accounts: &[AccountInput]) -> String {
    accounts
        .iter()
        .map(|a| {
            let mut flags = Vec::new();
            if a.is_signer {
                flags.push("signer");
            }
            if a.is_writable {
                flags.push("mut");
            }
            if flags.is_empty() {
                a.name.clone()
            } else {
                format!("{} [{}]", a.name, flags.join(","))
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::InstructionDef;

    fn acc(name: &str) -> AccountInput {
        AccountInput {
            name: name.into(),
            is_signer: false,
            is_writable: false,
            is_optional: false,
            pda: None,
        }
    }

    fn ix(name: &str, accounts: Vec<AccountInput>) -> InstructionDef {
        InstructionDef {
            name: name.into(),
            discriminator: [0; 8],
            args: vec![],
            accounts,
        }
    }

    fn surface(i: InstructionDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(i.name.clone(), i);
        s
    }

    #[test]
    fn identical_accounts_no_finding() {
        let s = surface(ix("deposit", vec![acc("user"), acc("vault")]));
        assert!(InstructionAccountListChange
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn account_reorder_is_breaking() {
        let old = surface(ix("deposit", vec![acc("user"), acc("vault")]));
        let new = surface(ix("deposit", vec![acc("vault"), acc("user")]));
        let findings = InstructionAccountListChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Breaking);
        assert_eq!(
            findings[0].allow_flag.as_deref(),
            Some("allow-ix-account-change")
        );
    }

    #[test]
    fn account_removal_is_breaking() {
        let old = surface(ix(
            "deposit",
            vec![acc("user"), acc("vault"), acc("system_program")],
        ));
        let new = surface(ix("deposit", vec![acc("user"), acc("vault")]));
        let findings = InstructionAccountListChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn account_rename_is_caught_as_reorder() {
        // Renaming an account changes the index-to-name mapping, which we
        // treat the same as a reorder. The message shows both lists.
        let old = surface(ix("deposit", vec![acc("user"), acc("vault")]));
        let new = surface(ix("deposit", vec![acc("owner"), acc("vault")]));
        let findings = InstructionAccountListChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn signer_writable_flips_are_not_this_rules_scope() {
        let old = surface(ix("deposit", vec![acc("user")]));
        let mut mutated = acc("user");
        mutated.is_writable = true;
        let new = surface(ix("deposit", vec![mutated]));
        // Same name sequence [user] == [user], so R009 does not fire.
        assert!(InstructionAccountListChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
