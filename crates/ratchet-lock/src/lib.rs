//! `ratchet.lock` — a committed snapshot of a program's public surface.
//!
//! Teams commit a lockfile to their repository so CI can diff a newly
//! built IDL against a known-good baseline without hitting an RPC. A
//! lockfile is just a serialized [`ProgramSurface`] wrapped in a small
//! envelope (format version, canonical JSON encoding) so it diffs well
//! in pull requests.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ratchet_core::ProgramSurface;
use serde::{Deserialize, Serialize};

/// Current lockfile schema version. Bump when the shape is
/// incompatibly changed.
pub const CURRENT_VERSION: u32 = 1;

/// Conventional filename written and read by the CLI.
pub const DEFAULT_FILENAME: &str = "ratchet.lock";

/// On-disk representation of a ratchet lockfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    pub surface: ProgramSurface,
}

impl Lockfile {
    /// Wrap a surface into a current-version lockfile.
    pub fn of(surface: ProgramSurface) -> Self {
        Self {
            version: CURRENT_VERSION,
            surface,
        }
    }

    /// Serialize to pretty JSON (stable field order via `BTreeMap`
    /// inside `ProgramSurface`, so diffs stay tight).
    pub fn to_json(&self) -> Result<String> {
        let mut json = serde_json::to_string_pretty(self).context("serializing lockfile")?;
        json.push('\n');
        Ok(json)
    }

    pub fn from_json(s: &str) -> Result<Self> {
        let lock: Self = serde_json::from_str(s).context("parsing lockfile JSON")?;
        if lock.version != CURRENT_VERSION {
            anyhow::bail!(
                "unsupported ratchet.lock version {}: this binary expects v{CURRENT_VERSION}",
                lock.version
            );
        }
        Ok(lock)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let json = self.to_json()?;
        fs::write(path, json).with_context(|| format!("writing {}", path.display()))
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        Self::from_json(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratchet_core::{AccountDef, FieldDef, PrimitiveType, ProgramSurface, TypeRef};

    fn sample_surface() -> ProgramSurface {
        let mut s = ProgramSurface {
            name: "vault".into(),
            program_id: Some("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS".into()),
            version: Some("0.1.0".into()),
            ..Default::default()
        };
        s.accounts.insert(
            "Vault".into(),
            AccountDef {
                name: "Vault".into(),
                discriminator: [1, 2, 3, 4, 5, 6, 7, 8],
                fields: vec![FieldDef {
                    name: "balance".into(),
                    ty: TypeRef::primitive(PrimitiveType::U64),
                    offset: None,
                    size: None,
                }],
                size: None,
            },
        );
        s
    }

    #[test]
    fn round_trip() {
        let lock = Lockfile::of(sample_surface());
        let json = lock.to_json().unwrap();
        let back = Lockfile::from_json(&json).unwrap();
        assert_eq!(back.version, CURRENT_VERSION);
        assert_eq!(back.surface.name, "vault");
        assert_eq!(back.surface.accounts["Vault"].discriminator, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rejects_future_version() {
        let future = r#"{ "version": 9999, "surface": { "name": "vault" } }"#;
        let err = Lockfile::from_json(future).unwrap_err();
        assert!(format!("{err}").contains("unsupported"));
    }

    #[test]
    fn write_then_read_from_disk() {
        let lock = Lockfile::of(sample_surface());
        let mut path = std::env::temp_dir();
        path.push(format!(
            "ratchet-lock-test-{}-{}.lock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        lock.write(&path).unwrap();
        let back = Lockfile::read(&path).unwrap();
        assert_eq!(back.surface.name, "vault");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pretty_json_is_stable_and_newline_terminated() {
        let lock = Lockfile::of(sample_surface());
        let json = lock.to_json().unwrap();
        assert!(json.ends_with('\n'));
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"Vault\""));
    }
}
