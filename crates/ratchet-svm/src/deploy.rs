//! In-process `add_program` smoke test via LiteSVM.
//!
//! Enabled by the `litesvm` feature. The feature pulls in `litesvm` and
//! `solana-pubkey`, which in turn drag the Solana runtime crates into
//! the dep graph — worth it when a caller wants a real VM check,
//! skippable when the ELF-header sanity scan is enough.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(feature = "litesvm")]
use litesvm::LiteSVM;
#[cfg(feature = "litesvm")]
use solana_pubkey::Pubkey;

use crate::elf::{verify_sbf_program, SbfProgramInfo};
use ratchet_anchor::pda::decode_pubkey;

/// Result of the deploy smoke test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployReport {
    pub binary: SbfProgramInfo,
    pub program_id: String,
    pub deploy_succeeded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Load a program binary and `add_program` it into a fresh LiteSVM
/// instance. Returns `Ok(DeployReport)` whether or not the SVM accepted
/// the bytecode — callers check `deploy_succeeded` and `error` for
/// triage. The ELF header is verified unconditionally before the SVM
/// call so obvious corruption fails fast with a sharper error.
#[cfg(feature = "litesvm")]
pub fn verify_deploy(program_id_b58: &str, so_bytes: &[u8]) -> Result<DeployReport> {
    let binary = verify_sbf_program(so_bytes).context("verifying ELF header")?;
    let program_id_bytes = decode_pubkey(program_id_b58)?;
    let program_id = Pubkey::new_from_array(program_id_bytes);

    let mut svm = LiteSVM::new();
    // LiteSVM::add_program returns () and panics on malformed bytecode.
    // Catch the panic so callers get a structured DeployReport rather
    // than a crashed process.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        svm.add_program(program_id, so_bytes);
    }));
    let (deploy_succeeded, error) = match result {
        Ok(()) => (true, None),
        Err(payload) => {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "LiteSVM::add_program panicked (payload type unknown)".to_string()
            };
            (false, Some(msg))
        }
    };

    Ok(DeployReport {
        binary,
        program_id: program_id_b58.to_string(),
        deploy_succeeded,
        error,
    })
}

/// Compile-time-disabled stub so callers can use `verify_deploy`
/// regardless of feature state; without the feature, the result is
/// always an error pointing at the missing flag.
#[cfg(not(feature = "litesvm"))]
pub fn verify_deploy(program_id_b58: &str, so_bytes: &[u8]) -> Result<DeployReport> {
    let _ = verify_sbf_program(so_bytes).context("verifying ELF header")?;
    let _ = decode_pubkey(program_id_b58)?;
    anyhow::bail!(
        "ratchet-svm was built without the `litesvm` feature — \
         rebuild ratchet-cli with --features litesvm-deploy to enable in-process deploy tests"
    )
}

#[cfg(all(test, feature = "litesvm"))]
mod tests {
    use super::*;

    #[test]
    fn non_sbf_bytes_rejected_before_svm() {
        let err = verify_deploy("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", &[0u8; 16]).unwrap_err();
        assert!(format!("{err:#}").contains("ELF"));
    }
}

#[cfg(all(test, not(feature = "litesvm")))]
mod tests_stub {
    use super::*;

    #[test]
    fn stub_points_at_feature_flag() {
        // Pass a syntactically valid ELF (plus a valid pubkey) so the
        // stub is reached by both checks rather than short-circuiting
        // on them.
        let mut hdr = vec![0u8; 64];
        hdr[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        hdr[4] = 2;
        hdr[5] = 1;
        hdr[16..18].copy_from_slice(&3u16.to_le_bytes());
        hdr[18..20].copy_from_slice(&0xf7u16.to_le_bytes());
        let err = verify_deploy("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", &hdr).unwrap_err();
        assert!(format!("{err:#}").contains("litesvm"));
    }
}
