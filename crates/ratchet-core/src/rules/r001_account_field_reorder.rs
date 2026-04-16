//! R001 — account-field-reorder.
//!
//! Accounts are serialized with Borsh, which lays fields out in declaration
//! order. Reordering the fields of an existing account changes the byte
//! offset of every field after the swap, which means the program reads
//! garbage off of every existing account. There is no safe acknowledgement
//! flag: the only way to recover is a migration instruction that rewrites
//! the accounts, and even then it's usually cheaper to go back to the
//! original order.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R001";
pub const NAME: &str = "account-field-reorder";
pub const DESCRIPTION: &str =
    "Fields of an existing account were reordered; every on-chain account now deserializes to garbage.";

pub struct AccountFieldReorder;

impl Rule for AccountFieldReorder {
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

            let old_names: HashSet<&str> =
                old_acc.fields.iter().map(|f| f.name.as_str()).collect();
            let new_names: HashSet<&str> =
                new_acc.fields.iter().map(|f| f.name.as_str()).collect();
            let common: HashSet<&str> = old_names.intersection(&new_names).copied().collect();

            if common.len() < 2 {
                // Reorder requires at least two shared fields to be meaningful.
                continue;
            }

            let old_order: Vec<&str> = old_acc
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .filter(|n| common.contains(n))
                .collect();
            let new_order: Vec<&str> = new_acc
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .filter(|n| common.contains(n))
                .collect();

            if old_order != new_order {
                findings.push(
                    self.finding(Severity::Breaking)
                        .at([format!("account:{name}")])
                        .message(format!(
                            "fields reordered in account `{name}`: [{}] → [{}]",
                            old_order.join(", "),
                            new_order.join(", ")
                        ))
                        .old(old_order.join(", "))
                        .new_value(new_order.join(", "))
                        .suggestion(
                            "Borsh lays fields out in declaration order. Put the fields back \
                             in the original order; if the new order is semantically needed, \
                             write a one-shot migration instruction to rewrite every account.",
                        ),
                );
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, FieldDef, PrimitiveType, TypeRef};

    fn f(name: &str, ty: PrimitiveType) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty: TypeRef::primitive(ty),
            offset: None,
            size: None,
        }
    }

    fn account(name: &str, fields: Vec<FieldDef>) -> AccountDef {
        AccountDef {
            name: name.into(),
            discriminator: [0; 8],
            fields,
            size: None,
        }
    }

    fn surface_with(account: AccountDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(account.name.clone(), account);
        s
    }

    #[test]
    fn identical_fields_produce_no_findings() {
        let old = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("balance", PrimitiveType::U64)],
        ));
        let findings = AccountFieldReorder.check(&old, &old, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn reordering_shared_fields_is_breaking() {
        let old = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("balance", PrimitiveType::U64)],
        ));
        let new = surface_with(account(
            "Vault",
            vec![f("balance", PrimitiveType::U64), f("owner", PrimitiveType::Pubkey)],
        ));
        let findings = AccountFieldReorder.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.path, vec!["account:Vault"]);
        assert!(f.allow_flag.is_none(), "reorder must have no escape hatch");
    }

    #[test]
    fn appending_a_field_is_not_reorder() {
        // R001 only fires on reorder of shared fields; adding a field at the
        // end leaves the shared-field order unchanged and is handled by
        // other rules.
        let old = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("balance", PrimitiveType::U64)],
        ));
        let new = surface_with(account(
            "Vault",
            vec![
                f("owner", PrimitiveType::Pubkey),
                f("balance", PrimitiveType::U64),
                f("bump", PrimitiveType::U8),
            ],
        ));
        let findings = AccountFieldReorder.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn removing_a_field_is_not_reorder() {
        let old = surface_with(account(
            "Vault",
            vec![
                f("owner", PrimitiveType::Pubkey),
                f("balance", PrimitiveType::U64),
                f("bump", PrimitiveType::U8),
            ],
        ));
        let new = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("balance", PrimitiveType::U64)],
        ));
        let findings = AccountFieldReorder.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn new_accounts_are_ignored() {
        let old = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        let new = surface_with(account("NewThing", vec![f("a", PrimitiveType::U64)]));
        let findings = AccountFieldReorder.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn single_shared_field_is_not_reorder() {
        // If only one field is shared there is nothing to reorder.
        let old = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("balance", PrimitiveType::U64)],
        ));
        let new = surface_with(account(
            "Vault",
            vec![f("owner", PrimitiveType::Pubkey), f("nonce", PrimitiveType::U8)],
        ));
        let findings = AccountFieldReorder.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }
}
