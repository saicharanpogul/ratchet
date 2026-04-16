//! End-to-end pipeline tests: load IDL → normalize → check → assert.

use std::collections::HashSet;
use std::path::PathBuf;

use ratchet_anchor::{load_idl_from_file, normalize};
use ratchet_core::{check, default_rules, CheckContext, Severity};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn identical_idls_are_safe() {
    let idl = load_idl_from_file(fixture("vault_v1.json")).unwrap();
    let s = normalize(&idl).unwrap();
    let report = check(&s, &s, &CheckContext::new(), &default_rules());
    assert!(
        report.max_severity().is_none(),
        "report: {:?}",
        report.findings
    );
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn breaking_upgrade_fires_multiple_rules() {
    let old = normalize(&load_idl_from_file(fixture("vault_v1.json")).unwrap()).unwrap();
    let new =
        normalize(&load_idl_from_file(fixture("vault_v2_breaking.json")).unwrap()).unwrap();
    let report = check(&old, &new, &CheckContext::new(), &default_rules());

    assert_eq!(report.max_severity(), Some(Severity::Breaking));
    assert_eq!(report.exit_code(), 1);

    let ids: HashSet<&str> = report.findings.iter().map(|f| f.rule_id.as_str()).collect();
    for expected in [
        "R001", // field reorder (owner<->balance)
        "R005", // field append (bump)
        "R006", // discriminator change
        "R007", // instruction removed (withdraw)
        "R008", // instruction arg type change (u64->u32)
        "R011", // enum variant inserted (Cross between Bid and Ask)
    ] {
        assert!(
            ids.contains(expected),
            "expected rule {expected} to fire. Got: {:?}",
            ids
        );
    }
}

#[test]
fn additive_upgrade_is_safe() {
    let old = normalize(&load_idl_from_file(fixture("vault_v1.json")).unwrap()).unwrap();
    let new =
        normalize(&load_idl_from_file(fixture("vault_v2_additive.json")).unwrap()).unwrap();
    let report = check(&old, &new, &CheckContext::new(), &default_rules());

    // Only additive findings (R012 for the enum tail append). No Breaking
    // or Unsafe anywhere — a new instruction and a tail-appended enum
    // variant are both safe changes.
    assert_eq!(report.exit_code(), 0);
    assert!(
        matches!(
            report.max_severity(),
            None | Some(Severity::Additive)
        ),
        "expected safe verdict, got {:?}",
        report.max_severity()
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.rule_id == "R012" && f.message.contains("Cross")),
        "expected R012 to record the Side::Cross append"
    );
}

#[test]
fn migration_declaration_demotes_field_append() {
    let old = normalize(&load_idl_from_file(fixture("vault_v1.json")).unwrap()).unwrap();
    let new =
        normalize(&load_idl_from_file(fixture("vault_v2_breaking.json")).unwrap()).unwrap();
    let ctx = CheckContext::new().with_migration("Vault");
    let report = check(&old, &new, &ctx, &default_rules());

    let append = report
        .findings
        .iter()
        .find(|f| f.rule_id == "R005")
        .expect("R005 should still emit, just demoted");
    assert_eq!(append.severity, Severity::Additive);
}
