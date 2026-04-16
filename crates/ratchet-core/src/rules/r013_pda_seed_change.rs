//! R013 — pda-seed-change.
//!
//! The PDA seed expression for an account input changed between versions.
//! Any change — a new component, a reordered component, a different byte
//! literal — maps the account to a completely different address.
//!
//! Every existing account keyed under the old seeds is now orphaned:
//! the program can't find it, and no client can rederive its address with
//! the new seed formula. There is no safe acknowledgement; PDA changes
//! must be paired with a one-shot migration ix that reads from the old
//! address and writes to the new one.
//!
//! The rule works on the PDA information captured in the IDL
//! (`AccountInput::pda`). Anchor 0.30+ emits seeds faithfully; older IDLs
//! that don't carry seed info are silently skipped (nothing to compare).

use std::collections::HashMap;

use crate::diagnostics::{Finding, Severity};
use crate::rule::{CheckContext, Rule};
use crate::surface::{AccountInput, PdaSpec, ProgramSurface, Seed};

pub const ID: &str = "R013";
pub const NAME: &str = "pda-seed-change";
pub const DESCRIPTION: &str =
    "A PDA account's seed expression changed; every existing account at the old address is now orphaned.";

pub struct PdaSeedChange;

impl Rule for PdaSeedChange {
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
        for (ix_name, old_ix) in &old.instructions {
            let Some(new_ix) = new.instructions.get(ix_name) else {
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
                match (&old_acc.pda, &new_acc.pda) {
                    (None, None) | (None, Some(_)) | (Some(_), None) => {
                        // No baseline to compare; Anchor may have dropped
                        // seed info or this isn't a PDA in one version.
                        // Skip silently — catching these needs source-level
                        // analysis (deferred Phase 2 work).
                    }
                    (Some(old_pda), Some(new_pda)) => {
                        if !pdas_equal(old_pda, new_pda) {
                            findings.push(
                                self.finding(Severity::Breaking)
                                    .at([
                                        format!("ix:{ix_name}"),
                                        format!("account:{}", old_acc.name),
                                        "pda".into(),
                                    ])
                                    .message(format!(
                                        "PDA seeds of `{}.{}` changed: [{}] → [{}]",
                                        ix_name,
                                        old_acc.name,
                                        render_seeds(&old_pda.seeds),
                                        render_seeds(&new_pda.seeds),
                                    ))
                                    .old(render_seeds(&old_pda.seeds))
                                    .new_value(render_seeds(&new_pda.seeds))
                                    .suggestion(
                                        "Restore the original seeds, or write a migration \
                                         instruction that reads accounts at the old PDA and \
                                         writes them to the new one. There is no safe \
                                         acknowledgement — existing PDA accounts at the old \
                                         address become permanently orphaned otherwise.",
                                    ),
                            );
                        }
                    }
                }
            }
        }
        findings
    }
}

fn pdas_equal(a: &PdaSpec, b: &PdaSpec) -> bool {
    if a.program_id != b.program_id {
        return false;
    }
    if a.seeds.len() != b.seeds.len() {
        return false;
    }
    a.seeds.iter().zip(b.seeds.iter()).all(|(x, y)| match (x, y) {
        (Seed::Const { bytes: b1 }, Seed::Const { bytes: b2 }) => b1 == b2,
        (Seed::Arg { name: n1 }, Seed::Arg { name: n2 }) => n1 == n2,
        (
            Seed::Account {
                name: n1,
                field: f1,
            },
            Seed::Account {
                name: n2,
                field: f2,
            },
        ) => n1 == n2 && f1 == f2,
        (Seed::Unknown { raw: r1 }, Seed::Unknown { raw: r2 }) => r1 == r2,
        _ => false,
    })
}

fn render_seeds(seeds: &[Seed]) -> String {
    seeds
        .iter()
        .map(render_seed)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_seed(seed: &Seed) -> String {
    match seed {
        Seed::Const { bytes } => {
            if let Ok(s) = std::str::from_utf8(bytes) {
                if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                    return format!("b\"{s}\"");
                }
            }
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("0x{hex}")
        }
        Seed::Arg { name } => format!("arg:{name}"),
        Seed::Account { name, field } => match field {
            Some(f) => format!("account:{name}.{f}"),
            None => format!("account:{name}"),
        },
        Seed::Unknown { raw } => format!("?:{raw}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::{AccountInput, InstructionDef, PdaSpec, Seed};

    fn pda(seeds: Vec<Seed>) -> PdaSpec {
        PdaSpec {
            seeds,
            program_id: None,
        }
    }

    fn acc_with_pda(name: &str, pda: Option<PdaSpec>) -> AccountInput {
        AccountInput {
            name: name.into(),
            is_signer: false,
            is_writable: false,
            is_optional: false,
            pda,
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
    fn identical_seeds_no_finding() {
        let s = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![
                Seed::Const {
                    bytes: b"vault".to_vec(),
                },
                Seed::Account {
                    name: "user".into(),
                    field: None,
                },
            ])),
        )]));
        assert!(PdaSeedChange.check(&s, &s, &CheckContext::new()).is_empty());
    }

    #[test]
    fn changed_const_seed_is_breaking() {
        let old = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![Seed::Const {
                bytes: b"vault".to_vec(),
            }])),
        )]));
        let new = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![Seed::Const {
                bytes: b"safe".to_vec(),
            }])),
        )]));
        let findings = PdaSeedChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Breaking);
        assert!(findings[0].allow_flag.is_none());
        assert!(findings[0].old.as_deref().unwrap().contains("vault"));
        assert!(findings[0].new.as_deref().unwrap().contains("safe"));
    }

    #[test]
    fn reordered_seeds_are_breaking() {
        let a_before_b = vec![
            Seed::Const {
                bytes: b"a".to_vec(),
            },
            Seed::Const {
                bytes: b"b".to_vec(),
            },
        ];
        let b_before_a = vec![
            Seed::Const {
                bytes: b"b".to_vec(),
            },
            Seed::Const {
                bytes: b"a".to_vec(),
            },
        ];
        let old = surface(ix(vec![acc_with_pda("vault", Some(pda(a_before_b)))]));
        let new = surface(ix(vec![acc_with_pda("vault", Some(pda(b_before_a)))]));
        let findings = PdaSeedChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn added_seed_component_is_breaking() {
        let old = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![Seed::Const {
                bytes: b"vault".to_vec(),
            }])),
        )]));
        let new = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![
                Seed::Const {
                    bytes: b"vault".to_vec(),
                },
                Seed::Account {
                    name: "user".into(),
                    field: None,
                },
            ])),
        )]));
        let findings = PdaSeedChange.check(&old, &new, &CheckContext::new());
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn missing_pda_info_on_either_side_is_silently_skipped() {
        let old = surface(ix(vec![acc_with_pda(
            "vault",
            Some(pda(vec![Seed::Const {
                bytes: b"vault".to_vec(),
            }])),
        )]));
        let new = surface(ix(vec![acc_with_pda("vault", None)]));
        // Pre-0.30 IDLs strip seed info; we can't reliably compare so we
        // stay silent rather than produce false positives.
        assert!(PdaSeedChange
            .check(&old, &new, &CheckContext::new())
            .is_empty());
    }

    #[test]
    fn non_pda_accounts_are_untouched() {
        let s = surface(ix(vec![acc_with_pda("vault", None)]));
        assert!(PdaSeedChange.check(&s, &s, &CheckContext::new()).is_empty());
    }
}
