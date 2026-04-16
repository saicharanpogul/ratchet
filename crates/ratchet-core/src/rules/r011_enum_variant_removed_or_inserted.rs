//! R011 — enum-variant-removed-or-inserted.
//!
//! Borsh tags enum variants by ordinal — the first declared variant is
//! index 0, the second is 1, and so on. Removing a variant frees an
//! ordinal that existing serialized data still uses, and inserting a new
//! variant anywhere but the end renumbers every later variant.
//!
//! Both cases produce Breaking findings. Tail appends are handled by R012
//! as safe additive changes.

use std::collections::HashSet;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{ProgramSurface, TypeDef};

pub const ID: &str = "R011";
pub const NAME: &str = "enum-variant-removed-or-inserted";
pub const DESCRIPTION: &str =
    "An enum variant was removed or inserted before an existing variant, shifting every later Borsh ordinal.";

pub struct EnumVariantRemovedOrInserted;

impl Rule for EnumVariantRemovedOrInserted {
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
            let TypeDef::Enum { variants: old_variants } = old_ty else {
                continue;
            };
            let Some(TypeDef::Enum {
                variants: new_variants,
            }) = new.types.get(name)
            else {
                continue;
            };

            let old_names: Vec<&str> = old_variants.iter().map(|v| v.name.as_str()).collect();
            let new_names: Vec<&str> = new_variants.iter().map(|v| v.name.as_str()).collect();
            let old_set: HashSet<&str> = old_names.iter().copied().collect();
            let new_set: HashSet<&str> = new_names.iter().copied().collect();

            // (a) Variants removed.
            for old_variant in &old_names {
                if !new_set.contains(*old_variant) {
                    findings.push(
                        self.finding(Severity::Breaking)
                            .at([format!("type:{name}"), format!("variant:{old_variant}")])
                            .message(format!(
                                "enum variant `{name}::{old_variant}` was removed; serialized data with that ordinal now deserializes to garbage"
                            ))
                            .old(old_variant.to_string())
                            .suggestion(
                                "Never remove an enum variant. Keep it declared — even if \
                                 unused — and only remove once every last persisted value \
                                 has been migrated away from it.",
                            ),
                    );
                }
            }

            // (b) Variants inserted before one or more shared variants.
            for (idx, new_variant) in new_names.iter().enumerate() {
                if old_set.contains(*new_variant) {
                    continue; // shared, not newly inserted
                }
                let has_shared_after = new_names
                    .iter()
                    .skip(idx + 1)
                    .any(|n| old_set.contains(n));
                if has_shared_after {
                    findings.push(
                        self.finding(Severity::Breaking)
                            .at([format!("type:{name}"), format!("variant:{new_variant}")])
                            .message(format!(
                                "enum variant `{name}::{new_variant}` inserted before existing variants; Borsh ordinals of every later variant shift"
                            ))
                            .new_value(new_variant.to_string())
                            .suggestion(
                                "Append new variants at the end of the enum so existing \
                                 ordinals stay stable.",
                            ),
                    );
                }
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
    fn identical_enums_no_finding() {
        let s = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        assert!(EnumVariantRemovedOrInserted
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn removing_a_variant_is_breaking() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum("Side", vec![variant("Bid")]);
        let findings = EnumVariantRemovedOrInserted.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["type:Side", "variant:Ask"]);
        assert_eq!(findings[0].severity, Severity::Breaking);
    }

    #[test]
    fn inserting_variant_at_start_is_breaking() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Auction"), variant("Bid"), variant("Ask")],
        );
        let findings = EnumVariantRemovedOrInserted.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["type:Side", "variant:Auction"]);
    }

    #[test]
    fn inserting_variant_in_middle_is_breaking() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Bid"), variant("Cross"), variant("Ask")],
        );
        let findings = EnumVariantRemovedOrInserted.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, vec!["type:Side", "variant:Cross"]);
    }

    #[test]
    fn appending_variant_is_not_this_rules_scope() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Bid"), variant("Ask"), variant("Cross")],
        );
        assert!(EnumVariantRemovedOrInserted
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn remove_and_insert_emit_two_findings() {
        let old = surface_with_enum("Side", vec![variant("Bid"), variant("Ask"), variant("Stop")]);
        let new = surface_with_enum(
            "Side",
            vec![variant("Cross"), variant("Bid"), variant("Ask")],
        );
        // `Stop` removed, `Cross` inserted at start → 2 findings.
        let findings = EnumVariantRemovedOrInserted.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn struct_types_are_ignored() {
        let mut old = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        old.types
            .insert("NotAnEnum".into(), TypeDef::Struct { fields: vec![] });
        let new = old.clone();
        assert!(EnumVariantRemovedOrInserted
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
