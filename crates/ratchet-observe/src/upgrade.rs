//! Decoders for the BPF Loader Upgradeable program's on-chain state.
//!
//! The loader stores two accounts per deployed program:
//!
//! - **Program account** (at the program id itself). Contains a tagged
//!   reference to the ProgramData account.
//! - **ProgramData account** (at a loader-derived PDA). Contains the
//!   current slot + upgrade authority + the actual ELF bytes. We
//!   decode just the header — the ELF body is megabytes of noise from
//!   our perspective.
//!
//! The layout is an `UpgradeableLoaderState` enum serialized as:
//!
//! ```text
//!   variant_tag: u32 (little-endian)
//!   variant-specific fields
//! ```
//!
//! Variants:
//! - 0 Uninitialized
//! - 1 Buffer            { authority_address: Option<Pubkey> }
//! - 2 Program           { programdata_address: Pubkey }
//! - 3 ProgramData       { slot: u64, upgrade_authority_address: Option<Pubkey> }
//!
//! `Option<Pubkey>` encodes as a 1-byte discriminator (0 = None, 1 =
//! Some) followed by 32 bytes when Some.

use anyhow::{bail, Result};

/// Loader-owned program account: the 36-byte record at the program's
/// address. Dereferences to the ProgramData PDA where the actual ELF
/// and authority live.
#[derive(Debug, Clone)]
pub struct ProgramRecord {
    pub programdata_address: [u8; 32],
}

/// Header of the ProgramData account — everything before the ELF bytes
/// begin. Enough to answer "when was this upgraded? who can upgrade?"
/// without hauling the full binary over the wire.
#[derive(Debug, Clone)]
pub struct ProgramDataHeader {
    /// Slot at which the program was last deployed / upgraded.
    pub last_deploy_slot: u64,
    /// Upgrade authority; `None` when the program has been made
    /// immutable (the authority has been set to the all-ones sentinel
    /// or removed via `SetAuthority`).
    pub upgrade_authority: Option<[u8; 32]>,
}

pub fn parse_program_record(bytes: &[u8]) -> Result<ProgramRecord> {
    if bytes.len() < 4 + 32 {
        bail!(
            "program account too small: {} bytes, need at least 36",
            bytes.len()
        );
    }
    let tag = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if tag != 2 {
        bail!("program account tag {tag}, expected 2 (Program)");
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes[4..36]);
    Ok(ProgramRecord {
        programdata_address: addr,
    })
}

pub fn parse_program_data_header(bytes: &[u8]) -> Result<ProgramDataHeader> {
    // Minimum shape: 4-byte tag + 8-byte slot + 1-byte option discriminator.
    if bytes.len() < 13 {
        bail!(
            "programdata account too small: {} bytes, need at least 13",
            bytes.len()
        );
    }
    let tag = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if tag != 3 {
        bail!("programdata account tag {tag}, expected 3 (ProgramData)");
    }
    let slot = u64::from_le_bytes(bytes[4..12].try_into().unwrap());
    let auth = match bytes[12] {
        0 => None,
        1 => {
            if bytes.len() < 45 {
                bail!(
                    "programdata claims Some(authority) but account is only {} bytes",
                    bytes.len()
                );
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes[13..45]);
            Some(key)
        }
        other => bail!("invalid Option<Pubkey> discriminator {other}"),
    };
    Ok(ProgramDataHeader {
        last_deploy_slot: slot,
        upgrade_authority: auth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_record_parses_well_formed_bytes() {
        let mut buf = Vec::<u8>::new();
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&[7u8; 32]);
        let rec = parse_program_record(&buf).unwrap();
        assert_eq!(rec.programdata_address, [7u8; 32]);
    }

    #[test]
    fn program_record_rejects_wrong_tag() {
        let mut buf = Vec::<u8>::new();
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 32]);
        assert!(parse_program_record(&buf).is_err());
    }

    #[test]
    fn program_data_header_with_authority() {
        let mut buf = Vec::<u8>::new();
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&42u64.to_le_bytes());
        buf.push(1); // Some
        buf.extend_from_slice(&[9u8; 32]);
        let hdr = parse_program_data_header(&buf).unwrap();
        assert_eq!(hdr.last_deploy_slot, 42);
        assert_eq!(hdr.upgrade_authority, Some([9u8; 32]));
    }

    #[test]
    fn program_data_header_without_authority() {
        let mut buf = Vec::<u8>::new();
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&100u64.to_le_bytes());
        buf.push(0); // None
        let hdr = parse_program_data_header(&buf).unwrap();
        assert_eq!(hdr.last_deploy_slot, 100);
        assert_eq!(hdr.upgrade_authority, None);
    }

    #[test]
    fn program_data_header_rejects_truncated_some_case() {
        let mut buf = Vec::<u8>::new();
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&1u64.to_le_bytes());
        buf.push(1); // Some but no 32-byte key follows
        assert!(parse_program_data_header(&buf).is_err());
    }
}
