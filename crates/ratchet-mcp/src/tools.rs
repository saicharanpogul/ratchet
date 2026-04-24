//! Tool implementations — one dispatcher function per tool name.
//!
//! Every tool is a thin adapter: parse the JSON args, call into the
//! already-tested `ratchet_core` / `ratchet_observe` libraries, hand
//! the result back as JSON. No lint or aggregation logic lives here.
//!
//! Tool names use `kebab-case` (not `snake_case`) to match the
//! CLI subcommand convention — an agent that learned the CLI
//! recognises the tool surface immediately.

use anyhow::{Context, Result};
use ratchet_anchor::{normalize, AnchorIdl};
use ratchet_core::{
    check, default_preflight_rules, default_rules, preflight, CheckContext, Report,
};
use ratchet_observe::{Cluster, ObserveOpts, ObserveReport};
use serde::Deserialize;
use serde_json::{json, Value};

/// Metadata for `tools/list`. The JSON Schema for each tool's input
/// is inlined so agents can construct well-formed calls without a
/// separate doc fetch.
pub fn catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "readiness",
            "description": "Run the ratchet P-rule preflight on a single Anchor IDL. Pre-deploy \
                           readiness lint: missing `version: u8` prefix, reserved padding, \
                           unpinned discriminators, name collisions, unsigned writable accounts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "idl_path": {
                        "type": "string",
                        "description": "Path to an Anchor IDL JSON file. Mutually exclusive with idl_json."
                    },
                    "idl_json": {
                        "type": "string",
                        "description": "Raw Anchor IDL JSON string. Mutually exclusive with idl_path."
                    }
                },
                "oneOf": [ { "required": ["idl_path"] }, { "required": ["idl_json"] } ]
            }
        }),
        json!({
            "name": "check-upgrade",
            "description": "Diff an old vs new Anchor IDL under the ratchet R-rule engine. \
                           Catches breaking / unsafe upgrades: field reorder, discriminator \
                           change, orphaned accounts, PDA seed drift, signer tightening.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "old_path":  { "type": "string" },
                    "old_json":  { "type": "string" },
                    "new_path":  { "type": "string" },
                    "new_json":  { "type": "string" },
                    "unsafes":   { "type": "array", "items": { "type": "string" } },
                    "migrated_accounts": { "type": "array", "items": { "type": "string" } },
                    "realloc_accounts":  { "type": "array", "items": { "type": "string" } }
                }
            }
        }),
        json!({
            "name": "observe-program",
            "description": "Observe a deployed program over a time window: per-instruction success \
                           rate + error distribution, CU percentiles, recent failures with decoded \
                           account inputs. Requires a live RPC endpoint.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "program_id": { "type": "string" },
                    "cluster":    { "type": "string", "description": "mainnet, devnet, testnet, or full RPC URL." },
                    "window_seconds": { "type": "integer", "default": 86400 },
                    "limit":          { "type": "integer", "default": 1000 },
                    "include_account_counts": { "type": "boolean", "default": false },
                    "idl_path":  { "type": "string" },
                    "idl_json":  { "type": "string" }
                },
                "required": ["program_id"]
            }
        }),
        json!({
            "name": "list-rules-preflight",
            "description": "Enumerate the P-rule catalog applied by `readiness`.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "list-rules-diff",
            "description": "Enumerate the R-rule catalog applied by `check-upgrade`.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
    ]
}

/// Dispatcher. Returns the tool's result JSON to embed in the
/// MCP `tools/call` response's `content` field.
pub fn dispatch(name: &str, args: Value) -> Result<Value> {
    match name {
        "readiness" => readiness(args),
        "check-upgrade" => check_upgrade(args),
        "observe-program" => observe_program(args),
        "list-rules-preflight" => Ok(list_rules_preflight()),
        "list-rules-diff" => Ok(list_rules_diff()),
        other => anyhow::bail!("unknown tool: {other}"),
    }
}

// -----------------------------------------------------------------------------
// Individual tools
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ReadinessArgs {
    idl_path: Option<String>,
    idl_json: Option<String>,
}

fn readiness(args: Value) -> Result<Value> {
    let a: ReadinessArgs = serde_json::from_value(args).context("parsing readiness args")?;
    let idl = load_idl(a.idl_path.as_deref(), a.idl_json.as_deref())
        .context("resolving IDL for readiness")?;
    let surface = normalize(&idl).context("normalising IDL")?;
    let ctx = CheckContext::new();
    let rules = default_preflight_rules();
    let report = preflight(&surface, &ctx, &rules);
    report_to_json(&report)
}

#[derive(Debug, Deserialize)]
struct CheckUpgradeArgs {
    old_path: Option<String>,
    old_json: Option<String>,
    new_path: Option<String>,
    new_json: Option<String>,
    #[serde(default)]
    unsafes: Vec<String>,
    #[serde(default)]
    migrated_accounts: Vec<String>,
    #[serde(default)]
    realloc_accounts: Vec<String>,
}

