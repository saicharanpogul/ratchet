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

/// `EM_BPF` — the BPF machine type Solana programs ship with. The SBPF
/// version (v0 through v3) is encoded in `e_flags`, not in a separate
/// `e_machine` value.
pub const EM_BPF: u16 = 0xf7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbfProgramInfo {
    pub size_bytes: usize,
    /// Always `EM_BPF` for valid Solana programs.
    pub machine: u16,
    /// Raw ELF `e_flags` field (bytes 48..52 in the ELF64 header).
    /// Solana encodes SBPF version information here; see
    /// [`sbpf_version_hint`] for a best-effort interpretation.
    pub e_flags: u32,
    pub elf_class_64: bool,
    pub little_endian: bool,
    pub is_shared_object: bool,
}

/// Best-effort interpretation of `e_flags` for Solana SBPF binaries.
/// Returns a stable string tag (`"sbpf-v0"` through `"sbpf-v3"`) when
/// the flag matches a known Solana encoding, or `"unknown"` otherwise.
/// Informational only — downstream tooling should not depend on
/// recognising specific versions.
pub fn sbpf_version_hint(e_flags: u32) -> &'static str {
    // Encoded by Solana's agave/sbpf toolchains as small integer tags.
    // Values checked against the shipping loaders; extend as new
    // versions emerge.
    match e_flags {
        0x00 => "sbpf-v0",
        0x01 => "sbpf-v1",
        0x20 => "sbpf-v2",
        0x30 => "sbpf-v3",
        _ => "unknown",
    }
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
    let e_flags = u32::from_le_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]);

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
    if machine != EM_BPF {
        bail!(
            "unexpected machine type e_machine={machine:#x}; expected EM_BPF ({EM_BPF:#x}). \
             Solana programs all ship with e_machine=EM_BPF; SBPF version is encoded in e_flags."
        );
    }

    Ok(SbfProgramInfo {
        size_bytes: bytes.len(),
        machine,
        e_flags,
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

    fn synth_header(machine: u16, class: u8, data: u8, e_type: u16, e_flags: u32) -> Vec<u8> {
        let mut out = vec![0u8; 64];
        out[0..4].copy_from_slice(&ELF_MAGIC);
        out[4] = class;
        out[5] = data;
        out[16..18].copy_from_slice(&e_type.to_le_bytes());
        out[18..20].copy_from_slice(&machine.to_le_bytes());
        out[48..52].copy_from_slice(&e_flags.to_le_bytes());
        out
    }

    #[test]
    fn valid_sbf_header_parses() {
        let hdr = synth_header(EM_BPF, 2, 1, 3, 0);
        let info = verify_sbf_program(&hdr).unwrap();
        assert_eq!(info.machine, EM_BPF);
        assert_eq!(info.e_flags, 0);
        assert!(info.elf_class_64 && info.little_endian && info.is_shared_object);
    }

    #[test]
    fn sbpf_version_flag_is_surfaced() {
        let hdr = synth_header(EM_BPF, 2, 1, 3, 0x20);
        let info = verify_sbf_program(&hdr).unwrap();
        assert_eq!(info.e_flags, 0x20);
        assert_eq!(sbpf_version_hint(info.e_flags), "sbpf-v2");
    }

    #[test]
    fn unknown_sbpf_version_tag_reports_unknown() {
        assert_eq!(sbpf_version_hint(0xDEADBEEF), "unknown");
    }

    #[test]
    fn non_elf_magic_rejected() {
        let mut hdr = synth_header(EM_BPF, 2, 1, 3, 0);
        hdr[0] = b'M';
        assert!(verify_sbf_program(&hdr).is_err());
    }

    #[test]
    fn short_buffer_rejected() {
        assert!(verify_sbf_program(&[0u8; 10]).is_err());
    }

    #[test]
    fn non_shared_object_rejected() {
        let hdr = synth_header(EM_BPF, 2, 1, 2 /* ET_EXEC */, 0);
        let err = verify_sbf_program(&hdr).unwrap_err();
        assert!(format!("{err}").contains("shared-object"));
    }

    #[test]
    fn non_bpf_machine_rejected() {
        let hdr = synth_header(0x3e /* x86_64 */, 2, 1, 3, 0);
        let err = verify_sbf_program(&hdr).unwrap_err();
        assert!(format!("{err}").contains("machine"));
    }

    #[test]
    fn fabricated_em_sbpf_is_rejected_now() {
        // The old code accepted 0x0107 as a second "machine" value;
        // real Solana binaries never use it. Reject so we don't let a
        // random ELF through on that label.
        let hdr = synth_header(0x0107, 2, 1, 3, 0);
        assert!(verify_sbf_program(&hdr).is_err());
    }

    #[test]
    fn big_endian_rejected() {
        let hdr = synth_header(EM_BPF, 2, 2 /* big-endian */, 3, 0);
        assert!(verify_sbf_program(&hdr).is_err());
    }

    #[test]
    fn elf32_rejected() {
        let hdr = synth_header(EM_BPF, 1 /* 32-bit */, 1, 3, 0);
        assert!(verify_sbf_program(&hdr).is_err());
    }
}
