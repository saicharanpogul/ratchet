//! R002 — account-field-retype.
//!
//! A field kept its name but changed type (`u32 → u64`, `Pubkey → u8`,
//! `Option<u8> → u8`, etc.). Any primitive size change shifts every byte
//! offset after the field; same-size changes still change the semantics of
//! existing data and will almost always break clients.
//!
//! Emitted as `Breaking` with an `allow-type-change` escape hatch for the
//! rare case where a size-preserving retype (e.g. `u64 → i64`) is
//! deliberate.

use std::collections::HashMap;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{ProgramSurface, TypeRef};

pub const ID: &str = "R002";
pub const NAME: &str = "account-field-retype";
pub const DESCRIPTION: &str =
    "A shared account field changed type, which shifts byte offsets and usually corrupts data.";

pub struct AccountFieldRetype;

impl Rule for AccountFieldRetype {
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
            let new_fields: HashMap<&str, &TypeRef> = new_acc
                .fields
                .iter()
                .map(|f| (f.name.as_str(), &f.ty))
                .collect();
            for old_field in &old_acc.fields {
                if let Some(new_ty) = new_fields.get(old_field.name.as_str()) {
                    if old_field.ty != **new_ty {
                        findings.push(
                            self.finding(Severity::Breaking)
                                .at([
                                    format!("account:{name}"),
                                    format!("field:{}", old_field.name),
                                ])
                                .message(format!(
                                    "field `{}.{}` type changed: {} → {}",
                                    name, old_field.name, old_field.ty, new_ty
                                ))
                                .old(format!("{}", old_field.ty))
                                .new_value(format!("{new_ty}"))
                                .allow_flag("allow-type-change")
                                .suggestion(
                                    "Changing a field's type shifts every later byte offset. \
                                     Keep the original type; if the new representation is \
                                     essential, write a migration instruction.",
                                ),
                        );
                    }
                }
            }
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, FieldDef, PrimitiveType, TypeRef};

    fn f(name: &str, ty: TypeRef) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty,
            offset: None,
            size: None,
        }
    }

    fn prim(p: PrimitiveType) -> TypeRef {
        TypeRef::primitive(p)
    }

    fn surface_with(acc: AccountDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(acc.name.clone(), acc);
        s
    }

    fn account(fields: Vec<FieldDef>) -> AccountDef {
        AccountDef {
            name: "Vault".into(),
            discriminator: [0; 8],
            fields,
            size: None,
        }
    }

    #[test]
    fn identical_types_produce_no_finding() {
        let old = surface_with(account(vec![f("balance", prim(PrimitiveType::U64))]));
        let findings = AccountFieldRetype.check(&old, &old, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn primitive_retype_is_breaking_with_allow() {
        let old = surface_with(account(vec![f("balance", prim(PrimitiveType::U32))]));
        let new = surface_with(account(vec![f("balance", prim(PrimitiveType::U64))]));
        let findings = AccountFieldRetype.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-type-change"));
        assert_eq!(f.old.as_deref(), Some("u32"));
        assert_eq!(f.new.as_deref(), Some("u64"));
    }

    #[test]
    fn primitive_to_composite_retype_caught() {
        let old = surface_with(account(vec![f("payload", prim(PrimitiveType::U64))]));
        let new = surface_with(account(vec![f(
            "payload",
            TypeRef::Option {
                ty: Box::new(prim(PrimitiveType::U64)),
            },
        )]));
        let findings = AccountFieldRetype.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].new.as_deref(), Some("Option<u64>"));
    }

    #[test]
    fn removed_and_added_fields_are_ignored() {
        let old = surface_with(account(vec![f("a", prim(PrimitiveType::U64))]));
        let new = surface_with(account(vec![f("b", prim(PrimitiveType::U64))]));
        let findings = AccountFieldRetype.check(&old, &new, &CheckContext::new());
        assert!(findings.is_empty());
    }

    #[test]
    fn flag_demotion_handled_by_engine() {
        // Rule always emits Breaking; demotion is engine's job. Verify via
        // the full engine pass that --unsafe allow-type-change clears it.
        use crate::engine::check;
        let old = surface_with(account(vec![f("balance", prim(PrimitiveType::U32))]));
        let new = surface_with(account(vec![f("balance", prim(PrimitiveType::U64))]));
        let rules: Vec<Box<dyn Rule>> = vec![Box::new(AccountFieldRetype)];
        let ctx = CheckContext::new().with_allow("allow-type-change");
        let report = check(&old, &new, &ctx, &rules);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, Severity::Additive);
        assert_eq!(report.exit_code(), 0);
    }
}
