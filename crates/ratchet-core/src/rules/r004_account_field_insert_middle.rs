//! R004 — account-field-insert-middle.
//!
//! A brand-new field was inserted before one or more existing fields. With
//! Borsh, this shifts the byte offsets of every field after the insertion
//! point, so the program reads garbage out of every existing account.
//! Appending at the end is a different rule (R005) with a different
//! remediation.
//!
//! Severity:
//! - `Breaking` by default.
//! - Demoted to `Additive` when the account is listed in
//!   [`CheckContext::migrated_accounts`] — Migration<From, To> can
//!   handle any layout change.
//! - `allow-field-insert` escape hatch for the flag-day case.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R004";
pub const NAME: &str = "account-field-insert-middle";
pub const DESCRIPTION: &str =
    "A new field was inserted before an existing field; every later offset shifts and existing accounts deserialize to garbage.";

pub struct AccountFieldInsertMiddle;

impl Rule for AccountFieldInsertMiddle {
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
        ctx: &CheckContext,
    ) -> Vec<Finding> {
        let mut findings = Vec::new();
        for (name, old_acc) in &old.accounts {
            let Some(new_acc) = new.accounts.get(name) else {
                continue;
            };
            let old_names: HashSet<&str> = old_acc.fields.iter().map(|f| f.name.as_str()).collect();
            let has_migration = ctx.has_migration(name);

            for (idx, new_field) in new_acc.fields.iter().enumerate() {
                if old_names.contains(new_field.name.as_str()) {
                    continue;
                }
                let has_shared_after = new_acc
                    .fields
                    .iter()
                    .skip(idx + 1)
                    .any(|f| old_names.contains(f.name.as_str()));
                if !has_shared_after {
                    continue;
                }

                let severity = if has_migration {
                    Severity::Additive
                } else {
                    Severity::Breaking
                };
                let message = if has_migration {
                    format!(
                        "new field `{}.{}` ({}) inserted before existing fields; migration declared for `{}` (not verified by ratchet)",
                        name, new_field.name, new_field.ty, name
                    )
                } else {
                    format!(
                        "new field `{}.{}` ({}) was inserted before existing fields; \
                         Borsh layout shifts every subsequent offset",
                        name, new_field.name, new_field.ty
                    )
                };
                let mut finding = self
                    .finding(severity)
                    .at([
                        format!("account:{name}"),
                        format!("field:{}", new_field.name),
                    ])
                    .message(message)
                    .new_value(format!("{}", new_field.ty));
                if !has_migration {
                    finding = finding.allow_flag("allow-field-insert").suggestion(
                        "Append new fields at the end of the struct, declare the account \
                             in --migrated-account, or add them in a migration instruction \
                             that rewrites every account.",
                    );
                }
                findings.push(finding);
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

    fn acc(fields: Vec<FieldDef>) -> AccountDef {
        AccountDef {
            name: "Vault".into(),
            discriminator: [0; 8],
            fields,
            size: None,
        }
    }

    fn surface(account: AccountDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(account.name.clone(), account);
        s
    }

    #[test]
    fn identical_surface_no_findings() {
        let s = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        assert!(AccountFieldInsertMiddle
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn insert_at_start_is_breaking() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("new_front", PrimitiveType::U32),
            f("a", PrimitiveType::U64),
        ]));
        let findings = AccountFieldInsertMiddle.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["account:Vault", "field:new_front"]);
        assert_eq!(findings[0].severity, Severity::Breaking);
    }

    #[test]
    fn insert_between_shared_fields_is_breaking() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("middle", PrimitiveType::U32),
            f("b", PrimitiveType::U8),
        ]));
        let findings = AccountFieldInsertMiddle.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["account:Vault", "field:middle"]);
    }

    #[test]
    fn append_is_not_caught_by_this_rule() {
        // R005 handles pure appends with size growth.
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("bump", PrimitiveType::U8),
        ]));
        let findings = AccountFieldInsertMiddle.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn new_account_not_in_old_is_ignored() {
        let old = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        assert!(AccountFieldInsertMiddle
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn migration_declaration_demotes_to_additive() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("middle", PrimitiveType::U32),
            f("b", PrimitiveType::U8),
        ]));
        let ctx = CheckContext::new().with_migration("Vault");
        let findings = AccountFieldInsertMiddle.check(&old, &new, &ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
        assert!(findings[0].allow_flag.is_none());
    }

    #[test]
    fn insert_then_append_reports_only_the_insert() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("inserted", PrimitiveType::U32),
            f("a", PrimitiveType::U64),
            f("appended", PrimitiveType::U8),
        ]));
        let findings = AccountFieldInsertMiddle.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["account:Vault", "field:inserted"]);
    }
}
