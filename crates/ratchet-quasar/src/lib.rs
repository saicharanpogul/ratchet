//! Quasar adapter for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Quasar (<https://quasar-lang.com>) is a compile-time Solana program
//! framework. Quasar's IDL JSON shape is structurally distinct from
//! Anchor's (variable-length discriminators, untagged type union,
//! struct-only typedefs), so this crate ships:
//!
//! - [`idl::QuasarIdl`] + sub-types — the deserialisation shape that
//!   matches Quasar's `schema/src/lib.rs`.
//! - [`load_quasar_idl`] / [`parse_quasar_idl_str`] — file + string
//!   loaders for that JSON.
//! - [`normalize()`] — turns a `QuasarIdl` into ratchet's framework-
//!   agnostic [`ProgramSurface`] IR. Once normalised, every existing
//!   ratchet rule (P-rules, R-rules) runs against a Quasar surface
//!   identically to an Anchor one.
//! - [`check_pair`] / [`check_pair_readiness`] — convenience wrappers
//!   for callers that already have surfaces in hand. Run the diff
//!   (R-rules) and preflight (P-rules) catalogs respectively.
//! - [`SurfaceBuilder`] — fluent builder for assembling a surface
//!   directly from an AST without going through JSON. Useful for a
//!   future compiler-pass integration.
//! - [`detect_quasar_project`] — heuristic for tooling that needs to
//!   dispatch between the Anchor and Quasar IDL paths.
//!
//! See `docs/quasar-integration.md` in the repo root for the runtime
//! workflow (`quasar build && ratchet readiness …`) plus the roadmap
//! for matching Quasar's evolution (binary canonical schema, eventual
//! plugin API).

pub mod idl;
pub mod load;
pub mod normalize;

use std::path::{Path, PathBuf};

use ratchet_core::{
    check, default_preflight_rules, default_rules, preflight, AccountDef, CheckContext,
    Discriminator, FieldDef, InstructionDef, PreflightRule, ProgramSurface, Report, Rule,
};
use serde::{Deserialize, Serialize};

pub use idl::QuasarIdl;
pub use load::{load_quasar_idl, parse_quasar_idl_str};
pub use normalize::{normalize, normalize_str};

/// Run the diff rule set (R001–R016) against a pair of Quasar-derived
/// surfaces. Counterpart to [`check_pair_readiness`] for the preflight
/// catalog.
pub fn check_pair(old: &ProgramSurface, new: &ProgramSurface, ctx: &CheckContext) -> Report {
    let rules: Vec<Box<dyn Rule>> = default_rules();
    check(old, new, ctx, &rules)
}

/// Run the preflight rule set (P001–P006) against a single Quasar-
/// derived surface. Use before first deploy to catch the same
/// upgrade-readiness signals (`version` field, reserved padding,
/// signer coverage, name collisions) ratchet checks for Anchor —
/// the rules are framework-agnostic, this just picks the right
/// catalog.
///
/// Note: P003 / P004 (default-discriminator-pin) won't fire on
/// Quasar surfaces. Quasar devs always assign discriminators
/// explicitly (`#[instruction(discriminator = N)]`), so the "is this
/// the default Anchor sha256 prefix?" check has no semantic meaning —
/// the padded discriminator never matches, and the rule stays silent
/// by design.
pub fn check_pair_readiness(surface: &ProgramSurface, ctx: &CheckContext) -> Report {
    let rules: Vec<Box<dyn PreflightRule>> = default_preflight_rules();
    preflight(surface, ctx, &rules)
}

/// Ergonomic builder for [`ProgramSurface`]. Fluent chaining lets a
/// compiler assemble a surface in one go.
#[derive(Debug, Default, Clone)]
pub struct SurfaceBuilder {
    surface: ProgramSurface,
}

impl SurfaceBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            surface: ProgramSurface {
                name: name.into(),
                ..Default::default()
            },
        }
    }

    pub fn program_id(mut self, id: impl Into<String>) -> Self {
        self.surface.program_id = Some(id.into());
        self
    }

    pub fn version(mut self, v: impl Into<String>) -> Self {
        self.surface.version = Some(v.into());
        self
    }

    pub fn account(
        mut self,
        name: impl Into<String>,
        disc: Discriminator,
        fields: Vec<FieldDef>,
    ) -> Self {
        let name = name.into();
        self.surface.accounts.insert(
            name.clone(),
            AccountDef {
                name,
                discriminator: disc,
                fields,
                size: None,
            },
        );
        self
    }

    pub fn instruction(mut self, ix: InstructionDef) -> Self {
        self.surface.instructions.insert(ix.name.clone(), ix);
        self
    }

    pub fn build(self) -> ProgramSurface {
        self.surface
    }
}

/// Best-effort detection of a Quasar project. Looks for a `Quasar.toml`
/// or a `quasar.toml` anywhere under `root`.
pub fn detect_quasar_project(root: impl AsRef<Path>) -> bool {
    let root = root.as_ref();
    for candidate in ["Quasar.toml", "quasar.toml"] {
        if root.join(candidate).exists() {
            return true;
        }
    }
    false
}

/// Where Quasar writes its emitted IDL(s) — convention matches Anchor's
/// `target/idl/<program>.json`.
pub const QUASAR_IDL_DIR_SUFFIX: [&str; 2] = ["target", "idl"];

