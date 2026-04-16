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
//! Two decoder paths are available:
//!
//! - [`decode_vault_transaction`] does a full Borsh walk of the account
//!   blob and returns a [`VaultTransactionSummary`] with concrete
//!   `program_id` and `buffer` pubkeys when the embedded instruction is
//!   a BPF loader Upgrade.
//! - [`decode_vault_transaction_fast`] is a heuristic byte-scan fallback
//!   retained for corner cases where the structured decode fails (e.g.
//!   future Squads schema revisions). It classifies the proposal kind
//!   but doesn't extract field values.

mod decoded;
mod read;

pub use decoded::{
    CompiledInstruction, MessageAddressTableLookup, VaultTransaction, VaultTransactionMessage,
};

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

/// Decode a raw Squads V4 `VaultTransaction` account.
///
/// Tries the structured Borsh decoder first and, on success, extracts
/// the program id + buffer of a BPF-loader Upgrade proposal. Falls back
/// to a coarse byte-scan ([`decode_vault_transaction_fast`]) if the
/// structured decode fails — that way future Squads schema changes
/// don't take down the whole decoder.
pub fn decode_vault_transaction(data: &[u8]) -> Result<VaultTransactionSummary> {
    if data.len() < 8 {
        bail!(
            "vault transaction data too short ({} bytes) to contain an account discriminator",
            data.len()
        );
    }

    match VaultTransaction::decode(data) {
        Ok(vt) => Ok(summarise_structured(&vt, data.len())),
        Err(_) => decode_vault_transaction_fast(data),
    }
}

