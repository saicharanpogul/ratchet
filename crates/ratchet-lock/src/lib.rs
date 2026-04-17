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
    /// Elevated from `surface.program_id` so tooling can read the lock's
    /// bound program without deserialising the full surface. Present
    /// whenever the surface had a program id at lock time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Program name at lock time. Useful to spot lockfile/target
    /// mismatches early ("this is vault.lock but you passed
    /// target/idl/treasury.json"). Optional for backward compatibility
    /// with v0 lockfiles that predate the envelope fields.
    #[serde(default)]
    pub program_name: String,
    pub surface: ProgramSurface,
}

impl Lockfile {
    /// Wrap a surface into a current-version lockfile.
    pub fn of(surface: ProgramSurface) -> Self {
        Self {
            version: CURRENT_VERSION,
            program_id: surface.program_id.clone(),
            program_name: surface.name.clone(),
            surface,
        }
    }

    /// Return `Err` when the candidate surface binds a program id that
    /// disagrees with what the lockfile captured, or whose program name
    /// differs. When either side is missing an identifier, the check is
    /// skipped — loud failure requires both sides to have named the same
    /// thing. Returns `Ok(())` on match and on missing-either-side.
    pub fn ensure_matches(&self, candidate: &ProgramSurface) -> anyhow::Result<()> {
        if !self.program_name.is_empty()
            && !candidate.name.is_empty()
            && self.program_name != candidate.name
        {
            anyhow::bail!(
                "lockfile is for program `{}`, but the candidate IDL's name is `{}`",
                self.program_name,
                candidate.name
            );
        }
        if let (Some(locked_pid), Some(candidate_pid)) =
            (self.program_id.as_deref(), candidate.program_id.as_deref())
        {
            if locked_pid != candidate_pid {
                anyhow::bail!(
                    "lockfile was captured against program id `{locked_pid}`, but the \
                     candidate IDL binds `{candidate_pid}`. If this is intentional, regenerate \
                     the lockfile with `ratchet lock`.",
                );
            }
        }
        Ok(())
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
        let content =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
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
        assert_eq!(
            back.surface.accounts["Vault"].discriminator,
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn rejects_future_version() {
        let future =
            r#"{ "version": 9999, "program_name": "vault", "surface": { "name": "vault" } }"#;
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

    #[test]
    fn envelope_surfaces_program_id_and_name() {
        let lock = Lockfile::of(sample_surface());
        assert_eq!(lock.program_name, "vault");
        assert_eq!(
            lock.program_id.as_deref(),
            Some("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS")
        );
    }

    #[test]
    fn ensure_matches_accepts_identical_identity() {
        let lock = Lockfile::of(sample_surface());
        let candidate = sample_surface();
        lock.ensure_matches(&candidate).unwrap();
    }

    #[test]
    fn ensure_matches_rejects_different_program_id() {
        let lock = Lockfile::of(sample_surface());
        let mut candidate = sample_surface();
        candidate.program_id = Some("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".into());
        let err = lock.ensure_matches(&candidate).unwrap_err();
        assert!(format!("{err}").contains("program id"));
    }

    #[test]
    fn ensure_matches_rejects_different_program_name() {
        let lock = Lockfile::of(sample_surface());
        let mut candidate = sample_surface();
        candidate.name = "treasury".into();
        let err = lock.ensure_matches(&candidate).unwrap_err();
        assert!(format!("{err}").contains("treasury"));
    }

    #[test]
    fn ensure_matches_tolerates_missing_candidate_program_id() {
        let lock = Lockfile::of(sample_surface());
        let mut candidate = sample_surface();
        candidate.program_id = None;
        lock.ensure_matches(&candidate).unwrap();
    }
}
