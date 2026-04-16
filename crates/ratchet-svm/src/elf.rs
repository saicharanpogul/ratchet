//! Minimal ELF header parser for `.so` sanity-checking.
//!
//! A full LiteSVM deploy needs `solana-sdk` and a matching SBF runtime;
//! the pragmatic 80% check is verifying the bytes are at least a valid
//! Solana BPF shared-object ELF before sending the upgrade. That's what
//! this module does — no solana dependency, no compilation, just a parse
//! of the first 64 bytes of the file.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// ELF magic bytes (`0x7F 'E' 'L' 'F'`).
pub const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

/// `EM_BPF` — BPF machine type used by the original Solana loader.
pub const EM_BPF: u16 = 0xf7;

/// `EM_SBPF` — future SBPF machine identifier Solana's newer loader uses
/// in some tooling; included so we recognise both targets.
pub const EM_SBPF: u16 = 0x0107;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbfProgramInfo {
    pub size_bytes: usize,
    pub machine: u16,
    pub elf_class_64: bool,
    pub little_endian: bool,
    pub is_shared_object: bool,
}

/// Parse the ELF header of a Solana program `.so` and return the key
/// fields. Errors when the file is too short or the magic is wrong.
pub fn verify_sbf_program(bytes: &[u8]) -> Result<SbfProgramInfo> {
    if bytes.len() < 64 {
        bail!(
            "too short to be a Solana program binary: {} bytes (need at least 64)",
            bytes.len()
        );
    }
    if bytes[0..4] != ELF_MAGIC {
        bail!("not an ELF file (magic {:02x?})", &bytes[0..4]);
    }
    let elf_class_64 = bytes[4] == 2;
    let little_endian = bytes[5] == 1;
    let e_type = u16::from_le_bytes([bytes[16], bytes[17]]);
    let is_shared_object = e_type == 3; // ET_DYN
    let machine = u16::from_le_bytes([bytes[18], bytes[19]]);

    if !elf_class_64 {
        bail!(
            "ELF must be 64-bit (EI_CLASS=2); got EI_CLASS={}",
            bytes[4]
        );
    }
    if !little_endian {
        bail!(
            "ELF must be little-endian (EI_DATA=1); got EI_DATA={}",
            bytes[5]
        );
    }
    if !is_shared_object {
        bail!(
            "expected a shared-object ELF (ET_DYN=3); got e_type={e_type} — Solana programs are dynamically-linked"
        );
    }
    if machine != EM_BPF && machine != EM_SBPF {
        bail!(
            "unexpected machine type e_machine={machine:#x}; expected EM_BPF ({EM_BPF:#x}) or EM_SBPF ({EM_SBPF:#x})"
        );
    }

    Ok(SbfProgramInfo {
        size_bytes: bytes.len(),
        machine,
        elf_class_64,
        little_endian,
        is_shared_object,
    })
}

/// Read a program `.so` from disk and verify its header.
pub fn verify_sbf_program_file(path: impl AsRef<Path>) -> Result<SbfProgramInfo> {
    let path = path.as_ref();
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    verify_sbf_program(&bytes).with_context(|| format!("verifying {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_header(machine: u16, class: u8, data: u8, e_type: u16) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        out[0..4].copy_from_slice(&ELF_MAGIC);
        out[4] = class;
        out[5] = data;
        out[16..18].copy_from_slice(&e_type.to_le_bytes());
        out[18..20].copy_from_slice(&machine.to_le_bytes());
        out
    }

    #[test]
    fn valid_sbf_header_parses() {
        let hdr = synth_header(EM_BPF, 2, 1, 3);
        let info = verify_sbf_program(&hdr).unwrap();
        assert_eq!(info.machine, EM_BPF);
        assert!(info.elf_class_64 && info.little_endian && info.is_shared_object);
    }

    #[test]
    fn valid_sbpf_header_parses() {
        let hdr = synth_header(EM_SBPF, 2, 1, 3);
        assert_eq!(verify_sbf_program(&hdr).unwrap().machine, EM_SBPF);
    }

    #[test]
    fn non_elf_magic_rejected() {
        let mut hdr = synth_header(EM_BPF, 2, 1, 3);
        hdr[0] = b'M';
        assert!(verify_sbf_program(&hdr).is_err());
    }

    #[test]
    fn short_buffer_rejected() {
        assert!(verify_sbf_program(&[0u8; 10]).is_err());
    }

    #[test]
    fn non_shared_object_rejected() {
        let hdr = synth_header(EM_BPF, 2, 1, 2 /* ET_EXEC */);
        let err = verify_sbf_program(&hdr).unwrap_err();
        assert!(format!("{err}").contains("shared-object"));
    }

    #[test]
    fn non_bpf_machine_rejected() {
        let hdr = synth_header(0x3e /* x86_64 */, 2, 1, 3);
        let err = verify_sbf_program(&hdr).unwrap_err();
        assert!(format!("{err}").contains("machine"));
    }

    #[test]
    fn big_endian_rejected() {
        let hdr = synth_header(EM_BPF, 2, 2 /* big-endian */, 3);
        assert!(verify_sbf_program(&hdr).is_err());
    }

    #[test]
    fn elf32_rejected() {
        let hdr = synth_header(EM_BPF, 1 /* 32-bit */, 1, 3);
        assert!(verify_sbf_program(&hdr).is_err());
    }
}
