//! Shape of the [`ReplayReport`] returned by
//! [`validate_surface`](crate::validate::validate_surface).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Aggregate result of one replay pass.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ReplayReport {
    pub total_samples: usize,
    pub tallies_by_type: BTreeMap<String, TypeTally>,
    pub verdicts: Vec<AccountVerdict>,
}

impl ReplayReport {
    pub fn is_clean(&self) -> bool {
        self.verdicts
            .iter()
            .all(|v| matches!(v, AccountVerdict::Ok { .. }))
    }

    pub fn failing(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|v| !matches!(v, AccountVerdict::Ok { .. }))
            .count()
    }
}

/// Per-account-type accounting counts.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct TypeTally {
    pub ok: usize,
    pub undersized: usize,
    pub unknown: usize,
}

impl TypeTally {
    pub fn total(&self) -> usize {
        self.ok + self.undersized + self.unknown
    }
}

/// Per-account outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountVerdict {
    /// Discriminator matched a known account type and the data was large
    /// enough for the new minimum layout.
    Ok {
        pubkey: String,
        account_type: String,
    },
    /// Discriminator matched but the data is shorter than the new
    /// layout's minimum — the account likely hasn't been re-reallocated
    /// after a field append.
    Undersized {
        pubkey: String,
        account_type: String,
        actual: usize,
        expected_min: usize,
    },
    /// Discriminator did not match any `#[account]` in the new surface.
    /// Could be a migration-candidate (old layout), a non-Anchor account
    /// the program creates, or a spurious pda.
    UnknownDiscriminator {
        pubkey: String,
        #[serde(with = "disc_hex")]
        discriminator: [u8; 8],
    },
    /// Account data is too short to even contain a discriminator.
    Malformed { pubkey: String, reason: String },
}

mod disc_hex {
    //! Discriminator hex format matches R006 / R014 findings: `0x` prefix
    //! followed by 16 lowercase hex characters.
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &[u8; 8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(18);
        out.push_str("0x");
        for b in d {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 8], D::Error> {
        let s = String::deserialize(d)?;
        let body = s.strip_prefix("0x").unwrap_or(&s);
        if body.len() != 16 {
            return Err(serde::de::Error::custom(format!(
                "expected 16 hex chars (with optional 0x prefix), got {}",
                body.len()
            )));
        }
        let mut out = [0u8; 8];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&body[i * 2..i * 2 + 2], 16)
                .map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}
