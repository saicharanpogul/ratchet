//! Quasar adapter for [`ratchet`](https://github.com/saicharanpogul/ratchet).
//!
//! Quasar (https://quasar-lang.com) is a compile-time Solana program
//! framework. Running `ratchet` as a *compiler pass* inside Quasar —
//! rather than a standalone CLI invoked from CI — gives the strongest
//! possible guarantee: an incompatible upgrade refuses to compile unless
//! a migration is declared in source.
//!
//! This crate provides the surface that such a compiler pass uses:
//!
//! - [`SurfaceBuilder`] — fluent builder for assembling a
//!   [`ProgramSurface`] from an AST without manually mutating
//!   `BTreeMap`s.
//! - [`check_pair`] — the one-call convenience wrapper the Quasar pass
//!   will invoke: `check_pair(old, new, ctx)` → [`Report`].
//! - [`detect_quasar_project`] — heuristic for tooling that needs to
//!   dispatch between the Anchor and Quasar IDL loaders.
//!
//! The Quasar compiler's own source parser is deliberately out of scope
//! here — Quasar's AST is internal, and once its schema output
//! stabilises the loader can either live upstream in the Quasar
//! repository or be added to this crate.

use std::path::Path;

use ratchet_core::{
    check, AccountDef, CheckContext, Discriminator, FieldDef, InstructionDef, ProgramSurface,
    Report, Rule,
};

/// Run the default rule set against a pair of Quasar-derived surfaces.
///
/// This is the function a Quasar compiler pass invokes after it has
/// built [`ProgramSurface`] values for the old (baseline) and new
/// (current) versions of the program.
pub fn check_pair(
    old: &ProgramSurface,
    new: &ProgramSurface,
    ctx: &CheckContext,
) -> Report {
    let rules: Vec<Box<dyn Rule>> = ratchet_core::default_rules();
    check(old, new, ctx, &rules)
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

    pub fn account(mut self, name: impl Into<String>, disc: Discriminator, fields: Vec<FieldDef>) -> Self {
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
                vec![field("a", PrimitiveType::U64), field("b", PrimitiveType::U8)],
            )
            .build();
        let new = SurfaceBuilder::new("vault")
            .account(
                "Vault",
                [1; 8],
                vec![field("b", PrimitiveType::U8), field("a", PrimitiveType::U64)],
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
}
