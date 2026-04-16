//! [`SourcePatch`] — PDA seed information extracted from a project's
//! Rust source, applied on top of a ProgramSurface that was normalized
//! from an IDL.
//!
//! The IDL already describes instructions and accounts accurately; what
//! it sometimes misses is the *expression* behind a PDA seed (Anchor
//! flattens complex seed expressions into an account-path reference,
//! dropping any field access). Source parsing refines that.

use std::collections::HashMap;

use ratchet_core::{PdaSpec, ProgramSurface};
use serde::{Deserialize, Serialize};

/// The key identifies a specific account slot inside a specific
/// instruction: `(ix_name, account_name)`.
pub type PdaKey = (String, String);

/// Patch of PDA information extracted from Rust source.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SourcePatch {
    /// PDA seed specs keyed by `(ix_name, account_name)`.
    pub pda_seeds: HashMap<String, PdaSpec>,
}

impl SourcePatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, ix: &str, account: &str, pda: PdaSpec) {
        self.pda_seeds.insert(key(ix, account), pda);
    }

    pub fn get(&self, ix: &str, account: &str) -> Option<&PdaSpec> {
        self.pda_seeds.get(&key(ix, account))
    }

    pub fn len(&self) -> usize {
        self.pda_seeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pda_seeds.is_empty()
    }

    /// Merge source-derived PDA specs into `surface`. Only account inputs
    /// that currently have *no* PDA info or whose IDL seeds are a coarser
    /// match (fewer seeds) are overwritten — the IDL stays authoritative
    /// when it carries richer structure.
    pub fn apply_to(&self, surface: &mut ProgramSurface) -> usize {
        let mut applied = 0usize;
        for (ix_name, ix) in surface.instructions.iter_mut() {
            for input in ix.accounts.iter_mut() {
                let Some(src) = self.get(ix_name, &input.name) else {
                    continue;
                };
                let should_overwrite = match &input.pda {
                    None => true,
                    Some(current) => src.seeds.len() > current.seeds.len(),
                };
                if should_overwrite {
                    input.pda = Some(src.clone());
                    applied += 1;
                }
            }
        }
        applied
    }
}

fn key(ix: &str, account: &str) -> String {
    format!("{ix}::{account}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratchet_core::{AccountInput, InstructionDef, ProgramSurface, Seed};

    fn surface_with_ix_and_account(pda: Option<PdaSpec>) -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "t".into(),
            ..Default::default()
        };
        s.instructions.insert(
            "deposit".into(),
            InstructionDef {
                name: "deposit".into(),
                discriminator: [0; 8],
                args: vec![],
                accounts: vec![AccountInput {
                    name: "vault".into(),
                    is_signer: false,
                    is_writable: false,
                    is_optional: false,
                    pda,
                }],
            },
        );
        s
    }

    fn seed_const(b: &[u8]) -> Seed {
        Seed::Const { bytes: b.to_vec() }
    }

    #[test]
    fn apply_fills_empty_pda() {
        let mut s = surface_with_ix_and_account(None);
        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![seed_const(b"vault")],
                program_id: None,
            },
        );
        let applied = patch.apply_to(&mut s);
        assert_eq!(applied, 1);
        assert!(s.instructions["deposit"].accounts[0].pda.is_some());
    }

    #[test]
    fn apply_overwrites_when_source_has_more_seeds() {
        let coarse = PdaSpec {
            seeds: vec![seed_const(b"vault")],
            program_id: None,
        };
        let mut s = surface_with_ix_and_account(Some(coarse));

        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![
                    seed_const(b"vault"),
                    Seed::Account {
                        name: "user".into(),
                        field: None,
                    },
                ],
                program_id: None,
            },
        );
        let applied = patch.apply_to(&mut s);
        assert_eq!(applied, 1);
        assert_eq!(
            s.instructions["deposit"].accounts[0]
                .pda
                .as_ref()
                .unwrap()
                .seeds
                .len(),
            2
        );
    }

    #[test]
    fn apply_does_not_overwrite_when_idl_is_richer() {
        let rich = PdaSpec {
            seeds: vec![
                seed_const(b"vault"),
                Seed::Account {
                    name: "user".into(),
                    field: None,
                },
            ],
            program_id: None,
        };
        let mut s = surface_with_ix_and_account(Some(rich));

        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![seed_const(b"vault")],
                program_id: None,
            },
        );
        let applied = patch.apply_to(&mut s);
        assert_eq!(applied, 0);
    }

    #[test]
    fn apply_is_noop_when_patch_is_empty() {
        let mut s = surface_with_ix_and_account(None);
        assert_eq!(SourcePatch::new().apply_to(&mut s), 0);
    }
}
