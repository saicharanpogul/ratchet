//! Decode an Anchor IDL account's raw account data.
//!
//! The layout written by `anchor idl init` / `anchor idl upgrade` is:
//!
//! ```text
//! [0..8)    Anchor account discriminator (for the IDL account type)
//! [8..40)   authority pubkey (32 bytes)
//! [40..44)  declared payload length (u32 little-endian)
//! [44..)    zlib-compressed IDL JSON, exactly `declared_length` bytes
//! ```
//!
//! This module doesn't know or care how the bytes were obtained — RPC,
//! snapshot, or `solana account --output json-compact` — so it can be tested
//! in isolation with synthetic bytes.

use std::io::Read;

use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;

use crate::idl::AnchorIdl;

/// Fixed prefix length in front of the compressed IDL payload.
pub const IDL_PREFIX_LEN: usize = 8 + 32 + 4;

/// Decode an Anchor IDL account blob into a parsed [`AnchorIdl`].
pub fn decode_idl_account(data: &[u8]) -> Result<AnchorIdl> {
    if data.len() < IDL_PREFIX_LEN {
        bail!(
            "IDL account is too short ({} bytes; need at least {})",
            data.len(),
            IDL_PREFIX_LEN
        );
    }

    let length = u32::from_le_bytes(
        data[40..44]
            .try_into()
            .expect("slice of length 4 is always convertible"),
    ) as usize;
    let end = IDL_PREFIX_LEN + length;
    if data.len() < end {
        bail!(
            "IDL account is truncated: header declares {} compressed bytes but only {} available",
            length,
            data.len() - IDL_PREFIX_LEN
        );
    }

    let compressed = &data[IDL_PREFIX_LEN..end];
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .context("zlib-decompressing IDL payload")?;

    serde_json::from_slice(&decompressed).context("parsing inflated IDL JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn synth_account(json: &str) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes()).unwrap();
        let payload = encoder.finish().unwrap();

        let mut out = Vec::with_capacity(IDL_PREFIX_LEN + payload.len());
        out.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef, 0x00, 0x00, 0x00, 0x00]); // disc
        out.extend_from_slice(&[0u8; 32]); // authority
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn decodes_valid_idl_account() {
        let blob = synth_account(
            r#"{ "metadata": { "name": "minimal" }, "instructions": [], "accounts": [] }"#,
        );
        let idl = decode_idl_account(&blob).unwrap();
        assert_eq!(idl.metadata.unwrap().name, "minimal");
    }

    #[test]
    fn rejects_too_short_account() {
        let err = decode_idl_account(&[0u8; 10]).unwrap_err();
        assert!(format!("{err}").contains("too short"));
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut blob = synth_account(r#"{"metadata":{"name":"x"},"instructions":[],"accounts":[]}"#);
        // Chop off some compressed bytes while leaving the declared length intact.
        blob.truncate(blob.len() - 4);
        let err = decode_idl_account(&blob).unwrap_err();
        assert!(format!("{err}").contains("truncated"));
    }

    #[test]
    fn rejects_non_zlib_payload() {
        let mut blob = vec![0u8; IDL_PREFIX_LEN + 16];
        let len: u32 = 16;
        blob[40..44].copy_from_slice(&len.to_le_bytes());
        // bytes [44..60) are left as zeros — not a valid zlib stream.
        let err = decode_idl_account(&blob).unwrap_err();
        assert!(format!("{err:#}").contains("decompressing"));
    }
}
