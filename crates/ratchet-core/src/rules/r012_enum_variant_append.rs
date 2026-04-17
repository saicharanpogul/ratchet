//! R012 — enum-variant-append.
//!
//! A new enum variant was appended after all existing variants. Because
//! Borsh ordinals are assigned in declaration order, appending to the
//! tail does not shift any existing ordinal, so old serialized data still
//! decodes correctly.
//!
//! Emits `Additive` findings purely for visibility — the check does not
//! fail CI. Users running `ratchet --json` get an explicit record of every
//! safe enum change.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{ProgramSurface, TypeDef};

pub const ID: &str = "R012";
pub const NAME: &str = "enum-variant-append";
pub const DESCRIPTION: &str =
    "A new enum variant was appended to the tail of an existing enum (safe, additive).";

pub struct EnumVariantAppend;

impl Rule for EnumVariantAppend {
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
        for (name, old_ty) in &old.types {
            let TypeDef::Enum {
                variants: old_variants,
            } = old_ty
            else {
                continue;
            };
            let Some(TypeDef::Enum {
                variants: new_variants,
            }) = new.types.get(name)
            else {
                continue;
            };

            let old_names: HashSet<&str> = old_variants.iter().map(|v| v.name.as_str()).collect();

            for (idx, new_variant) in new_variants.iter().enumerate() {
                if old_names.contains(new_variant.name.as_str()) {
                    continue;
                }
                let has_shared_after = new_variants
                    .iter()
                    .skip(idx + 1)
                    .any(|v| old_names.contains(v.name.as_str()));
                if has_shared_after {
                    continue; // non-tail insert — R011's job
                }
                findings.push(
                    self.finding(Severity::Additive)
                        .at([
                            format!("type:{name}"),
                            format!("variant:{}", new_variant.name),
                        ])
                        .message(format!(
                            "enum variant `{name}::{}` appended to the tail (Borsh ordinals unchanged)",
                            new_variant.name
                        ))
                        .new_value(new_variant.name.clone()),
                );
            }
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{EnumVariant, EnumVariantFields};

    fn variant(name: &str) -> EnumVariant {
        EnumVariant {
            name: name.into(),
            fields: EnumVariantFields::Unit,
        }
    }

    fn surface_with_enum(name: &str, variants: Vec<EnumVariant>) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.types.insert(name.into(), TypeDef::Enum { variants });
        s
    }

    #[test]
    fn identical_enum_no_finding() {
        let s = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        assert!(EnumVariantAppend
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn tail_append_emits_additive_finding() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Bid"), variant("Ask"), variant("Cross")],
        );
        let findings = EnumVariantAppend.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
        assert_eq!(findings[0].path, vec!["type:Side", "variant:Cross"]);
    }

    #[test]
    fn multiple_tail_appends_emit_one_finding_each() {
        let old = surface_with_enum("Side", vec![variant("Bid")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Bid"), variant("Ask"), variant("Cross")],
        );
        let findings = EnumVariantAppend.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn mid_insert_is_not_this_rules_scope() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Bid"), variant("Cross"), variant("Ask")],
        );
        // Non-tail inserts are handled by R011; R012 stays silent.
        assert!(EnumVariantAppend
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
