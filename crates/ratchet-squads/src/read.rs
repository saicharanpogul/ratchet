//! Tiny cursor helpers for byte-exact deserialisation.
//!
//! Squads V4 uses Borsh for the top-level account layout but the
//! `VaultTransactionMessage` substructure uses a custom `SmallVec<L, T>`
//! with `u8` or `u16` length prefixes instead of Borsh's default `u32`.
//! Rather than pull in the full `squads-multisig` crate (which drags in
//! `solana-sdk`), we read the bytes by hand.

use anyhow::{bail, Result};

/// Advancing cursor over a byte buffer.
pub(crate) struct Cursor<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            bail!(
                "unexpected end of buffer at offset {}: need {n} bytes, have {}",
                self.pos,
                self.remaining()
            );
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16_le(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32_le(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64_le(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_le_bytes(arr))
    }

    pub fn pubkey(&mut self) -> Result<[u8; 32]> {
        let b = self.take(32)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(b);
        Ok(arr)
    }

    /// Borsh-style `Vec<u8>`: 4-byte little-endian length then bytes.
    pub fn vec_u8_borsh(&mut self) -> Result<Vec<u8>> {
        let len = self.u32_le()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    /// Borsh-style `Vec<Pubkey>`: 4-byte length then 32-byte entries.
    pub fn vec_pubkey_borsh(&mut self) -> Result<Vec<[u8; 32]>> {
        let len = self.u32_le()? as usize;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(self.pubkey()?);
        }
        Ok(out)
    }

    /// Squads `SmallVec<u8, u8>`: 1-byte length then bytes.
    pub fn small_vec_u8(&mut self) -> Result<Vec<u8>> {
        let len = self.u8()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    /// Squads `SmallVec<u16, u8>`: 2-byte little-endian length then bytes.
    pub fn small_vec_u16(&mut self) -> Result<Vec<u8>> {
        let len = self.u16_le()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    /// Squads `SmallVec<u8, Pubkey>`: 1-byte length then 32-byte entries.
    pub fn small_vec_pubkey_u8(&mut self) -> Result<Vec<[u8; 32]>> {
        let len = self.u8()? as usize;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(self.pubkey()?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_reads_primitives_and_tracks_position() {
        let buf = vec![1u8, 2, 0, 3, 0, 0, 0];
        let mut c = Cursor::new(&buf);
        assert_eq!(c.u8().unwrap(), 1);
        assert_eq!(c.u16_le().unwrap(), 2);
        assert_eq!(c.u32_le().unwrap(), 3);
        assert_eq!(c.pos, 7);
        assert!(c.u8().is_err());
    }

    #[test]
    fn small_vecs_use_correct_prefix() {
        let mut buf = vec![];
        buf.push(3u8); // SmallVec<u8, u8> length
        buf.extend_from_slice(&[10, 20, 30]);
        let mut c = Cursor::new(&buf);
        assert_eq!(c.small_vec_u8().unwrap(), vec![10, 20, 30]);

        let mut buf = vec![];
        buf.extend_from_slice(&(4u16).to_le_bytes()); // SmallVec<u16, u8>
        buf.extend_from_slice(&[1, 2, 3, 4]);
        let mut c = Cursor::new(&buf);
        assert_eq!(c.small_vec_u16().unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn borsh_vec_uses_u32_prefix() {
        let mut buf = vec![];
        buf.extend_from_slice(&(2u32).to_le_bytes());
        buf.extend_from_slice(&[0xAA; 32]);
        buf.extend_from_slice(&[0xBB; 32]);
        let mut c = Cursor::new(&buf);
        let pks = c.vec_pubkey_borsh().unwrap();
        assert_eq!(pks.len(), 2);
        assert_eq!(pks[0][0], 0xAA);
        assert_eq!(pks[1][0], 0xBB);
    }

    #[test]
    fn small_vec_pubkey_uses_u8_prefix() {
        let mut buf = vec![];
        buf.push(2u8);
        buf.extend_from_slice(&[0xCC; 32]);
        buf.extend_from_slice(&[0xDD; 32]);
        let mut c = Cursor::new(&buf);
        let pks = c.small_vec_pubkey_u8().unwrap();
        assert_eq!(pks.len(), 2);
        assert_eq!(pks[0][0], 0xCC);
        assert_eq!(pks[1][0], 0xDD);
    }
}
