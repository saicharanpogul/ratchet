//! Decoded Squads V4 types. Mirrors the upstream
//! `squads-multisig-program` Borsh layout without pulling in
//! solana-sdk.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::read::Cursor;

/// A single compiled instruction inside a vault-transaction message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledInstruction {
    pub program_id_index: u8,
    pub account_indexes: Vec<u8>,
    pub data: Vec<u8>,
}

impl CompiledInstruction {
    pub(crate) fn read(c: &mut Cursor) -> Result<Self> {
        Ok(Self {
            program_id_index: c.u8()?,
            account_indexes: c.small_vec_u8()?,
            data: c.small_vec_u16()?,
        })
    }
}

/// Address-table lookup inside the message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAddressTableLookup {
    pub account_key: [u8; 32],
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
}

impl MessageAddressTableLookup {
    pub(crate) fn read(c: &mut Cursor) -> Result<Self> {
        Ok(Self {
            account_key: c.pubkey()?,
            writable_indexes: c.small_vec_u8()?,
            readonly_indexes: c.small_vec_u8()?,
        })
    }
}

/// The v0-style message carried by a Squads vault transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultTransactionMessage {
    pub num_signers: u8,
    pub num_writable_signers: u8,
    pub num_writable_non_signers: u8,
    pub account_keys: Vec<[u8; 32]>,
    pub instructions: Vec<CompiledInstruction>,
    pub address_table_lookups: Vec<MessageAddressTableLookup>,
}

impl VaultTransactionMessage {
    pub(crate) fn read(c: &mut Cursor) -> Result<Self> {
        // Squads V4's VaultTransactionMessage uses SmallVec<u8, T> with
        // a single-byte length prefix for its three inner vectors —
        // *not* Borsh's default u32-prefixed Vec. This matches the
        // upstream `squads_multisig_program::state::vault_transaction`.
        let num_signers = c.u8()?;
        let num_writable_signers = c.u8()?;
        let num_writable_non_signers = c.u8()?;
        let account_keys = c.small_vec_pubkey_u8()?;
        let ix_len = c.u8()? as usize;
        let mut instructions = Vec::with_capacity(ix_len);
        for _ in 0..ix_len {
            instructions.push(CompiledInstruction::read(c)?);
        }
        let atl_len = c.u8()? as usize;
        let mut address_table_lookups = Vec::with_capacity(atl_len);
        for _ in 0..atl_len {
            address_table_lookups.push(MessageAddressTableLookup::read(c)?);
        }
        Ok(Self {
            num_signers,
            num_writable_signers,
            num_writable_non_signers,
            account_keys,
            instructions,
            address_table_lookups,
        })
    }
}

/// Squads V4 `VaultTransaction` account, decoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultTransaction {
    pub account_discriminator: [u8; 8],
    pub multisig: [u8; 32],
    pub creator: [u8; 32],
    pub index: u64,
    pub bump: u8,
    pub vault_index: u8,
    pub vault_bump: u8,
    pub ephemeral_signer_bumps: Vec<u8>,
    pub message: VaultTransactionMessage,
}

impl VaultTransaction {
    /// Decode a `VaultTransaction` from raw account data. Expects the
    /// leading 8-byte Anchor discriminator.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(data);
        let disc = c.read_discriminator()?;
        let multisig = c.pubkey()?;
        let creator = c.pubkey()?;
        let index = c.u64_le()?;
        let bump = c.u8()?;
        let vault_index = c.u8()?;
        let vault_bump = c.u8()?;
        let ephemeral_signer_bumps = c.vec_u8_borsh()?;
        let message = VaultTransactionMessage::read(&mut c)?;
        Ok(Self {
            account_discriminator: disc,
            multisig,
            creator,
            index,
            bump,
            vault_index,
            vault_bump,
            ephemeral_signer_bumps,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a byte blob that matches a minimal VaultTransaction layout.
    /// One compiled ix and no address-table lookups.
    pub(super) fn synth_blob() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&[0xaa; 8]); // disc
        buf.extend_from_slice(&[0x01; 32]); // multisig
        buf.extend_from_slice(&[0x02; 32]); // creator
        buf.extend_from_slice(&7u64.to_le_bytes()); // index
        buf.push(254); // bump
        buf.push(0); // vault_index
        buf.push(255); // vault_bump
        buf.extend_from_slice(&(0u32).to_le_bytes()); // ephemeral_signer_bumps empty

        // Message:
        buf.push(1); // num_signers
        buf.push(1); // num_writable_signers
        buf.push(1); // num_writable_non_signers

        // account_keys (SmallVec<u8, Pubkey>): two pubkeys, u8 prefix
        buf.push(2u8);
        buf.extend_from_slice(&[0x11; 32]);
        buf.extend_from_slice(&[0x22; 32]);

        // instructions (SmallVec<u8, CompiledInstruction>): 1 ix, u8 prefix
        buf.push(1u8);
        buf.push(0); // program_id_index
        // account_indexes SmallVec<u8,u8>: [0]
        buf.push(1);
        buf.push(0);
        // data SmallVec<u16,u8>: 4 bytes "ABCD"
        buf.extend_from_slice(&(4u16).to_le_bytes());
        buf.extend_from_slice(b"ABCD");

        // address_table_lookups (SmallVec<u8, ...>): empty, u8 prefix
        buf.push(0u8);

        buf
    }

    #[test]
    fn decodes_minimal_vault_transaction() {
        let buf = synth_blob();
        let vt = VaultTransaction::decode(&buf).unwrap();
        assert_eq!(vt.account_discriminator, [0xaa; 8]);
        assert_eq!(vt.multisig, [0x01; 32]);
        assert_eq!(vt.creator, [0x02; 32]);
        assert_eq!(vt.index, 7);
        assert_eq!(vt.message.account_keys.len(), 2);
        assert_eq!(vt.message.instructions.len(), 1);
        assert_eq!(vt.message.instructions[0].data, b"ABCD".to_vec());
    }

    #[test]
    fn rejects_truncated_blob() {
        assert!(VaultTransaction::decode(&[0u8; 4]).is_err());
    }
}
