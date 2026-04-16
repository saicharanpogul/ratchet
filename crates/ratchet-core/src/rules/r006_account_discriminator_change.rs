//! R006 — account-discriminator-change.
//!
//! Every Anchor account is routed at the start of `try_deserialize` by
//! comparing its first 8 bytes against an expected discriminator. If those
//! bytes change (whether the struct was renamed, a custom discriminator
//! was added, or the default convention changed), every existing on-chain
//! account becomes unreadable — the new binary refuses to deserialize it
//! with `AccountDiscriminatorMismatch`.
//!
//! Emitted as `Breaking` with an `allow-rename` escape hatch for the
//! (rare) deliberate rename during a coordinated migration.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R006";
pub const NAME: &str = "account-discriminator-change";
pub const DESCRIPTION: &str =
    "An account's discriminator changed; every existing on-chain account now fails to deserialize.";

pub struct AccountDiscriminatorChange;

impl Rule for AccountDiscriminatorChange {
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
        for (name, old_acc) in &old.accounts {
            let Some(new_acc) = new.accounts.get(name) else {
                continue;
            };
            if old_acc.discriminator == new_acc.discriminator {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("account:{name}"), "discriminator".into()])
                    .message(format!(
                        "discriminator of account `{name}` changed: {} → {}",
                        hex(&old_acc.discriminator),
                        hex(&new_acc.discriminator)
                    ))
                    .old(hex(&old_acc.discriminator))
                    .new_value(hex(&new_acc.discriminator))
                    .allow_flag("allow-rename")
                    .suggestion(
                        "If this is an accidental struct rename, restore the original name. \
                         If the rename is intentional, pin the original discriminator with \
                         `#[account(discriminator = <bytes>)]` (Anchor 0.31+).",
                    ),
            );
        }
        findings
    }
}

fn hex(disc: &[u8; 8]) -> String {
    let mut out = String::with_capacity(18);
    out.push_str("0x");
    for b in disc {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, Discriminator};

    fn acc(disc: Discriminator) -> AccountDef {
        AccountDef {
            name: "Vault".into(),
            discriminator: disc,
            fields: vec![],
            size: None,
        }
    }

    fn surface(a: AccountDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(a.name.clone(), a);
        s
    }

    #[test]
    fn identical_discriminators_no_finding() {
        let s = surface(acc([1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(AccountDiscriminatorChange
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn discriminator_change_is_breaking_with_allow() {
        let old = surface(acc([1, 2, 3, 4, 5, 6, 7, 8]));
        let new = surface(acc([9, 10, 11, 12, 13, 14, 15, 16]));
        let findings = AccountDiscriminatorChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-rename"));
        assert_eq!(f.old.as_deref(), Some("0x0102030405060708"));
        assert_eq!(f.new.as_deref(), Some("0x090a0b0c0d0e0f10"));
    }

    #[test]
    fn renamed_account_not_in_scope_of_this_rule() {
        // Rename produces: old has `Vault`, new has `VaultV2`. No shared
        // name to compare — handled by account-removed / account-added
        // rules instead.
        let old = surface(acc([1; 8]));
        let mut new = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        new.accounts.insert(
            "VaultV2".into(),
            AccountDef {
                name: "VaultV2".into(),
                discriminator: [2; 8],
                fields: vec![],
                size: None,
            },
        );
        assert!(AccountDiscriminatorChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
