//! R003 — account-field-removed.
//!
//! A field that existed in the deployed account no longer exists in the
//! new version. The bytes for that field are still sitting on-chain in
//! every existing account, so the new program will interpret them as part
//! of whatever follows, corrupting deserialization.
//!
//! Severity:
//! - `Breaking` by default; the only real fix is a migration instruction
//!   that rewrites and shrinks every existing account.
//! - Demoted to `Additive` when the account is listed in
//!   [`CheckContext::migrated_accounts`] — the developer has promised a
//!   Migration<From, To> wrapper that handles the old layout.
//! - `allow-field-removed` escape hatch for the "no active data at that
//!   field" case (rare but legitimate when a field was never written).

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R003";
pub const NAME: &str = "account-field-removed";
pub const DESCRIPTION: &str =
    "An existing account field was removed; the bytes are still on-chain and now alias the next field.";

pub struct AccountFieldRemoved;

impl Rule for AccountFieldRemoved {
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
            let new_names: HashSet<&str> = new_acc.fields.iter().map(|f| f.name.as_str()).collect();
            let has_migration = ctx.has_migration(name);
            for old_field in &old_acc.fields {
                if new_names.contains(old_field.name.as_str()) {
                    continue;
                }
                let severity = if has_migration {
                    Severity::Additive
                } else {
                    Severity::Breaking
                };
                let message = if has_migration {
                    format!(
                        "field `{}.{}` ({}) removed; migration declared for `{}` (not verified by ratchet)",
                        name, old_field.name, old_field.ty, name
                    )
                } else {
                    format!(
                        "field `{}.{}` ({}) was removed; its bytes remain on-chain and will be misread by the new program",
                        name, old_field.name, old_field.ty
                    )
                };
                let mut finding = self
                    .finding(severity)
                    .at([
                        format!("account:{name}"),
                        format!("field:{}", old_field.name),
                    ])
                    .message(message)
                    .old(format!("{}", old_field.ty));
                if !has_migration {
                    finding = finding.allow_flag("allow-field-removed").suggestion(
                        "Keep the field and stop using it, declare the account in \
                             --migrated-account, or write a migration instruction that \
                             rewrites every account with a new layout and shrinks via `realloc`.",
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
    fn identical_fields_produce_no_finding() {
        let s = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        assert!(AccountFieldRemoved
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn removed_field_is_breaking() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let findings = AccountFieldRemoved.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.path, vec!["account:Vault", "field:b"]);
        // Now carries an escape hatch for the rare "the field was never
        // actually populated" case, plus a pointer at --migrated-account.
        assert_eq!(f.allow_flag.as_deref(), Some("allow-field-removed"));
    }

    #[test]
    fn multiple_removals_each_emit_a_finding() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
            f("c", PrimitiveType::U32),
        ]));
        let new = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let findings = AccountFieldRemoved.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 2);
        let names: Vec<_> = findings.iter().map(|f| f.path[1].clone()).collect();
        assert!(names.contains(&"field:b".into()));
        assert!(names.contains(&"field:c".into()));
    }

    #[test]
    fn addition_is_not_removal() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        assert!(AccountFieldRemoved
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn missing_account_in_new_is_not_scope_of_this_rule() {
        // Account dropped entirely is a different rule (account-removed);
        // R003 only cares about field removal inside still-present accounts.
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        assert!(AccountFieldRemoved
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn migration_declaration_demotes_to_additive() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let ctx = CheckContext::new().with_migration("Vault");
        let findings = AccountFieldRemoved.check(&old, &new, &ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
        assert!(findings[0].allow_flag.is_none());
    }

    #[test]
    fn allow_flag_demotes_through_engine() {
        use crate::engine::check;
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(AccountFieldRemoved)];
        let ctx = CheckContext::new().with_allow("allow-field-removed");
        let report = check(&old, &new, &ctx, &rules);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, Severity::Additive);
    }
}
