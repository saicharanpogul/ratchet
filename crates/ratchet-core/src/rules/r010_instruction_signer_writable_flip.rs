//! R010 — instruction-signer-writable-flip.
//!
//! An account slot with the same name had its `is_signer` or `is_writable`
//! flag tightened between versions. Tightening (`false → true`) changes
//! caller obligations: a previously unsigned slot now requires a signature,
//! or a previously readonly slot now demands mut. Existing clients don't
//! update, so their transactions fail pre-flight.
//!
//! Relaxation (`true → false`) is safe — the Solana runtime accepts
//! transactions whose `AccountMeta` is stricter than the program needs.
//! We still emit an `Additive` finding so the change shows up in JSON
//! reports, just without failing CI.
//!
//! Tightening is `Breaking` with `allow-signer-mut-flip` escape hatch.

use std::collections::HashMap;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{AccountInput, ProgramSurface};

pub const ID: &str = "R010";
pub const NAME: &str = "instruction-signer-writable-flip";
pub const DESCRIPTION: &str =
    "An instruction slot toggled is_signer or is_writable; existing callers send the wrong account metas.";

pub struct InstructionSignerWritableFlip;

impl Rule for InstructionSignerWritableFlip {
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
            let new_by_name: HashMap<&str, &AccountInput> = new_ix
                .accounts
                .iter()
                .map(|a| (a.name.as_str(), a))
                .collect();
            for old_acc in &old_ix.accounts {
                let Some(new_acc) = new_by_name.get(old_acc.name.as_str()) else {
                    continue;
                };
                let signer_tightened = !old_acc.is_signer && new_acc.is_signer;
                let signer_relaxed = old_acc.is_signer && !new_acc.is_signer;
                let writable_tightened = !old_acc.is_writable && new_acc.is_writable;
                let writable_relaxed = old_acc.is_writable && !new_acc.is_writable;
                if !signer_tightened && !signer_relaxed && !writable_tightened && !writable_relaxed
                {
                    continue;
                }

                let tightening = signer_tightened || writable_tightened;

                let mut changes = Vec::new();
                if signer_tightened || signer_relaxed {
                    changes.push(format!(
                        "is_signer: {} → {}",
                        old_acc.is_signer, new_acc.is_signer
                    ));
                }
                if writable_tightened || writable_relaxed {
                    changes.push(format!(
                        "is_writable: {} → {}",
                        old_acc.is_writable, new_acc.is_writable
                    ));
                }

                let severity = if tightening {
                    Severity::Breaking
                } else {
                    Severity::Additive
                };
                let message = if tightening {
                    format!(
                        "account `{}` of `{name}` tightened: {} — existing callers will fail pre-flight",
                        old_acc.name,
                        changes.join(", ")
                    )
                } else {
                    format!(
                        "account `{}` of `{name}` relaxed: {} — existing callers stay compatible",
                        old_acc.name,
                        changes.join(", ")
                    )
                };

                let mut finding = self
                    .finding(severity)
                    .at([format!("ix:{name}"), format!("account:{}", old_acc.name)])
                    .message(message)
                    .old(flags(old_acc))
                    .new_value(flags(new_acc));
                if tightening {
                    finding = finding.allow_flag("allow-signer-mut-flip").suggestion(
                        "Existing transactions encode the signer/writable bits in their \
                             AccountMeta list. Tightening breaks pre-flight; coordinate the \
                             rollout with callers or create a new instruction.",
                    );
                }
                findings.push(finding);
            }
        }
        findings
    }
}

fn flags(acc: &AccountInput) -> String {
    let mut f = Vec::new();
    if acc.is_signer {
        f.push("signer");
    }
    if acc.is_writable {
        f.push("mut");
    }
    if f.is_empty() {
        "(none)".into()
    } else {
        f.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::InstructionDef;

    fn acc(name: &str, signer: bool, writable: bool) -> AccountInput {
        AccountInput {
            name: name.into(),
            is_signer: signer,
            is_writable: writable,
            is_optional: false,
            pda: None,
        }
    }

    fn ix(accounts: Vec<AccountInput>) -> InstructionDef {
        InstructionDef {
            name: "deposit".into(),
            discriminator: [0; 8],
            args: vec![],
            accounts,
        }
    }

    fn surface(i: InstructionDef) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(i.name.clone(), i);
        s
    }

    #[test]
    fn identical_flags_no_finding() {
        let s = surface(ix(vec![
            acc("user", true, false),
            acc("vault", false, true),
        ]));
        assert!(InstructionSignerWritableFlip
            .check(&s, &s, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn signer_tightening_is_breaking() {
        let old = surface(ix(vec![acc("user", false, false)]));
        let new = surface(ix(vec![acc("user", true, false)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert_eq!(f.severity, Severity::Breaking);
        assert!(f.message.contains("is_signer: false → true"));
        assert_eq!(f.old.as_deref(), Some("(none)"));
        assert_eq!(f.new.as_deref(), Some("signer"));
        assert_eq!(f.allow_flag.as_deref(), Some("allow-signer-mut-flip"));
    }

    #[test]
    fn writable_tightening_is_breaking() {
        let old = surface(ix(vec![acc("vault", false, false)]));
        let new = surface(ix(vec![acc("vault", false, true)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Breaking);
        assert!(findings[0].message.contains("is_writable: false → true"));
    }

    #[test]
    fn signer_relaxation_is_additive() {
        let old = surface(ix(vec![acc("user", true, false)]));
        let new = surface(ix(vec![acc("user", false, false)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
        assert!(findings[0].allow_flag.is_none());
        assert!(findings[0].message.contains("relaxed"));
    }

    #[test]
    fn writable_relaxation_is_additive() {
        let old = surface(ix(vec![acc("vault", false, true)]));
        let new = surface(ix(vec![acc("vault", false, false)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Additive);
    }

    #[test]
    fn both_tightening_emit_single_combined_finding() {
        let old = surface(ix(vec![acc("mixed", false, false)]));
        let new = surface(ix(vec![acc("mixed", true, true)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Breaking);
        assert!(findings[0].message.contains("is_signer"));
        assert!(findings[0].message.contains("is_writable"));
    }

    #[test]
    fn mixed_tighten_and_relax_is_breaking_overall() {
        // Signer tightens while writable relaxes — net: Breaking, since
        // any tightening breaks callers regardless of compensating relaxation.
        let old = surface(ix(vec![acc("m", false, true)]));
        let new = surface(ix(vec![acc("m", true, false)]));
        let findings = InstructionSignerWritableFlip.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Breaking);
    }

    #[test]
    fn renamed_accounts_are_out_of_scope() {
        let old = surface(ix(vec![acc("user", true, false)]));
        let new = surface(ix(vec![acc("owner", false, false)]));
        assert!(InstructionSignerWritableFlip
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }
}
