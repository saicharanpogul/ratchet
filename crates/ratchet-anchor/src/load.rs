//! Load Anchor IDL JSON from the local filesystem.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::idl::AnchorIdl;

/// Parse an Anchor IDL from a JSON file on disk.
pub fn load_idl_from_file(path: impl AsRef<Path>) -> Result<AnchorIdl> {
    let path = path.as_ref();
    let content =
        fs::read_to_string(path).with_context(|| format!("reading IDL at {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("parsing IDL at {}", path.display()))
}

/// Resolve and load an Anchor IDL from a standard Anchor workspace layout.
///
/// Looks for `<workspace>/target/idl/<program_name>.json` — the path
/// `anchor build` writes IDLs to.
pub fn load_idl_from_workspace(
    workspace: impl AsRef<Path>,
    program_name: &str,
) -> Result<AnchorIdl> {
    let path: PathBuf = workspace
        .as_ref()
        .join("target")
        .join("idl")
        .join(format!("{program_name}.json"));
    load_idl_from_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const MINIMAL_IDL: &str = r#"{
        "metadata": { "name": "tiny" },
        "instructions": [],
        "accounts": []
    }"#;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "ratchet-anchor-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn loads_idl_from_file() {
        let path = temp_path("idl.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(MINIMAL_IDL.as_bytes()).unwrap();

        let idl = load_idl_from_file(&path).unwrap();
        assert_eq!(idl.metadata.unwrap().name, "tiny");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn file_not_found_is_contextualized() {
        let err = load_idl_from_file("/does/not/exist/ratchet.json").unwrap_err();
        let display = format!("{err:#}");
        assert!(display.contains("/does/not/exist/ratchet.json"));
    }

    #[test]
    fn workspace_layout_resolved() {
        let root = temp_path("ws");
        let idl_dir = root.join("target").join("idl");
        fs::create_dir_all(&idl_dir).unwrap();
        fs::write(idl_dir.join("tiny.json"), MINIMAL_IDL).unwrap();

        let idl = load_idl_from_workspace(&root, "tiny").unwrap();
        assert_eq!(idl.metadata.unwrap().name, "tiny");

        let _ = fs::remove_dir_all(&root);
    }
}
