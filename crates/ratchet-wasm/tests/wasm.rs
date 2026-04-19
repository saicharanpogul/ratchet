//! wasm-bindgen-test suite — runs the compiled wasm inside a real
//! Node/headless runtime via `wasm-pack test --node`. Covers the
//! unique ground the native unit tests can't: the actual
//! `check_upgrade` entry point exposed through wasm-bindgen, including
//! JS string marshalling and JsError propagation.
//!
//! Run locally:
//!
//! ```sh
//! wasm-pack test --node crates/ratchet-wasm
//! ```

#![cfg(target_arch = "wasm32")]

use solana_ratchet_wasm::{check_upgrade, version};
use wasm_bindgen_test::*;

// Default runner is Node. `wasm-pack test --node` matches the
// environment the web frontend imports the module into when the
// bundler decides to ship it client-side.

const V1: &str = r#"{
    "metadata": { "name": "vault" },
    "instructions": [
        {
            "name": "deposit",
            "discriminator": [242, 35, 198, 137, 82, 225, 242, 182],
            "accounts": [
                { "name": "user", "signer": true },
                { "name": "vault", "writable": true }
            ],
            "args": [{ "name": "amount", "type": "u64" }]
        },
        {
            "name": "withdraw",
            "discriminator": [8, 7, 6, 5, 4, 3, 2, 1],
            "accounts": [{ "name": "user" }],
            "args": []
        }
    ],
    "accounts": [
        { "name": "Vault", "discriminator": [211, 8, 232, 43, 2, 152, 117, 119] }
    ],
    "types": [
        {
            "name": "Vault",
            "type": {
                "kind": "struct",
                "fields": [
                    { "name": "owner", "type": "pubkey" },
                    { "name": "balance", "type": "u64" }
                ]
            }
        }
    ]
}"#;

const V2_BREAKING: &str = r#"{
    "metadata": { "name": "vault" },
    "instructions": [
        {
            "name": "deposit",
            "discriminator": [242, 35, 198, 137, 82, 225, 242, 182],
            "accounts": [
                { "name": "user", "signer": true },
                { "name": "vault", "writable": true }
            ],
            "args": [{ "name": "amount", "type": "u32" }]
        }
    ],
    "accounts": [
        { "name": "Vault", "discriminator": [99, 99, 99, 99, 99, 99, 99, 99] }
    ],
    "types": [
        {
            "name": "Vault",
            "type": {
                "kind": "struct",
                "fields": [
                    { "name": "balance", "type": "u64" },
                    { "name": "owner", "type": "pubkey" }
                ]
            }
        }
    ]
}"#;

#[wasm_bindgen_test]
fn version_is_crate_version() {
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
}

#[wasm_bindgen_test]
fn identical_inputs_produce_empty_report() {
    let out = check_upgrade(V1, V1).expect("check_upgrade returned Err");
    let report: serde_json::Value = serde_json::from_str(&out).expect("returned non-JSON");
    let findings = report["findings"]
        .as_array()
        .expect("missing findings array");
    assert_eq!(findings.len(), 0);
}

#[wasm_bindgen_test]
fn vault_v1_to_breaking_fires_multiple_rules() {
    let out = check_upgrade(V1, V2_BREAKING).expect("check_upgrade returned Err");
    let report: serde_json::Value = serde_json::from_str(&out).unwrap();
    let findings = report["findings"].as_array().unwrap();

    let rule_ids: Vec<&str> = findings
        .iter()
        .map(|f| f["rule_id"].as_str().unwrap())
        .collect();

    // The breaking surface: field reorder, discriminator change,
    // instruction removed (withdraw), and instruction arg type change
    // (u64 → u32). All four should fire.
    for expected in ["R001", "R006", "R007", "R008"] {
        assert!(
            rule_ids.contains(&expected),
            "expected {expected} to fire; got {rule_ids:?}"
        );
    }
}

#[wasm_bindgen_test]
fn malformed_input_surfaces_parse_error_via_js_error() {
    // The wasm-bindgen Err→JsError path. We can't pattern-match on the
    // JsError's message in Rust (the value is opaque on the Rust side),
    // but we can confirm the call fails rather than panicking.
    let err = check_upgrade("not json", V1);
    assert!(err.is_err());
}
