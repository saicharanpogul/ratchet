//! Match raw program account data against a [`ProgramSurface`].
//!
//! For each sampled account we:
//! 1. Read the first 8 bytes as a discriminator.
//! 2. Look up the matching [`AccountDef`] in the surface.
//! 3. Check the data length is at least the lower bound implied by the
//!    account's fixed-size fields. Variable-size fields (`Vec`, `String`)
//!    contribute their minimum-length encoding.
//! 4. If no account type matches, tally under "unknown".

use std::collections::BTreeMap;

use ratchet_core::{AccountDef, FieldDef, PrimitiveType, ProgramSurface, TypeDef, TypeRef};

use crate::fetch::ProgramAccount;
use crate::report::{AccountVerdict, ReplayReport, TypeTally};

/// Validate a batch of sampled accounts against a program surface.
pub fn validate_surface(surface: &ProgramSurface, samples: &[ProgramAccount]) -> ReplayReport {
    let mut report = ReplayReport::default();
    let mut tallies: BTreeMap<String, TypeTally> = BTreeMap::new();
    let mut by_discriminator: BTreeMap<[u8; 8], &AccountDef> = BTreeMap::new();
    for acc in surface.accounts.values() {
        by_discriminator.insert(acc.discriminator, acc);
    }

    for sample in samples {
        report.total_samples += 1;
        if sample.data.len() < 8 {
            report.verdicts.push(AccountVerdict::Malformed {
                pubkey: sample.pubkey.clone(),
                reason: "data shorter than 8-byte discriminator".into(),
            });
            continue;
        }
        let mut disc = [0u8; 8];
        disc.copy_from_slice(&sample.data[..8]);

        match by_discriminator.get(&disc) {
            None => {
                report.verdicts.push(AccountVerdict::UnknownDiscriminator {
                    pubkey: sample.pubkey.clone(),
                    discriminator: disc,
                });
                *tallies.entry("<unknown>".into()).or_default() += VerdictKind::Unknown;
            }
            Some(acc) => {
                let min_size = min_account_size(surface, acc);
                if sample.data.len() < min_size {
                    report.verdicts.push(AccountVerdict::Undersized {
                        pubkey: sample.pubkey.clone(),
                        account_type: acc.name.clone(),
                        actual: sample.data.len(),
                        expected_min: min_size,
                    });
                    *tallies.entry(acc.name.clone()).or_default() += VerdictKind::Undersized;
                } else {
                    report.verdicts.push(AccountVerdict::Ok {
                        pubkey: sample.pubkey.clone(),
                        account_type: acc.name.clone(),
                    });
                    *tallies.entry(acc.name.clone()).or_default() += VerdictKind::Ok;
                }
            }
        }
    }

    report.tallies_by_type = tallies;
    report
}

/// Lower bound on the serialized byte length of an account, including the
/// 8-byte Anchor discriminator. Returns `8` for surfaces with no fields or
/// when a field's size is not statically computable.
pub fn min_account_size(surface: &ProgramSurface, acc: &AccountDef) -> usize {
    8usize + fields_min_size(surface, &acc.fields)
}

fn fields_min_size(surface: &ProgramSurface, fields: &[FieldDef]) -> usize {
    fields.iter().map(|f| type_min_size(surface, &f.ty)).sum()
}

