//! R014 — instruction-discriminator-change.
//!
//! Symmetric to R006 but for instructions. Every Anchor instruction is
//! routed at dispatch by matching the first 8 bytes of the transaction's
//! instruction data against the expected discriminator. If those bytes
//! change (whether via `#[instruction(discriminator = ...)]`, a rename
//! whose default convention produces a different hash, or an on-chain
//! override), existing clients call what they think is the same ix but
//! receive `InstructionFallbackNotFound` or dispatch into the wrong
//! handler.
//!
//! Emitted as `Breaking` with an `allow-ix-rename` escape hatch for the
//! deliberate rebind case.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R014";
pub const NAME: &str = "instruction-discriminator-change";
pub const DESCRIPTION: &str =
    "An instruction's discriminator changed; every existing caller routes to the wrong dispatch slot.";

pub struct InstructionDiscriminatorChange;

impl Rule for InstructionDiscriminatorChange {
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
        for (name, old_ix) in &old.instructions {
            let Some(new_ix) = new.instructions.get(name) else {
                continue;
            };
            if old_ix.discriminator == new_ix.discriminator {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("ix:{name}"), "discriminator".into()])
                    .message(format!(
                        "discriminator of instruction `{name}` changed: {} → {}",
                        hex(&old_ix.discriminator),
                        hex(&new_ix.discriminator)
                    ))
                    .old(hex(&old_ix.discriminator))
                    .new_value(hex(&new_ix.discriminator))
                    .allow_flag("allow-ix-rename")
                    .suggestion(
                        "If this was an accidental rename of the handler function, restore \
                         the original name. If the rebind is deliberate, pin the original \
                         discriminator with `#[instruction(discriminator = <bytes>)]` \
                         (Anchor 0.31+) so existing callers keep working.",
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
    use crate::surface::InstructionDef;

    fn ix(name: &str, disc: [u8; 8]) -> InstructionDef {
        InstructionDef {
            name: name.into(),
            discriminator: disc,
            args: vec![],
            accounts: vec![],
        }
    }

    fn surface_with(i: InstructionDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(i.name.clone(), i);
        s
    }

    #[test]
    fn identical_discriminators_no_finding() {
        let s = surface_with(ix("deposit", [1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(InstructionDiscriminatorChange
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn discriminator_change_is_breaking_with_allow() {
        let old = surface_with(ix("deposit", [1, 2, 3, 4, 5, 6, 7, 8]));
        let new = surface_with(ix("deposit", [9, 10, 11, 12, 13, 14, 15, 16]));
        let findings = InstructionDiscriminatorChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-ix-rename"));
        assert_eq!(f.old.as_deref(), Some("0x0102030405060708"));
        assert_eq!(f.new.as_deref(), Some("0x090a0b0c0d0e0f10"));
        assert_eq!(f.path, vec!["ix:deposit", "discriminator"]);
    }

    #[test]
    fn removed_ix_not_in_scope() {
        // Removal is R007's job; R014 requires both sides to have the ix.
        let old = surface_with(ix("deposit", [1; 8]));
        let new = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        assert!(InstructionDiscriminatorChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn new_ix_not_in_scope() {
        let old = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        let new = surface_with(ix("deposit", [1; 8]));
        assert!(InstructionDiscriminatorChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
