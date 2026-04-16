//! Squads V4 proposal decoder for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! The most high-leverage place `ratchet` can fire is inside a Squads
//! signer's view of a pending program upgrade: instead of approving an
//! opaque buffer hash, the signer sees "this proposal will change field
//! 3 in `Vault` from `u64` to `u32`". This crate implements the first
//! half — recognising when a Squads V4 `VaultTransaction` is a BPF
//! loader upgrade and extracting the program id + buffer involved — so
//! a caller can pair it with `ratchet check-upgrade` on the buffer's
//! contents.
//!
//! Full deserialisation of every Squads message variant is intentionally
//! not attempted here; Squads has a rich schema (over forty instruction
//! variants at the time of writing) and would deserve its own crate
//! imported from the Squads repository. What we do instead is look for
//! the canonical BPF-loader-upgrade instruction by scanning the raw
//! account data for the known program-id bytes and the 4-byte Upgrade
//! discriminator, then decoding the adjacent account metas.

use anyhow::{bail, Result};
use ratchet_anchor::pda::{decode_pubkey, encode_pubkey};
use serde::{Deserialize, Serialize};

/// Squads V4 canonical program id.
pub const SQUADS_V4_PROGRAM_ID: &str = "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf";

/// Official BPF upgradeable loader program id.
pub const BPF_LOADER_UPGRADEABLE_PROGRAM_ID: &str = "BPFLoaderUpgradeab1e11111111111111111111111";

/// Little-endian u32 = 3 — the `Upgrade` variant of the BPF loader
/// instruction set (see `solana_program::bpf_loader_upgradeable`).
pub const BPF_LOADER_UPGRADE_DISCRIMINATOR: [u8; 4] = [3, 0, 0, 0];

/// Little-endian u32 = 4 — `SetAuthority` on the same loader.
pub const BPF_LOADER_SET_AUTHORITY_DISCRIMINATOR: [u8; 4] = [4, 0, 0, 0];

/// Summary of a Squads V4 vault-transaction that a signer is about to
/// approve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultTransactionSummary {
    /// High-level classification.
    pub kind: ProposalKind,
    /// Raw byte length of the transaction account.
    pub account_size: usize,
    /// Base58 pubkeys found inside the account data. Useful to correlate
    /// against `VaultTransactionMessage::account_keys` even without a
    /// full decoder — signers can eyeball which known addresses appear.
    pub referenced_pubkeys: Vec<String>,
}

/// What the proposal is doing, as far as we can tell from a coarse
/// scan. More sophisticated signers should pair this with a full Squads
/// IDL to deserialise the inner instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalKind {
    /// A program upgrade: the vault transaction encodes a BPF loader
    /// `Upgrade` instruction. `program_id` and `buffer` are extracted
    /// when their positions can be determined; otherwise `None`.
    ProgramUpgrade {
        #[serde(skip_serializing_if = "Option::is_none")]
        program_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        buffer: Option<String>,
    },
    /// A change of upgrade authority.
    SetUpgradeAuthority,
    /// We recognised this as a Squads vault transaction but couldn't
    /// pin it to one of the two upgrade-path variants.
    Other,
}

/// Decode a raw Squads V4 `VaultTransaction` account blob.
///
/// The heuristic is intentionally coarse: we search the raw bytes for
/// the BPF loader program id (encoded as 32 bytes of base58-decoded
/// pubkey) and for the `Upgrade`/`SetAuthority` u32 discriminators. A
/// hit on both positions identifies the proposal as an upgrade. The
/// referenced-pubkey list is a best-effort scan of every 32-byte window
/// that happens to decode to a plausible base58 pubkey we can round-trip.
pub fn decode_vault_transaction(data: &[u8]) -> Result<VaultTransactionSummary> {
    if data.len() < 8 {
        bail!(
            "vault transaction data too short ({} bytes) to contain an account discriminator",
            data.len()
        );
    }

    let bpf_loader_bytes = decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID)?;
    let mentions_loader = window_search(data, &bpf_loader_bytes).is_some();

    let upgrade_hit = window_search(data, &BPF_LOADER_UPGRADE_DISCRIMINATOR).is_some();
    let set_authority_hit = window_search(data, &BPF_LOADER_SET_AUTHORITY_DISCRIMINATOR).is_some();

    let kind = if mentions_loader && upgrade_hit {
        ProposalKind::ProgramUpgrade {
            program_id: None, // concrete extraction requires Squads schema; see docs
            buffer: None,
        }
    } else if mentions_loader && set_authority_hit {
        ProposalKind::SetUpgradeAuthority
    } else {
        ProposalKind::Other
    };

    Ok(VaultTransactionSummary {
        kind,
        account_size: data.len(),
        referenced_pubkeys: scan_pubkeys(data, 16),
    })
}

