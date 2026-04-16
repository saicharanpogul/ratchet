//! R007 — instruction-removed.
//!
//! An instruction exported by the deployed program is absent from the new
//! version. Every existing client that calls it will receive
//! `InstructionFallbackNotFound` (Anchor) or a raw dispatch failure.
//!
//! Breaking, with an `allow-ix-removal` flag for the case where the author
//! has confirmed no active caller exists.

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::ProgramSurface;

pub const ID: &str = "R007";
pub const NAME: &str = "instruction-removed";
pub const DESCRIPTION: &str =
    "An instruction was removed from the program; every client that calls it will fail.";

pub struct InstructionRemoved;

impl Rule for InstructionRemoved {
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
        for name in old.instructions.keys() {
            if new.instructions.contains_key(name) {
                continue;
            }
            findings.push(
                self.finding(Severity::Breaking)
                    .at([format!("ix:{name}")])
                    .message(format!(
                        "instruction `{name}` was removed; any client that still calls it will fail"
                    ))
                    .allow_flag("allow-ix-removal")
                    .suggestion(
                        "Keep the instruction declared but make its body a no-op (or return \
                         a specific error) for one release cycle before removing entirely.",
                    ),
            );
        }
        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::InstructionDef;

    fn ix(name: &str) -> InstructionDef {
        InstructionDef {
            name: name.into(),
            discriminator: [0; 8],
            args: vec![],
            accounts: vec![],
        }
    }

    fn surface_with<I: IntoIterator<Item = InstructionDef>>(ixs: I) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        for i in ixs {
            s.instructions.insert(i.name.clone(), i);
        }
        s
    }

    #[test]
    fn identical_instruction_set_no_finding() {
        let s = surface_with([ix("foo"), ix("bar")]);
        assert!(InstructionRemoved
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn removed_instruction_is_breaking() {
        let old = surface_with([ix("foo"), ix("bar")]);
        let new = surface_with([ix("foo")]);
        let findings = InstructionRemoved.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.rule_id, ID);
        assert_eq!(f.severity, Severity::Breaking);
        assert_eq!(f.path, vec!["ix:bar"]);
        assert_eq!(f.allow_flag.as_deref(), Some("allow-ix-removal"));
    }

    #[test]
    fn adding_instructions_is_not_a_removal() {
        let old = surface_with([ix("foo")]);
        let new = surface_with([ix("foo"), ix("bar")]);
        assert!(InstructionRemoved
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
