//! R005 — account-field-append.
//!
//! A new field was appended at the end of an existing account. This is the
//! safest shape of account growth, but it still needs a story: the existing
//! on-chain accounts don't have those bytes yet. Until the account is
//! reallocated (via `Migration<From, To>` or a manual `realloc` in an
//! update instruction), the new binary will fail to deserialize them.
//!
//! Emitted as `Unsafe` by default with an `allow-field-append` escape hatch.
//! When the account is listed in `CheckContext::migrated_accounts`, the
//! rule reports it as `Additive` — the developer explicitly promised a
//! migration.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R005";
pub const NAME: &str = "account-field-append";
pub const DESCRIPTION: &str =
    "A new field was appended to an account; existing accounts must be reallocated before the new binary can read them.";

pub struct AccountFieldAppend;

impl Rule for AccountFieldAppend {
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
            let old_names: HashSet<&str> =
                old_acc.fields.iter().map(|f| f.name.as_str()).collect();

            for (idx, new_field) in new_acc.fields.iter().enumerate() {
                if old_names.contains(new_field.name.as_str()) {
                    continue;
                }
                // Appended iff no shared field follows it.
                let is_append = !new_acc
                    .fields
                    .iter()
                    .skip(idx + 1)
                    .any(|f| old_names.contains(f.name.as_str()));
                if !is_append {
                    continue; // handled by R004
                }

                let has_migration = ctx.has_migration(name);
                let severity = if has_migration {
                    Severity::Additive
                } else {
                    Severity::Unsafe
                };
                let mut finding = self
                    .finding(severity)
                    .at([
                        format!("account:{name}"),
                        format!("field:{}", new_field.name),
                    ])
                    .new_value(format!("{}", new_field.ty));
                if has_migration {
                    finding = finding.message(format!(
                        "field `{}.{}` appended; migration declared for `{}`, safe to apply",
                        name, new_field.name, name
                    ));
                } else {
                    finding = finding
                        .message(format!(
                            "field `{}.{}` ({}) appended; existing accounts lack these bytes and must be reallocated",
                            name, new_field.name, new_field.ty
                        ))
                        .allow_flag("allow-field-append")
                        .suggestion(
                            "Reallocate existing accounts in an update instruction, or wrap \
                             the account with `Migration<Old, New>` (Anchor 1.0+) so the \
                             runtime handles old vs new layouts.",
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
    use crate::engine::check;
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
        let s = surface(acc(vec![f("a", PrimitiveType::U64)]));
        assert!(AccountFieldAppend
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn tail_append_is_unsafe() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("extra", PrimitiveType::U32),
        ]));
        let findings = AccountFieldAppend.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Unsafe);
        assert_eq!(findings[0].allow_flag.as_deref(), Some("allow-field-append"));
    }

    #[test]
    fn mid_insertion_is_not_caught_by_this_rule() {
        let old = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("b", PrimitiveType::U8),
        ]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("middle", PrimitiveType::U32),
            f("b", PrimitiveType::U8),
        ]));
        assert!(AccountFieldAppend
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn migration_declaration_demotes_to_additive_directly() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("extra", PrimitiveType::U32),
        ]));
        let ctx = CheckContext::new().with_migration("Vault");
        let findings = AccountFieldAppend.check(&old, &new, &ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
        assert!(findings[0].allow_flag.is_none());
    }

    #[test]
    fn unsafe_flag_demoted_by_engine() {
        let old = surface(acc(vec![f("a", PrimitiveType::U64)]));
        let new = surface(acc(vec![
            f("a", PrimitiveType::U64),
            f("extra", PrimitiveType::U32),
        ]));
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(AccountFieldAppend)];
        let ctx = CheckContext::new().with_allow("allow-field-append");
        let report = check(&old, &new, &ctx, &rules);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, Severity::Additive);
        assert_eq!(report.exit_code(), 0);
    }
}