/// Byte-scan heuristic fallback. Classifies the proposal kind from the
/// presence of the loader program id + the Upgrade/SetAuthority u32
/// discriminator, but cannot pin concrete pubkey field values.
pub fn decode_vault_transaction_fast(data: &[u8]) -> Result<VaultTransactionSummary> {
    if data.len() < 8 {
        bail!("too short for any Squads account: {} bytes", data.len());
    }
    let bpf_loader_bytes = decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID)?;
    let mentions_loader = window_search(data, &bpf_loader_bytes).is_some();
    let upgrade_hit = window_search(data, &BPF_LOADER_UPGRADE_DISCRIMINATOR).is_some();
    let set_authority_hit = window_search(data, &BPF_LOADER_SET_AUTHORITY_DISCRIMINATOR).is_some();

    let kind = if mentions_loader && upgrade_hit {
        ProposalKind::ProgramUpgrade {
            program_id: None,
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

fn summarise_structured(vt: &VaultTransaction, account_size: usize) -> VaultTransactionSummary {
    let loader_bytes = match decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID) {
        Ok(b) => b,
        Err(_) => return fallback_summary(vt, account_size),
    };

    let mut kind = ProposalKind::Other;
    for ix in &vt.message.instructions {
        let program_key = match vt
            .message
            .account_keys
            .get(ix.program_id_index as usize)
        {
            Some(k) => k,
            None => continue,
        };
        if program_key != &loader_bytes {
            continue;
        }
        if ix.data.starts_with(&BPF_LOADER_UPGRADE_DISCRIMINATOR) {
            kind = program_upgrade_from_ix(ix, &vt.message);
            break;
        }
        if ix.data.starts_with(&BPF_LOADER_SET_AUTHORITY_DISCRIMINATOR) {
            kind = ProposalKind::SetUpgradeAuthority;
            break;
        }
    }

    VaultTransactionSummary {
        kind,
        account_size,
        referenced_pubkeys: vt.message.account_keys.iter().map(encode_pubkey).collect(),
    }
}

fn fallback_summary(vt: &VaultTransaction, account_size: usize) -> VaultTransactionSummary {
    VaultTransactionSummary {
        kind: ProposalKind::Other,
        account_size,
        referenced_pubkeys: vt.message.account_keys.iter().map(encode_pubkey).collect(),
    }
}

/// Given the compiled Upgrade instruction and its enclosing message,
/// read the BPF-loader-upgradeable layout:
///
/// ```text
///   accounts[0] = ProgramData (the address whose bytecode gets replaced)
///   accounts[1] = Program (the program id itself)
///   accounts[2] = Buffer (new bytecode source)
///   accounts[3] = Spill (receives the buffer's lamports)
///   accounts[4] = Rent sysvar
///   accounts[5] = Clock sysvar
///   accounts[6] = Authority (must sign)
/// ```
fn program_upgrade_from_ix(
    ix: &CompiledInstruction,
    message: &VaultTransactionMessage,
) -> ProposalKind {
    let pubkey_at = |slot: usize| -> Option<String> {
        let index = *ix.account_indexes.get(slot)? as usize;
        message.account_keys.get(index).map(encode_pubkey)
    };
    let program_id = pubkey_at(1);
    let buffer = pubkey_at(2);
    ProposalKind::ProgramUpgrade { program_id, buffer }
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

    /// Build a real Borsh-formatted VaultTransaction whose inner
    /// instruction is a BPF loader Upgrade. Exercises the structured
    /// decode path end-to-end.
    fn synth_structured_upgrade_blob(
        target_program: &str,
        buffer: &str,
    ) -> (Vec<u8>, String, String) {
        let loader = decode_pubkey(BPF_LOADER_UPGRADEABLE_PROGRAM_ID).unwrap();
        let program = decode_pubkey(target_program).unwrap();
        let buf_key = decode_pubkey(buffer).unwrap();
        let spill = [0x33u8; 32];
        let rent = [0x44u8; 32];
        let clock = [0x55u8; 32];
        let authority = [0x66u8; 32];
        let program_data = [0x77u8; 32];

        let mut blob = Vec::new();
        blob.extend_from_slice(&[0xab; 8]); // disc
        blob.extend_from_slice(&[0x01; 32]); // multisig
        blob.extend_from_slice(&[0x02; 32]); // creator
        blob.extend_from_slice(&7u64.to_le_bytes());
        blob.push(254); // bump
        blob.push(0); // vault_index
        blob.push(255); // vault_bump
        blob.extend_from_slice(&(0u32).to_le_bytes()); // ephemeral_signer_bumps empty

        // Message
        blob.push(1); // num_signers
        blob.push(1); // num_writable_signers
        blob.push(1); // num_writable_non_signers

        // account_keys: loader is at index 7, followed by the Upgrade
        // instruction's 7 accounts (program_data, program, buffer, spill,
        // rent, clock, authority) in order. The compiled ix will reference
        // these by index into this array.
        let keys: Vec<&[u8; 32]> =
            vec![&program_data, &program, &buf_key, &spill, &rent, &clock, &authority, &loader];
        blob.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        for k in &keys {
            blob.extend_from_slice(*k);
        }

        // One compiled instruction: program_id_index=7 (loader), account
        // indexes 0..=6 pointing at the 7 upgrade accounts, data =
        // Upgrade discriminator.
        blob.extend_from_slice(&(1u32).to_le_bytes());
        blob.push(7); // program_id_index
        blob.push(7); // account_indexes len (SmallVec<u8,u8>)
        for i in 0u8..7 {
            blob.push(i);
        }
        blob.extend_from_slice(&(4u16).to_le_bytes()); // data len
        blob.extend_from_slice(&BPF_LOADER_UPGRADE_DISCRIMINATOR);

        // No ATLs
        blob.extend_from_slice(&(0u32).to_le_bytes());

        (blob, target_program.into(), buffer.into())
    }

    #[test]
    fn structured_decode_extracts_program_id_and_buffer() {
        let prog = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        let buffer = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
        let (blob, expected_prog, expected_buf) = synth_structured_upgrade_blob(prog, buffer);
        let summary = decode_vault_transaction(&blob).unwrap();
        match summary.kind {
            ProposalKind::ProgramUpgrade {
                program_id,
                buffer: buf,
            } => {
                assert_eq!(program_id.as_deref(), Some(expected_prog.as_str()));
                assert_eq!(buf.as_deref(), Some(expected_buf.as_str()));
            }
            other => panic!("expected ProgramUpgrade with fields, got {other:?}"),
        }
        // account_keys should surface all 8 referenced pubkeys in order.
        assert_eq!(summary.referenced_pubkeys.len(), 8);
        assert!(summary
            .referenced_pubkeys
            .iter()
            .any(|p| p == BPF_LOADER_UPGRADEABLE_PROGRAM_ID));
    }

    #[test]
    fn decode_vault_transaction_falls_back_on_bad_layout() {
        // Random blob — Borsh walk fails, heuristic kicks in.
        let summary = decode_vault_transaction(&synth_upgrade_blob()).unwrap();
        assert!(matches!(summary.kind, ProposalKind::ProgramUpgrade { .. }));
    }
}