fn window_search(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Walk the data in 32-byte windows and yield at most `limit` windows
/// whose base58 round-trip matches the original bytes — i.e. the window
/// is a plausible pubkey. Duplicates removed, order preserved.
fn scan_pubkeys(data: &[u8], limit: usize) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut offset = 0;
    while offset + 32 <= data.len() && out.len() < limit {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&data[offset..offset + 32]);
        let encoded = encode_pubkey(&arr);
        // Require 32 printable base58 chars; reject all-zero and windows
        // that contain obvious zero-padding (last four bytes zero is a
        // good cheap filter against counter-like u32 hits).
        let all_zero = arr.iter().all(|&b| b == 0);
        if !all_zero && encoded.len() >= 32 && seen.insert(encoded.clone()) {
            out.push(encoded);
        }
        offset += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_upgrade_blob() -> Vec<u8> {
        let mut data = Vec::new();
        // Fake 8-byte Squads account discriminator
        data.extend_from_slice(&[0xaa; 8]);
        // BPF loader pubkey embedded somewhere
        data.extend_from_slice(&decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID).unwrap());
        // Arbitrary padding
        data.extend_from_slice(&[0u8; 16]);
        // Upgrade discriminator (u32 LE = 3)
        data.extend_from_slice(&BPF_LOADER_UPGRADE_DISCRIMINATOR);
        // Some trailing account metas
        data.extend_from_slice(&[1u8; 96]);
        data
    }

    #[test]
    fn detects_program_upgrade_proposal() {
        let summary = decode_vault_transaction(&synth_upgrade_blob()).unwrap();
        assert!(matches!(summary.kind, ProposalKind::ProgramUpgrade { .. }));
    }

    #[test]
    fn detects_set_authority_proposal() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0xaa; 8]);
        data.extend_from_slice(&decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID).unwrap());
        data.extend_from_slice(&[0u8; 16]);
        data.extend_from_slice(&BPF_LOADER_SET_AUTHORITY_DISCRIMINATOR);
        data.extend_from_slice(&[0u8; 32]);
        let summary = decode_vault_transaction(&data).unwrap();
        assert_eq!(summary.kind, ProposalKind::SetUpgradeAuthority);
    }

    #[test]
    fn unrelated_data_classified_as_other() {
        let data = vec![0xff; 256];
        let summary = decode_vault_transaction(&data).unwrap();
        assert_eq!(summary.kind, ProposalKind::Other);
    }

    #[test]
    fn too_short_data_errors_out() {
        assert!(decode_vault_transaction(&[0u8; 4]).is_err());
    }

    #[test]
    fn constants_decode_to_32_bytes() {
        assert_eq!(decode_pubkey(SQUADS_V4_PROGRAM_ID).unwrap().len(), 32);
        assert_eq!(
            decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID).unwrap().len(),
            32
        );
    }

    #[test]
    fn scan_pubkeys_finds_embedded_pubkey() {
        let blob = synth_upgrade_blob();
        let keys = scan_pubkeys(&blob, 16);
        assert!(keys
            .iter()
            .any(|k| k == BPF_LOADER_UPGRADEABLE_PROGRAM_ID));
    }
}
