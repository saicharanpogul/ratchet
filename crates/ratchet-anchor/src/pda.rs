//! Program-derived address math — enough to derive Anchor's IDL account
//! from a program id without pulling in `solana-sdk`.
//!
//! All derivations boil down to two primitives:
//!
//! - [`find_program_address`]: iterate `bump` 255→0 until the resulting
//!   32-byte hash is *not* a valid Ed25519 point. That off-curve check is
//!   what makes a PDA a PDA — no corresponding private key can exist.
//! - [`create_with_seed`]: `sha256(base || seed || owner)`. Produces a
//!   deterministic child address from a known base pubkey.
//!
//! Anchor's IDL account is the composition of both:
//! `base = find_program_address(&[], program_id).0`,
//! `idl = create_with_seed(&base, "anchor:idl", program_id)`.

use anyhow::{bail, Context, Result};
use curve25519_dalek::edwards::CompressedEdwardsY;
use sha2::{Digest, Sha256};

/// Anchor's IDL-account seed marker passed to `create_with_seed`.
pub const ANCHOR_IDL_SEED: &str = "anchor:idl";

/// Fixed suffix appended by the runtime when deriving a PDA. Mirrors
/// `solana_program`'s `PDA_MARKER`.
const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";

/// Maximum length of a single seed passed to `find_program_address`. The
/// runtime rejects seeds longer than this, so we match.
pub const MAX_SEED_LEN: usize = 32;

/// Decode a base58-encoded Solana pubkey into raw bytes.
pub fn decode_pubkey(b58: &str) -> Result<[u8; 32]> {
    let bytes = bs58::decode(b58)
        .into_vec()
        .with_context(|| format!("decoding base58 pubkey `{b58}`"))?;
    if bytes.len() != 32 {
        bail!("pubkey must be 32 bytes after base58 decode, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Encode 32 raw pubkey bytes as base58.
pub fn encode_pubkey(bytes: &[u8; 32]) -> String {
    bs58::encode(bytes).into_string()
}

/// True when `bytes` is a valid Ed25519 compressed point. PDAs are
/// defined as the *off-curve* siblings of the curve — addresses that
/// cannot correspond to any private key.
pub fn is_on_curve(bytes: &[u8; 32]) -> bool {
    CompressedEdwardsY::from_slice(bytes)
        .ok()
        .and_then(|c| c.decompress())
        .is_some()
}

/// Derive a PDA exactly the way the Solana runtime does.
///
/// Returns `(address, bump)` where `bump` is the highest value in 0..=255
/// that produced an off-curve hash. Panics only if no bump in that range
/// yields an off-curve point, which is astronomically unlikely.
pub fn find_program_address(seeds: &[&[u8]], program_id: &[u8; 32]) -> ([u8; 32], u8) {
    for seed in seeds {
        assert!(
            seed.len() <= MAX_SEED_LEN,
            "seed exceeds {MAX_SEED_LEN} bytes"
        );
    }
    for bump in (0u8..=255u8).rev() {
        let mut hasher = Sha256::new();
        for seed in seeds {
            hasher.update(seed);
        }
        hasher.update([bump]);
        hasher.update(program_id);
        hasher.update(PDA_MARKER);
        let hash: [u8; 32] = hasher.finalize().into();
        if !is_on_curve(&hash) {
            return (hash, bump);
        }
    }
    panic!("no valid bump produced an off-curve PDA — should never happen");
}

/// Derive an address from a known base pubkey, a string seed, and an
/// owner program id. Equivalent to `Pubkey::create_with_seed`.
pub fn create_with_seed(base: &[u8; 32], seed: &str, owner: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(base);
    hasher.update(seed.as_bytes());
    hasher.update(owner);
    hasher.finalize().into()
}

/// Derive the Anchor IDL account address for a program id.
///
/// Two steps, exactly what `anchor idl fetch` does under the hood:
/// 1. `base = find_program_address(&[], program_id).0` — an off-curve
///    authority owned by the program.
/// 2. `idl = create_with_seed(&base, "anchor:idl", program_id)`.
pub fn anchor_idl_address(program_id: &[u8; 32]) -> [u8; 32] {
    let (base, _bump) = find_program_address(&[], program_id);
    create_with_seed(&base, ANCHOR_IDL_SEED, program_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_and_encode_round_trip() {
        let pk = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
        let bytes = decode_pubkey(pk).unwrap();
        assert_eq!(encode_pubkey(&bytes), pk);
    }

    #[test]
    fn rejects_invalid_base58_pubkey() {
        assert!(decode_pubkey("!!not-base58!!").is_err());
    }

    #[test]
    fn rejects_wrong_length_pubkey() {
        // "abcd" decodes to 3 bytes, not 32.
        assert!(decode_pubkey("abcd").is_err());
    }

    #[test]
    fn find_program_address_is_deterministic() {
        let program_id = decode_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let (addr1, bump1) = find_program_address(&[b"vault"], &program_id);
        let (addr2, bump2) = find_program_address(&[b"vault"], &program_id);
        assert_eq!(addr1, addr2);
        assert_eq!(bump1, bump2);
    }

    #[test]
    fn find_program_address_returns_off_curve() {
        let program_id = decode_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let (addr, _bump) = find_program_address(&[b"anchor", b"test"], &program_id);
        assert!(!is_on_curve(&addr), "PDA must be off-curve by definition");
    }

    #[test]
    fn is_on_curve_accepts_known_onchain_pubkey() {
        // The System Program id is a known on-curve pubkey.
        let system = decode_pubkey("11111111111111111111111111111111").unwrap();
        assert!(is_on_curve(&system));
    }

    #[test]
    fn anchor_idl_address_is_deterministic_and_distinct_from_program() {
        let program_id = decode_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let idl_a = anchor_idl_address(&program_id);
        let idl_b = anchor_idl_address(&program_id);
        assert_eq!(idl_a, idl_b);
        assert_ne!(idl_a, program_id);
    }

    #[test]
    fn different_programs_have_different_idl_addresses() {
        let a = decode_pubkey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
        let b = decode_pubkey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();
        assert_ne!(anchor_idl_address(&a), anchor_idl_address(&b));
    }

    #[test]
    fn create_with_seed_matches_known_formula() {
        // Hash-based derivation should match a manual computation.
        let base = [1u8; 32];
        let owner = [2u8; 32];
        let derived = create_with_seed(&base, "test", &owner);

        let mut hasher = Sha256::new();
        hasher.update(base);
        hasher.update(b"test");
        hasher.update(owner);
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(derived, expected);
    }
}