fn check_upgrade(args: Value) -> Result<Value> {
    let a: CheckUpgradeArgs = serde_json::from_value(args).context("parsing check-upgrade args")?;
    let old_idl = load_idl(a.old_path.as_deref(), a.old_json.as_deref()).context("old IDL")?;
    let new_idl = load_idl(a.new_path.as_deref(), a.new_json.as_deref()).context("new IDL")?;
    let old_surface = normalize(&old_idl).context("normalising old IDL")?;
    let new_surface = normalize(&new_idl).context("normalising new IDL")?;
    let mut ctx = CheckContext::new();
    for f in a.unsafes {
        ctx = ctx.with_allow(f);
    }
    for n in a.migrated_accounts {
        ctx = ctx.with_migration(n);
    }
    for n in a.realloc_accounts {
        ctx = ctx.with_realloc(n);
    }
    let rules = default_rules();
    let report = check(&old_surface, &new_surface, &ctx, &rules);
    report_to_json(&report)
}

#[derive(Debug, Deserialize)]
struct ObserveArgs {
    program_id: String,
    #[serde(default = "default_cluster")]
    cluster: String,
    #[serde(default = "default_window")]
    window_seconds: u64,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    include_account_counts: bool,
    idl_path: Option<String>,
    idl_json: Option<String>,
}

fn default_cluster() -> String {
    "mainnet".into()
}
fn default_window() -> u64 {
    86_400
}
fn default_limit() -> usize {
    1000
}

fn observe_program(args: Value) -> Result<Value> {
    let a: ObserveArgs = serde_json::from_value(args).context("parsing observe-program args")?;
    let cluster = Cluster::parse(&a.cluster);
    let idl_override = match (a.idl_path.as_deref(), a.idl_json.as_deref()) {
        (Some(p), _) => Some(ratchet_anchor::load_idl_from_file(p).context("loading idl file")?),
        (None, Some(s)) => Some(serde_json::from_str(s).context("parsing idl json")?),
        (None, None) => None,
    };
    let opts = ObserveOpts {
        program_id: a.program_id,
        window_seconds: a.window_seconds,
        limit: a.limit,
        idl_override,
        include_account_counts: a.include_account_counts,
        // MCP callers stay on the library default (250ms) — the tool
        // schema doesn't expose a pacing knob yet. Agent-driven runs
        // tend to use smaller limits anyway, so the default is rarely
        // load-bearing here.
        pace_ms: ObserveOpts::default().pace_ms,
        // MCP is silent — stderr frames would corrupt the JSON-RPC
        // stream on stdout if an agent mis-read them.
        show_progress: false,
    };
    let report: ObserveReport = ratchet_observe::observe(&cluster, &opts)?;
    Ok(serde_json::to_value(&report)?)
}

fn list_rules_preflight() -> Value {
    let rules = default_preflight_rules();
    let entries: Vec<Value> = rules
        .iter()
        .map(|r| {
            json!({
                "id": r.id(),
                "name": r.name(),
                "description": r.description(),
            })
        })
        .collect();
    json!({ "rules": entries })
}

fn list_rules_diff() -> Value {
    let rules = default_rules();
    let entries: Vec<Value> = rules
        .iter()
        .map(|r| {
            json!({
                "id": r.id(),
                "name": r.name(),
                "description": r.description(),
            })
        })
        .collect();
    json!({ "rules": entries })
}

fn load_idl(path: Option<&str>, raw: Option<&str>) -> Result<AnchorIdl> {
    match (path, raw) {
        (Some(p), _) => ratchet_anchor::load_idl_from_file(p).context("reading IDL from path"),
        (None, Some(s)) => serde_json::from_str(s).context("parsing inline IDL JSON"),
        (None, None) => anyhow::bail!("either idl_path or idl_json must be provided"),
    }
}

fn report_to_json(report: &Report) -> Result<Value> {
    serde_json::to_value(report).context("serialising ratchet report")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_public_tool() {
        let names: Vec<_> = catalog()
            .iter()
            .filter_map(|v| v.get("name")?.as_str())
            .map(str::to_string)
            .collect();
        for expected in [
            "readiness",
            "check-upgrade",
            "observe-program",
            "list-rules-preflight",
            "list-rules-diff",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing tool {expected} in catalog"
            );
        }
    }

    #[test]
    fn dispatch_list_rules_returns_non_empty_catalog() {
        let v = dispatch("list-rules-preflight", json!({})).unwrap();
        let rules = v["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 6);
        let r = dispatch("list-rules-diff", json!({})).unwrap();
        assert_eq!(r["rules"].as_array().unwrap().len(), 16);
    }

    #[test]
    fn dispatch_unknown_tool_errors() {
        assert!(dispatch("not-a-tool", json!({})).is_err());
    }

    #[test]
    fn readiness_runs_against_inline_idl() {
        let idl_json = r#"{
            "metadata": { "name": "t" },
            "instructions": [],
            "accounts": [{ "name": "S", "discriminator": [1,2,3,4,5,6,7,8] }],
            "types": [
                { "name": "S", "type": { "kind": "struct",
                  "fields": [{ "name": "balance", "type": "u64" }] } }
            ]
        }"#;
        let result = dispatch("readiness", json!({ "idl_json": idl_json })).unwrap();
        let ids: Vec<&str> = result["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["rule_id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"P001"));
    }
}