fn type_min_size(surface: &ProgramSurface, ty: &TypeRef) -> usize {
    match ty {
        TypeRef::Primitive { ty } => match ty {
            PrimitiveType::Bool => 1,
            PrimitiveType::U8 | PrimitiveType::I8 => 1,
            PrimitiveType::U16 | PrimitiveType::I16 => 2,
            PrimitiveType::U32 | PrimitiveType::I32 | PrimitiveType::F32 => 4,
            PrimitiveType::U64 | PrimitiveType::I64 | PrimitiveType::F64 => 8,
            PrimitiveType::U128 | PrimitiveType::I128 => 16,
            PrimitiveType::Pubkey => 32,
            // Vec<u8> / String minimum is 4 (Borsh length prefix).
            PrimitiveType::Bytes | PrimitiveType::String => 4,
        },
        TypeRef::Array { ty, len } => type_min_size(surface, ty) * len,
        // `Vec<T>` and `Option<T>` both have a minimum contribution (4-byte
        // length and 1-byte tag respectively).
        TypeRef::Vec { .. } => 4,
        TypeRef::Option { .. } => 1,
        TypeRef::Unrecognized { .. } => {
            // Unknown shape — can't bound its size. Returning 0 keeps
            // min_account_size a true lower bound: accounts whose
            // only oversized fields are Unrecognized won't be flagged
            // as undersized by mistake.
            0
        }
        TypeRef::Defined { name } => match surface.types.get(name) {
            Some(TypeDef::Struct { fields }) => fields_min_size(surface, fields),
            Some(TypeDef::Enum { variants }) => {
                // Borsh encodes enums as 1-byte tag + variant payload; take
                // the smallest payload so the minimum stays a lower bound.
                let min_variant = variants
                    .iter()
                    .map(|v| match &v.fields {
                        ratchet_core::EnumVariantFields::Unit => 0,
                        ratchet_core::EnumVariantFields::Tuple { types } => {
                            types.iter().map(|t| type_min_size(surface, t)).sum()
                        }
                        ratchet_core::EnumVariantFields::Named { fields } => {
                            fields_min_size(surface, fields)
                        }
                    })
                    .min()
                    .unwrap_or(0);
                1 + min_variant
            }
            Some(TypeDef::Alias { target }) => type_min_size(surface, target),
            None => 0, // unknown type — don't add noise to the lower bound
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum VerdictKind {
    Ok,
    Undersized,
    Unknown,
}

impl std::ops::AddAssign<VerdictKind> for TypeTally {
    fn add_assign(&mut self, rhs: VerdictKind) {
        match rhs {
            VerdictKind::Ok => self.ok += 1,
            VerdictKind::Undersized => self.undersized += 1,
            VerdictKind::Unknown => self.unknown += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratchet_core::{AccountDef, FieldDef, PrimitiveType, ProgramSurface, TypeRef};

    fn vault_surface() -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "vault".into(),
            ..Default::default()
        };
        s.accounts.insert(
            "Vault".into(),
            AccountDef {
                name: "Vault".into(),
                discriminator: [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0],
                fields: vec![
                    FieldDef {
                        name: "owner".into(),
                        ty: TypeRef::primitive(PrimitiveType::Pubkey),
                        offset: None,
                        size: None,
                    },
                    FieldDef {
                        name: "balance".into(),
                        ty: TypeRef::primitive(PrimitiveType::U64),
                        offset: None,
                        size: None,
                    },
                ],
                size: None,
            },
        );
        s
    }

    #[test]
    fn min_size_sums_fixed_fields_and_discriminator() {
        let s = vault_surface();
        let acc = &s.accounts["Vault"];
        // 8 (disc) + 32 (pubkey) + 8 (u64) = 48
        assert_eq!(min_account_size(&s, acc), 48);
    }

    #[test]
    fn ok_when_sample_matches_and_fits() {
        let s = vault_surface();
        let sample = ProgramAccount {
            pubkey: "Aa111111111111111111111111111111111111111".into(),
            data: {
                let mut v = Vec::with_capacity(48);
                v.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]);
                v.extend_from_slice(&[0u8; 40]);
                v
            },
        };
        let report = validate_surface(&s, &[sample]);
        assert_eq!(report.total_samples, 1);
        assert_eq!(report.tallies_by_type["Vault"].ok, 1);
    }

    #[test]
    fn undersized_when_data_shorter_than_min() {
        let s = vault_surface();
        let sample = ProgramAccount {
            pubkey: "Aa222222222222222222222222222222222222222".into(),
            data: {
                let mut v = Vec::with_capacity(16);
                v.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]);
                v.extend_from_slice(&[0u8; 8]);
                v
            },
        };
        let report = validate_surface(&s, &[sample]);
        assert_eq!(report.tallies_by_type["Vault"].undersized, 1);
        assert!(matches!(
            report.verdicts[0],
            AccountVerdict::Undersized { .. }
        ));
    }

    #[test]
    fn unknown_when_discriminator_matches_no_type() {
        let s = vault_surface();
        let sample = ProgramAccount {
            pubkey: "Aa333333333333333333333333333333333333333".into(),
            data: vec![9u8; 48],
        };
        let report = validate_surface(&s, &[sample]);
        assert_eq!(report.tallies_by_type["<unknown>"].unknown, 1);
    }

    #[test]
    fn malformed_when_shorter_than_discriminator() {
        let s = vault_surface();
        let sample = ProgramAccount {
            pubkey: "Aa444444444444444444444444444444444444444".into(),
            data: vec![1, 2, 3],
        };
        let report = validate_surface(&s, &[sample]);
        assert!(matches!(report.verdicts[0], AccountVerdict::Malformed { .. }));
    }
}
