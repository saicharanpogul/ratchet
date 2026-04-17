//! [`SourcePatch`] — PDA seed information extracted from a project's
//! Rust source, applied on top of a ProgramSurface that was normalized
//! from an IDL.
//!
//! The IDL already describes instructions and accounts accurately; what
//! it sometimes misses is the *expression* behind a PDA seed (Anchor
//! flattens complex seed expressions into an account-path reference,
//! dropping any field access). Source parsing refines that.

use std::collections::HashMap;

use ratchet_core::{PdaSpec, ProgramSurface, Seed};
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

    /// Merge source-derived PDA specs into `surface`.
    ///
    /// Per-seed merge, not a vec-level count comparison. For each
    /// `AccountInput::pda`:
    /// - If the IDL had no PDA spec, adopt source's entirely.
    /// - If the IDL has more seeds than source, keep IDL. (Source
    ///   missed seeds the IDL captured — the IDL is more complete.)
    /// - Otherwise walk zipped seed positions and prefer whichever
    ///   side carries more structural detail per seed — source wins
    ///   when the IDL's corresponding seed is a coarser account
    ///   reference (e.g. `Seed::Account { field: None }` where source
    ///   has `Seed::Account { field: Some(_) }`).
    ///
    /// Returns the number of account slots that had PDA info added or
    /// enriched.
    pub fn apply_to(&self, surface: &mut ProgramSurface) -> usize {
        let mut applied = 0usize;
        for (ix_name, ix) in surface.instructions.iter_mut() {
            for input in ix.accounts.iter_mut() {
                let Some(src) = self.get(ix_name, &input.name) else {
                    continue;
                };
                match input.pda.take() {
                    None => {
                        input.pda = Some(src.clone());
                        applied += 1;
                    }
                    Some(current) => {
                        let (merged, changed) = merge_pda(current, src);
                        input.pda = Some(merged);
                        if changed {
                            applied += 1;
                        }
                    }
                }
            }
        }
        applied
    }
}

/// Per-seed merge of two `PdaSpec`s. Returns the merged spec plus a
/// `bool` that's `true` iff the merge changed anything relative to the
/// original `idl` side.
fn merge_pda(idl: PdaSpec, src: &PdaSpec) -> (PdaSpec, bool) {
    // If source saw more seeds than the IDL, adopt source's list
    // wholesale — source has strictly more information.
    if src.seeds.len() > idl.seeds.len() {
        let changed = idl.seeds != src.seeds || idl.program_id != src.program_id;
        return (src.clone(), changed);
    }

    // If source saw fewer seeds than IDL, IDL is authoritative.
    if src.seeds.len() < idl.seeds.len() {
        return (idl, false);
    }

    // Equal length — per-seed merge with richness-aware choice.
    let mut merged_seeds = Vec::with_capacity(idl.seeds.len());
    let mut changed = false;
    for (i, idl_seed) in idl.seeds.iter().enumerate() {
        let src_seed = &src.seeds[i];
        let chosen = pick_richer_seed(idl_seed, src_seed);
        if chosen != idl_seed {
            changed = true;
        }
        merged_seeds.push(chosen.clone());
    }

    let program_id = match (&idl.program_id, &src.program_id) {
        (None, Some(_)) => {
            changed = true;
            src.program_id.clone()
        }
        (Some(_), _) => idl.program_id.clone(),
        (None, None) => None,
    };

    (
        PdaSpec {
            seeds: merged_seeds,
            program_id,
        },
        changed,
    )
}

fn pick_richer_seed<'a>(idl: &'a Seed, src: &'a Seed) -> &'a Seed {
    use Seed::*;
    match (idl, src) {
        // Unknown is the weakest signal; prefer anything else.
        (Unknown { .. }, _) => src,
        (_, Unknown { .. }) => idl,

        // Account references: field: Some beats field: None.
        (
            Account {
                name: ni,
                field: None,
            },
            Account {
                name: ns,
                field: Some(_),
            },
        ) if ni == ns => src,

        // Everything else: trust the IDL.
        _ => idl,
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

    #[test]
    fn per_seed_merge_prefers_source_when_idl_has_none_field() {
        // IDL: account:user with no field. Source: account:user.owner.
        let idl_pda = PdaSpec {
            seeds: vec![
                seed_const(b"vault"),
                Seed::Account {
                    name: "user".into(),
                    field: None,
                },
            ],
            program_id: None,
        };
        let mut s = surface_with_ix_and_account(Some(idl_pda));

        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![
                    seed_const(b"vault"),
                    Seed::Account {
                        name: "user".into(),
                        field: Some("owner".into()),
                    },
                ],
                program_id: None,
            },
        );
        let applied = patch.apply_to(&mut s);
        assert_eq!(applied, 1);
        let merged = s.instructions["deposit"].accounts[0]
            .pda
            .as_ref()
            .unwrap();
        match &merged.seeds[1] {
            Seed::Account { field, .. } => assert_eq!(field.as_deref(), Some("owner")),
            _ => panic!("expected enriched account seed"),
        }
    }

    #[test]
    fn per_seed_merge_keeps_idl_when_idl_field_is_more_specific() {
        // IDL: account:user.owner. Source: account:user (coarser).
        let idl_pda = PdaSpec {
            seeds: vec![Seed::Account {
                name: "user".into(),
                field: Some("owner".into()),
            }],
            program_id: None,
        };
        let mut s = surface_with_ix_and_account(Some(idl_pda));

        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![Seed::Account {
                    name: "user".into(),
                    field: None,
                }],
                program_id: None,
            },
        );
        let applied = patch.apply_to(&mut s);
        assert_eq!(applied, 0);
        let seeds = &s.instructions["deposit"].accounts[0]
            .pda
            .as_ref()
            .unwrap()
            .seeds;
        match &seeds[0] {
            Seed::Account { field, .. } => assert_eq!(field.as_deref(), Some("owner")),
            _ => panic!("IDL's richer field was lost"),
        }
    }

    #[test]
    fn unknown_seeds_are_always_replaced_by_source() {
        let idl_pda = PdaSpec {
            seeds: vec![Seed::Unknown {
                raw: "whatever".into(),
            }],
            program_id: None,
        };
        let mut s = surface_with_ix_and_account(Some(idl_pda));

        let mut patch = SourcePatch::new();
        patch.insert(
            "deposit",
            "vault",
            PdaSpec {
                seeds: vec![seed_const(b"vault")],
                program_id: None,
            },
        );
        patch.apply_to(&mut s);
        let seeds = &s.instructions["deposit"].accounts[0]
            .pda
            .as_ref()
            .unwrap()
            .seeds;
        assert!(matches!(seeds[0], Seed::Const { .. }));
    }
}
