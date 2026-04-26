//! File / string loaders for Quasar IDL JSON.
//!
//! Mirrors the shape of `ratchet_anchor::load_idl_from_file`: simple
//! `Result<QuasarIdl>` returns, no I/O abstractions, errors carry
//! enough context for a human to find the failing file.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::idl::QuasarIdl;

/// Read a Quasar IDL JSON file from disk.
///
/// Typical paths are `target/idl/<program>.json` written by
/// `quasar build`. Callers that already have the JSON in memory
/// should use [`parse_quasar_idl_str`] instead.
pub fn load_quasar_idl(path: impl AsRef<Path>) -> Result<QuasarIdl> {
    let path = path.as_ref();
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_quasar_idl_str(&text).with_context(|| format!("parsing Quasar IDL at {}", path.display()))
}

/// Parse a Quasar IDL JSON string into the typed [`QuasarIdl`] shape.
pub fn parse_quasar_idl_str(s: &str) -> Result<QuasarIdl> {
    serde_json::from_str(s).context("Quasar IDL JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        "address": "22222222222222222222222222222222222222222222",
        "metadata": { "name": "demo", "version": "0.1.0", "spec": "0.1.0" },
        "instructions": [],
        "accounts": [],
        "types": []
    }"#;

    #[test]
    fn parse_minimal_quasar_idl() {
        let idl = parse_quasar_idl_str(MINIMAL).unwrap();
        assert_eq!(idl.address.len(), 44);
        assert_eq!(idl.metadata.name, "demo");
        assert_eq!(idl.metadata.spec, "0.1.0");
        assert!(idl.instructions.is_empty());
    }

    #[test]
    fn malformed_input_is_a_parse_error() {
        assert!(parse_quasar_idl_str("not json").is_err());
    }

    #[test]
    fn missing_address_field_fails() {
        // address is required by the Quasar schema (top-level program id).
        let no_addr = r#"{ "metadata": { "name": "x", "version": "0", "spec": "0" }}"#;
        assert!(parse_quasar_idl_str(no_addr).is_err());
    }
}
