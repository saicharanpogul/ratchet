//! P002 — account-missing-reserved-padding.
//!
//! Appending a field to an account that lacks trailing reserved
//! padding always requires a realloc-or-migrate step (R005 in the
//! diff engine). Adding, say, a 128-byte `_reserved` array on first
//! deploy is cheap — it's a one-time 128-byte rent cost per account —
//! and it turns every future "we need one more u64 on this account"
//! from an Unsafe upgrade into an Additive one.
//!
//! Emitted as `Unsafe` with `allow-no-reserved-padding` for programs
//! where size efficiency matters more than upgrade ease (e.g. massive
//! fan-out per-user accounts with strict rent budgets).

use crate::diagnostics::{Finding, Severity};
use crate::preflight::PreflightRule;
use crate::rule::CheckContext;
use crate::surface::{FieldDef, ProgramSurface, TypeRef};

pub const ID: &str = "P002";
pub const NAME: &str = "account-missing-reserved-padding";
pub const DESCRIPTION: &str =
    "Accounts without a trailing `_reserved: [u8; N]` padding require a realloc or migration on every future field append.";

pub struct AccountMissingReservedPadding;

impl PreflightRule for AccountMissingReservedPadding {
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
            let last = account.fields.last().unwrap();
            if is_reserved_padding(last) {
                continue;
            }
            findings.push(
                self.finding(Severity::Unsafe)
                    .at([format!("account:{name}")])
                    .message(format!(
                        "account `{name}` has no trailing `_reserved` padding; every future field append becomes Unsafe (needs realloc or migration)"
                    ))
                    .suggestion(
                        "Add `pub _reserved: [u8; 64]` (or similar) as the last field. Cheap at init time, converts future field appends to Additive.",
                    )
                    .allow_flag("allow-no-reserved-padding"),
            );
        }
        findings
    }
}

fn is_reserved_padding(f: &FieldDef) -> bool {
    // Accept common naming variants: _reserved, reserved, padding,
    // _padding. Type must be a fixed-size u8 array.
    let name_ok = matches!(
        f.name.as_str(),
        "_reserved" | "reserved" | "padding" | "_padding" | "__reserved"
    );
    if !name_ok {
        return false;
    }
    matches!(
        &f.ty,
        TypeRef::Array { ty, .. }
            if matches!(
                ty.as_ref(),
                TypeRef::Primitive {
                    ty: crate::surface::PrimitiveType::U8
                }
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountDef, FieldDef, PrimitiveType, TypeRef};

    fn primitive_field(name: &str) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty: TypeRef::primitive(PrimitiveType::U64),
            offset: None,
            size: None,
        }
    }

    fn reserved_field(name: &str, len: usize) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty: TypeRef::Array {
                ty: Box::new(TypeRef::primitive(PrimitiveType::U8)),
                len,
            },
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
    fn trailing_reserved_u8_array_passes() {
        let s = surface_with(
            "Vault",
            vec![primitive_field("balance"), reserved_field("_reserved", 64)],
        );
        assert!(AccountMissingReservedPadding
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn no_trailing_padding_is_flagged() {
        let s = surface_with(
            "Vault",
            vec![primitive_field("owner"), primitive_field("balance")],
        );
        let findings = AccountMissingReservedPadding.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Unsafe);
        assert_eq!(
            findings[0].allow_flag.as_deref(),
            Some("allow-no-reserved-padding")
        );
    }

    #[test]
    fn reserved_not_at_end_is_flagged() {
        let s = surface_with(
            "Vault",
            vec![reserved_field("_reserved", 64), primitive_field("balance")],
        );
        let findings = AccountMissingReservedPadding.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn wrong_reserved_type_is_flagged() {
        // reserved but typed as Vec<u8> — not statically sized,
        // doesn't give the same forward-compat guarantee.
        let s = surface_with(
            "Vault",
            vec![
                primitive_field("balance"),
                FieldDef {
                    name: "_reserved".into(),
                    ty: TypeRef::Vec {
                        ty: Box::new(TypeRef::primitive(PrimitiveType::U8)),
                    },
                    offset: None,
                    size: None,
                },
            ],
        );
        let findings = AccountMissingReservedPadding.check(&s, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn empty_account_is_not_flagged() {
        let s = surface_with("Marker", vec![]);
        assert!(AccountMissingReservedPadding
            .check(&s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn padding_alias_is_accepted() {
        let s = surface_with(
            "Vault",
            vec![primitive_field("balance"), reserved_field("padding", 32)],
        );
        assert!(AccountMissingReservedPadding
            .check(&s, &CheckContext::new())
            .is_empty());
    }
}
