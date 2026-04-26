//! End-to-end integration tests against the committed Quasar IDL
//! fixtures in `examples/quasar/`.
//!
//! The fixtures are shaped to match `quasar build` output exactly
//! (verified against `blueshift-gg/quasar`'s `schema/src/lib.rs`),
//! so a passing test here means the parser + normalizer survive
//! a representative real-world IDL — not just the synthetic shapes
//! the unit tests use.

use ratchet_core::{CheckContext, Severity};
use solana_ratchet_quasar::{check_pair, check_pair_readiness, load_quasar_idl, normalize};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/quasar")
        .join(name)
}

#[test]
fn escrow_idl_parses_and_normalises() {
    let idl = load_quasar_idl(fixture("escrow.json")).expect("escrow.json must parse");
    let surface = normalize(&idl).expect("escrow.json must normalise");

    assert_eq!(surface.name, "escrow");
    assert!(surface.program_id.is_some());
    // 3 instructions, 1 account, 2 events, 1 typedef, 2 errors per fixture.
    assert_eq!(surface.instructions.len(), 3);
    assert_eq!(surface.accounts.len(), 1);
    assert_eq!(surface.events.len(), 2);
    assert_eq!(surface.errors.len(), 2);

    let make = surface
        .instructions
        .get("make")
        .expect("`make` ix should exist");
    // Quasar's discriminator [0] padded to 8 bytes.
    assert_eq!(make.discriminator, [0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(make.args.len(), 2);
    assert_eq!(make.accounts.len(), 7);
    let escrow_input = make
        .accounts
        .iter()
        .find(|a| a.name == "escrow")
        .expect("escrow account input");
    assert!(escrow_input.is_writable);
    assert!(escrow_input.pda.is_some());
}

#[test]
fn escrow_readiness_flags_expected_p_rules() {
    let idl = load_quasar_idl(fixture("escrow.json")).unwrap();
    let surface = normalize(&idl).unwrap();
    let report = check_pair_readiness(&surface, &CheckContext::new());

    let ids: Vec<&str> = report.findings.iter().map(|f| f.rule_id.as_str()).collect();
    // Bare hackathon shape: no `version` prefix, no `_reserved` padding.
    assert!(
        ids.contains(&"P001"),
        "expected P001 (missing-version-field), got {ids:?}"
    );
    assert!(
        ids.contains(&"P002"),
        "expected P002 (missing-reserved-padding), got {ids:?}"
    );
    // P003 should NOT fire — Quasar discriminators are explicitly
    // assigned (`[42]` here), and the Anchor sha256-prefix "default"
    // doesn't apply.
    assert!(
        !ids.contains(&"P003"),
        "P003 must not fire on Quasar surfaces, got {ids:?}"
    );
}

#[test]
fn escrow_v2_diff_fires_r001_r006_r007_r008() {
    let old_surface = normalize(&load_quasar_idl(fixture("escrow.json")).unwrap()).unwrap();
    let new_surface = normalize(&load_quasar_idl(fixture("escrow.v2.json")).unwrap()).unwrap();

    let report = check_pair(&old_surface, &new_surface, &CheckContext::new());
    let ids: Vec<&str> = report.findings.iter().map(|f| f.rule_id.as_str()).collect();

    // The v2 fixture is shaped to hit a quartet on a single diff:
    assert!(
        ids.contains(&"R001"),
        "expected R001 (account-field-reorder), got {ids:?}"
    );
    assert!(
        ids.contains(&"R006"),
        "expected R006 (account-discriminator-change), got {ids:?}"
    );
    assert!(
        ids.contains(&"R007"),
        "expected R007 (instruction-removed), got {ids:?}"
    );
    assert!(
        ids.contains(&"R008"),
        "expected R008 (instruction-arg-type-change), got {ids:?}"
    );

    // Verdict: BREAKING — the rules above are all severity Breaking.
    assert_eq!(
        report.max_severity(),
        Some(Severity::Breaking),
        "expected verdict BREAKING, got {:?}",
        report.max_severity()
    );
}

#[test]
fn identical_quasar_idls_produce_no_check_upgrade_findings() {
    let surface = normalize(&load_quasar_idl(fixture("escrow.json")).unwrap()).unwrap();
    let report = check_pair(&surface, &surface, &CheckContext::new());
    assert!(
        report.findings.is_empty(),
        "self-diff must be empty, got {} findings: {:?}",
        report.findings.len(),
        report
            .findings
            .iter()
            .map(|f| f.rule_id.as_str())
            .collect::<Vec<_>>()
    );
}