/// Locate the Anchor-compatible IDL that Quasar emits.
///
/// Quasar currently piggy-backs on Anchor's IDL format, so a Quasar
/// project diffs transparently with `ratchet-anchor`. This function
/// returns the default expected path so tooling doesn't have to
/// reinvent the convention.
pub fn default_idl_path(project_root: impl AsRef<Path>, program_name: &str) -> PathBuf {
    let mut p = project_root.as_ref().to_path_buf();
    for seg in QUASAR_IDL_DIR_SUFFIX {
        p.push(seg);
    }
    p.push(format!("{program_name}.json"));
    p
}

/// Forward-declared schema type that Quasar's compiler can target once
/// its native schema format stabilises. Today it carries a plain
/// [`ProgramSurface`] (since Quasar emits Anchor-compatible IDLs that
/// lower cleanly to `ProgramSurface`); when Quasar grows features the
/// IDL doesn't express, this struct gains fields and the normalizer
/// below learns about them.
///
/// The `spec` field is a version string so consumers can refuse loads
/// with incompatible layouts in the future — today only `"0.0.0"` is
/// accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuasarSchema {
    /// Schema version tag. Bumps when `surface` sub-structure or
    /// additional fields break compatibility.
    pub spec: String,
    /// Program surface in the framework-agnostic IR.
    pub surface: ProgramSurface,
}

/// Semver-ish tag the current forward-declared schema uses. Bumps when
/// additional fields are introduced.
pub const CURRENT_SCHEMA_SPEC: &str = "0.0.0";

impl QuasarSchema {
    /// Wrap a surface into a current-spec schema.
    pub fn of(surface: ProgramSurface) -> Self {
        Self {
            spec: CURRENT_SCHEMA_SPEC.into(),
            surface,
        }
    }

    /// Parse a `QuasarSchema` JSON blob. Rejects unknown spec versions
    /// with a clear error so out-of-date tooling fails loud rather than
    /// silently mis-interpreting future fields.
    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        let schema: QuasarSchema =
            serde_json::from_str(s).map_err(|e| anyhow::anyhow!("parse quasar schema: {e}"))?;
        if schema.spec != CURRENT_SCHEMA_SPEC {
            anyhow::bail!(
                "unsupported Quasar schema spec {:?}; this ratchet expects {CURRENT_SCHEMA_SPEC}",
                schema.spec
            );
        }
        Ok(schema)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratchet_core::{PrimitiveType, Severity, TypeRef};

    fn field(name: &str, ty: PrimitiveType) -> FieldDef {
        FieldDef {
            name: name.into(),
            ty: TypeRef::primitive(ty),
            offset: None,
            size: None,
        }
    }

    #[test]
    fn builder_constructs_surface() {
        let s = SurfaceBuilder::new("vault")
            .program_id("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS")
            .version("0.1.0")
            .account(
                "Vault",
                [1, 2, 3, 4, 5, 6, 7, 8],
                vec![field("balance", PrimitiveType::U64)],
            )
            .build();
        assert_eq!(s.name, "vault");
        assert_eq!(s.accounts.len(), 1);
        assert_eq!(
            s.program_id.as_deref(),
            Some("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS")
        );
    }

    #[test]
    fn check_pair_runs_default_rules() {
        let old = SurfaceBuilder::new("vault")
            .account(
                "Vault",
                [1; 8],
                vec![
                    field("a", PrimitiveType::U64),
                    field("b", PrimitiveType::U8),
                ],
            )
            .build();
        let new = SurfaceBuilder::new("vault")
            .account(
                "Vault",
                [1; 8],
                vec![
                    field("b", PrimitiveType::U8),
                    field("a", PrimitiveType::U64),
                ],
            )
            .build();
        let report = check_pair(&old, &new, &CheckContext::new());
        // R001 account-field-reorder should fire.
        assert!(report.findings.iter().any(|f| f.rule_id == "R001"));
        assert_eq!(report.max_severity(), Some(Severity::Breaking));
    }

    #[test]
    fn detect_quasar_project_returns_false_for_empty_dir() {
        let dir = std::env::temp_dir().join(format!(
            "ratchet-quasar-empty-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!detect_quasar_project(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_quasar_project_finds_marker_file() {
        let dir = std::env::temp_dir().join(format!(
            "ratchet-quasar-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Quasar.toml"), "# marker").unwrap();
        assert!(detect_quasar_project(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_idl_path_matches_anchor_convention() {
        let p = default_idl_path("/tmp/proj", "vault");
        assert_eq!(
            p,
            std::path::PathBuf::from("/tmp/proj/target/idl/vault.json")
        );
    }

    #[test]
    fn quasar_schema_round_trip() {
        let surface = SurfaceBuilder::new("vault").build();
        let schema = QuasarSchema::of(surface);
        let json = serde_json::to_string(&schema).unwrap();
        let back = QuasarSchema::from_json(&json).unwrap();
        assert_eq!(back.spec, CURRENT_SCHEMA_SPEC);
        assert_eq!(back.surface.name, "vault");
    }

    #[test]
    fn quasar_schema_rejects_future_spec() {
        let json = r#"{"spec":"9.9.9","surface":{"name":"x"}}"#;
        let err = QuasarSchema::from_json(json).unwrap_err();
        assert!(format!("{err}").contains("unsupported"));
    }
}
