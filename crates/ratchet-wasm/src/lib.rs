//! WebAssembly bindings for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Compiles `ratchet-core` + `ratchet-anchor` (minus the RPC fetch
//! helpers) to `wasm32-unknown-unknown` so a browser — or any JS
//! runtime — can diff two Anchor IDLs without a server round-trip.
//!
//! The surface is intentionally tiny: one function, [`check_upgrade`],
//! that takes two IDL JSON strings and returns a `Report` JSON string.
//! JS callers parse the returned string into their own typed shape —
//! keeping `serde-wasm-bindgen` off both sides of the wire keeps the
//! `.wasm` binary small and the API boring.
//!
//! Build with `wasm-pack build --target web` (see `web/package.json`'s
//! `build:wasm` script for the canonical invocation).

use ratchet_anchor::{normalize, AnchorIdl};
use ratchet_core::{check, default_rules, CheckContext};
use wasm_bindgen::prelude::*;

/// Run the default rule set against two Anchor IDL JSON strings and
/// return the resulting [`Report`](ratchet_core::Report) as a
/// serialised JSON string.
///
/// ## Errors
///
/// Returns a `JsError` when either input fails to parse as Anchor IDL,
/// when normalization fails (e.g. an unrecognized Anchor IDL type
/// shape), or — unreachably — when the report itself fails to
/// serialize.
#[wasm_bindgen]
pub fn check_upgrade(old_idl_json: &str, new_idl_json: &str) -> Result<String, JsError> {
    check_upgrade_inner(old_idl_json, new_idl_json).map_err(|e| JsError::new(&e))
}

/// Native-friendly entry point. Mirrors [`check_upgrade`] but returns
/// a plain `String` error so unit tests don't trip `JsError::new`'s
/// wasm-only machinery. Kept `pub(crate)` so it isn't exposed as part
/// of the public WASM API — JS consumers call `check_upgrade`.
pub(crate) fn check_upgrade_inner(
    old_idl_json: &str,
    new_idl_json: &str,
) -> Result<String, String> {
    let old_idl: AnchorIdl =
        serde_json::from_str(old_idl_json).map_err(|e| format!("parsing old IDL: {e}"))?;
    let new_idl: AnchorIdl =
        serde_json::from_str(new_idl_json).map_err(|e| format!("parsing new IDL: {e}"))?;

    let old_surface =
        normalize(&old_idl).map_err(|e| format!("normalizing old IDL: {e:#}"))?;
    let new_surface =
        normalize(&new_idl).map_err(|e| format!("normalizing new IDL: {e:#}"))?;

    let ctx = CheckContext::new();
    let rules = default_rules();
    let report = check(&old_surface, &new_surface, &ctx, &rules);

    serde_json::to_string(&report).map_err(|e| format!("serializing report: {e}"))
}

/// Version identifier baked into the compiled `.wasm` so the frontend
/// can display or assert which ratchet build it's running.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[cfg(test)]
mod tests {
    //! Native Rust tests — run with `cargo test -p solana-ratchet-wasm`.
    //! They exercise the same entry point the WASM build does, minus
    //! the `wasm-bindgen` indirection. Keeps correctness coverage on
    //! every `cargo test` invocation, while `wasm-bindgen-test` covers
    //! the real wasm target (see `tests/wasm.rs`).

    use super::*;

    const MINIMAL_IDL: &str = r#"{
        "metadata": { "name": "t" },
        "instructions": [],
        "accounts": [],
        "types": []
    }"#;

    #[test]
    fn identical_idls_produce_no_findings() {
        let out = check_upgrade_inner(MINIMAL_IDL, MINIMAL_IDL).unwrap();
        let report: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(report["findings"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn breaking_change_fires_expected_rule() {
        let old = r#"{
            "metadata": { "name": "t" },
            "instructions": [
                { "name": "old_ix", "discriminator": [1,2,3,4,5,6,7,8], "accounts": [], "args": [] }
            ],
            "accounts": [],
            "types": []
        }"#;
        let new = r#"{
            "metadata": { "name": "t" },
            "instructions": [],
            "accounts": [],
            "types": []
        }"#;
        let out = check_upgrade_inner(old, new).unwrap();
        let report: serde_json::Value = serde_json::from_str(&out).unwrap();
        let findings = report["findings"].as_array().unwrap();
        assert!(findings.iter().any(|f| f["rule_id"] == "R007"));
    }

    #[test]
    fn malformed_input_is_propagated_as_error() {
        let err = check_upgrade_inner("not json", "{}").unwrap_err();
        assert!(err.contains("parsing old IDL"));
    }

    #[test]
    fn version_string_matches_crate_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
