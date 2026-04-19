//! P001 — account-missing-version-field.
//!
//! Without a leading `version: u8` (or `u16`) field, there's no way
//! to discriminate old-layout bytes from new-layout bytes at
//! deserialization time. When the account layout eventually changes,
//! the program can't branch on version in its `try_deserialize` path
//! — it has to do a one-shot migration that rewrites every account,
//! which is a much harder upgrade to ship safely.
//!
//! Emitted as `Unsafe` with an `allow-no-version-field` flag. Ship
//! the flag if the account is genuinely immutable or if you've
//! decided migration-program-per-release is the preferred pattern.

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::{PrimitiveType, ProgramSurface, TypeRef};

pub const ID: &str = "P001";
pub const NAME: &str = "account-missing-version-field";
pub const DESCRIPTION: &str =
    "Accounts without a leading `version: u8` have no in-band signal for layout-version branching, making future migrations materially harder.";

pub struct AccountMissingVersionField;

impl PreflightRule for AccountMissingVersionField {
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
        for (name, account) in &surface.accounts {
            if account.fields.is_empty() {
                continue;
            }
            let first = &account.fields[0];
            if is_version_field(first) {
                continue;
            }
            findings.push(
                self.finding(Severity::Unsafe)
                    .at([format!("account:{name}")])
                    .message(format!(
                        "account `{name}` has no leading `version` field; future schema changes can't branch on layout version at deserialize time"
                    ))
                    .suggestion(
                        "Add `pub version: u8` (or u16) as the first field. Initialise it to 1 on creation and bump on every layout change so `try_deserialize` can route old vs new bytes.",
                    )
                    .allow_flag("allow-no-version-field"),
            );
        }
        findings
    }
}

fn is_version_field(f: &crate::surface::FieldDef) -> bool {
    let name_ok = matches!(
        f.name.as_str(),
        "version" | "_version" | "__version" | "schema_version"
    );
    let ty_ok = matches!(
        &f.ty,
        TypeRef::Primitive {
            ty: PrimitiveType::U8 | PrimitiveType::U16
        }
    );
    name_ok && ty_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, FieldDef, ProgramSurface};

    fn f(name: &str, ty: PrimitiveType) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty: TypeRef::primitive(ty),
            offset: None,
            size: None,
        }
    }

    fn surface_with(name: &str, fields: Vec<FieldDef>) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.accounts.insert(
            name.into(),
            AccountDef {
                name: name.into(),
                discriminator: [0; 8],
                fields,
                size: None,
            },
        );
        s
    }

    #[test]
    fn account_with_version_u8_first_passes() {
        let s = surface_with(
            "Vault",
            vec![
                f("version", PrimitiveType::U8),
                f("owner", PrimitiveType::Pubkey),
            ],
        );
        assert!(AccountMissingVersionField
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn account_with_version_u16_first_also_passes() {
        let s = surface_with(
            "Vault",
            vec![
                f("version", PrimitiveType::U16),
                f("owner", PrimitiveType::Pubkey),
            ],
        );
        assert!(AccountMissingVersionField
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn account_with_version_not_first_is_flagged() {
        let s = surface_with(
            "Vault",
            vec![
                f("owner", PrimitiveType::Pubkey),
                f("version", PrimitiveType::U8),
            ],
        );
        let findings = AccountMissingVersionField.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Unsafe);
    }

    #[test]
    fn account_without_any_version_field_is_flagged() {
        let s = surface_with(
            "Vault",
            vec![
                f("owner", PrimitiveType::Pubkey),
                f("balance", PrimitiveType::U64),
            ],
        );
        let findings = AccountMissingVersionField.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["account:Vault"]);
        assert_eq!(
            findings[0].allow_flag.as_deref(),
            Some("allow-no-version-field")
        );
    }

    #[test]
    fn empty_account_is_not_flagged() {
        let s = surface_with("EmptyMarker", vec![]);
        assert!(AccountMissingVersionField
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn wrong_type_version_field_is_still_flagged() {
        // `version: u64` isn't wrong per se but doesn't match the
        // common convention this rule enforces.
        let s = surface_with(
            "Vault",
            vec![
                f("version", PrimitiveType::U64),
                f("owner", PrimitiveType::Pubkey),
            ],
        );
        let findings = AccountMissingVersionField.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }
}
