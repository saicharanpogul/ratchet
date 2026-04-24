//! Transport layer for observe's RPC needs.
//!
//! Two primitives:
//!
//! - [`signatures_within_window`] — paginated `getSignaturesForAddress`,
//!   walks until either the time window is exhausted or the `limit`
//!   guard is hit.
//! - [`fetch_transactions`] — batched `getTransaction` calls using
//!   JSON-RPC's native batch support (an array of requests in one POST
//!   body). Works on every Solana RPC that respects the spec,
//!   including Helius' fast-path endpoints.
//!
//! Deliberately small — complex back-off, retries, and concurrency live
//! in higher-level layers if they're needed. Observe's workloads are
//! "a few thousand tx fetches once in a while," not "streaming
//! firehose," so keeping the primitives synchronous + sequential keeps
//! the failure modes legible.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub use ratchet_anchor::fetch::Cluster;
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::decode::RawTransaction;
use crate::ObserveOpts;

/// Batch size honoured by every RPC we've tested — Helius caps at 100,
/// stock Solana nodes are fine at 50. Lower value covers both.
const TX_BATCH: usize = 50;

/// `getSignaturesForAddress` hard cap per call.
const SIG_BATCH: usize = 1000;

/// Typed error surface so callers can distinguish "bad input" from
/// "network failed" from "RPC returned an error object." The CLI
/// collapses all three to exit 3, but library consumers (MCP, hosted
/// server) may want finer granularity.
#[derive(Debug, Error)]
pub enum FetchError {
    #[error("HTTP transport error: {0}")]
    Http(String),
    #[error("RPC returned an error: {0}")]
    Rpc(String),
    #[error("response did not match expected shape: {0}")]
    Shape(String),
}

/// Minimal view of a `getSignaturesForAddress` response row. Public so
/// the top-level [`crate::observe`] driver can pass it between the
/// signature and transaction fetch steps.
#[derive(Debug, Clone, Deserialize)]
pub struct SignatureInfo {
    pub signature: String,
    #[serde(rename = "blockTime")]
    pub block_time: Option<i64>,
    #[serde(default)]
    pub err: Option<Value>,
    pub slot: u64,
}

/// Walk `getSignaturesForAddress` backwards from tip, returning every
/// signature whose `blockTime` falls inside the caller-requested
/// window. Stops early if `opts.limit` is reached; stops immediately
/// if an RPC page returns a signature older than the window cutoff.
pub fn signatures_within_window(
    cluster: &Cluster,
    opts: &ObserveOpts,
) -> Result<Vec<SignatureInfo>, FetchError> {
    let cutoff = now_seconds().saturating_sub(opts.window_seconds as i64);
    let mut collected = Vec::<SignatureInfo>::with_capacity(opts.limit.min(SIG_BATCH));
    let mut before: Option<String> = None;
    let mut seen = HashSet::<String>::new();

    loop {
        if collected.len() >= opts.limit {
            break;
        }
        let remaining = (opts.limit - collected.len()).min(SIG_BATCH);
        let mut params = vec![Value::String(opts.program_id.clone())];
        let mut filter = serde_json::Map::new();
        filter.insert("limit".into(), json!(remaining));
        if let Some(b) = &before {
            filter.insert("before".into(), json!(b));
        }
        params.push(Value::Object(filter));

        let response = rpc_call(cluster, "getSignaturesForAddress", Value::Array(params))?;
        let page: Vec<SignatureInfo> = serde_json::from_value(response)
            .map_err(|e| FetchError::Shape(format!("getSignaturesForAddress: {e}")))?;

        if page.is_empty() {
            break;
        }

        let last_sig = page.last().map(|s| s.signature.clone());
        let mut page_ended_outside_window = false;
        for info in page {
            if seen.contains(&info.signature) {
                continue;
            }
            if let Some(bt) = info.block_time {
                if bt < cutoff {
                    page_ended_outside_window = true;
                    continue;
                }
            }
            seen.insert(info.signature.clone());
            collected.push(info);
        }
        if page_ended_outside_window {
            break;
        }
        match last_sig {
            Some(b) => before = Some(b),
            None => break,
        }
    }

    Ok(collected)
}

/// Batch-fetch transactions for every signature, returning them in the
/// same order. Uses JSON-RPC's native batch format so we pay one round
/// trip per [`TX_BATCH`] signatures.
pub fn fetch_transactions(
    cluster: &Cluster,
    sigs: &[SignatureInfo],
) -> Result<Vec<RawTransaction>, FetchError> {
    let mut out = Vec::<RawTransaction>::with_capacity(sigs.len());
    for chunk in sigs.chunks(TX_BATCH) {
        let batch: Vec<Value> = chunk
            .iter()
            .enumerate()
            .map(|(i, s)| {
                json!({
                    "jsonrpc": "2.0",
                    "id": i,
                    "method": "getTransaction",
                    "params": [
                        s.signature,
                        {
                            "encoding": "json",
                            "maxSupportedTransactionVersion": 0,
                            "commitment": "confirmed"
                        }
                    ]
                })
            })
            .collect();

        let responses: Vec<Value> = rpc_batch(cluster, &batch)?;
        for (info, resp) in chunk.iter().zip(responses.iter()) {
            let Some(result) = resp.get("result") else {
                if let Some(err) = resp.get("error") {
                    return Err(FetchError::Rpc(err.to_string()));
                }
                continue;
            };
            if result.is_null() {
                // The tx was pruned or never confirmed at the
                // requested commitment. Skip and carry on.
                continue;
            }
            let mut tx: RawTransaction = serde_json::from_value(result.clone())
                .map_err(|e| FetchError::Shape(format!("getTransaction result: {e}")))?;
            // The top-level response omits the signature; copy from the
            // request we sent so downstream renderers can link back.
            tx.signature = info.signature.clone();
            out.push(tx);
        }
    }
    Ok(out)
}

/// Execute a single JSON-RPC call and return the `result` payload.
fn rpc_call(cluster: &Cluster, method: &str, params: Value) -> Result<Value, FetchError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let url = cluster.url();
    let resp = ureq::post(url)
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(|e| FetchError::Http(e.to_string()))?;
    let full: Value = resp
        .into_json()
        .map_err(|e| FetchError::Shape(format!("parse response: {e}")))?;
    if let Some(err) = full.get("error") {
        return Err(FetchError::Rpc(err.to_string()));
    }
    full.get("result")
        .cloned()
        .ok_or_else(|| FetchError::Shape("missing `result` field".into()))
}

fn rpc_batch(cluster: &Cluster, batch: &[Value]) -> Result<Vec<Value>, FetchError> {
    let url = cluster.url();
    let resp = ureq::post(url)
        .set("content-type", "application/json")
        .send_json(Value::Array(batch.to_vec()))
        .map_err(|e| FetchError::Http(e.to_string()))?;
    let responses: Vec<Value> = resp
        .into_json()
        .map_err(|e| FetchError::Shape(format!("parse batch response: {e}")))?;
    Ok(responses)
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
