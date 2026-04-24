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
///
/// `pace_ms` introduces a fixed sleep between batches so sustained
/// pulls stay under paid-tier method-rate ceilings. First batch fires
/// immediately; every subsequent batch waits `pace_ms` before firing.
pub fn fetch_transactions(
    cluster: &Cluster,
    sigs: &[SignatureInfo],
    pace_ms: u64,
) -> Result<Vec<RawTransaction>, FetchError> {
    let mut out = Vec::<RawTransaction>::with_capacity(sigs.len());
    for (batch_idx, chunk) in sigs.chunks(TX_BATCH).enumerate() {
        // Pace between batches. No delay before the first one —
        // fast-return on small sample sizes still feels snappy.
        if batch_idx > 0 && pace_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(pace_ms));
        }
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

/// Fetch raw account data (base64-decoded) for `pubkey`. Returns
/// `Ok(None)` when the account does not exist, distinguishing that
/// case from network / shape errors.
pub fn fetch_account_bytes(cluster: &Cluster, pubkey: &str) -> Result<Option<Vec<u8>>, FetchError> {
    use base64::prelude::{Engine as _, BASE64_STANDARD};

    let params = json!([
        pubkey,
        { "encoding": "base64", "commitment": "confirmed" }
    ]);
    let result = rpc_call(cluster, "getAccountInfo", params)?;
    let value = result.get("value").cloned().unwrap_or(Value::Null);
    if value.is_null() {
        return Ok(None);
    }
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.as_str())
        .ok_or_else(|| FetchError::Shape("getAccountInfo.data[0] missing".into()))?;
    let bytes = BASE64_STANDARD
        .decode(data)
        .map_err(|e| FetchError::Shape(format!("base64 decode: {e}")))?;
    Ok(Some(bytes))
}

/// Resolve a slot to its wall-clock timestamp. Returns `Ok(None)` when
/// the slot has been pruned from the RPC's block history (common on
/// older slots; stock Solana RPC only keeps ~150k).
pub fn fetch_block_time(cluster: &Cluster, slot: u64) -> Result<Option<i64>, FetchError> {
    let params = json!([slot]);
    let result = rpc_call(cluster, "getBlockTime", params)?;
    Ok(result.as_i64())
}

/// Count accounts owned by `program_id` whose first 8 bytes match the
/// given discriminator. Uses `dataSlice { length: 0 }` so the RPC
/// returns pubkey-only rows — the actual account data never crosses
/// the wire, which keeps the call fast enough to run across an IDL's
/// entire account catalog.
///
/// Large programs on free RPC tiers often have `getProgramAccounts`
/// rate-limited or disabled; callers are expected to guard this
/// behind a flag and tell their users to use a paid tier if needed.
pub fn count_accounts_by_discriminator(
    cluster: &Cluster,
    program_id: &str,
    discriminator: &[u8; 8],
) -> Result<u64, FetchError> {
    let disc_b58 = bs58::encode(discriminator).into_string();
    let params = json!([
        program_id,
        {
            "encoding": "base64",
            "commitment": "confirmed",
            "dataSlice": { "offset": 0, "length": 0 },
            "filters": [
                { "memcmp": { "offset": 0, "bytes": disc_b58 } }
            ]
        }
    ]);
    let result = rpc_call(cluster, "getProgramAccounts", params)?;
    let arr = result
        .as_array()
        .ok_or_else(|| FetchError::Shape("getProgramAccounts expected array".into()))?;
    Ok(arr.len() as u64)
}

/// Maximum number of attempts per JSON-RPC call when the server
/// returns 429. Back-off sleeps between retries are 1s, 2s, 4s —
/// enough to get past a typical per-second rate-limit cap on free
/// tiers without turning a drop into a minute-long hang.
const RPC_MAX_RETRIES: u32 = 3;

/// Execute a single JSON-RPC call and return the `result` payload.
fn rpc_call(cluster: &Cluster, method: &str, params: Value) -> Result<Value, FetchError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let full = send_with_retry(cluster, &body)?;
    if let Some(err) = full.get("error") {
        return Err(FetchError::Rpc(err.to_string()));
    }
    full.get("result")
        .cloned()
        .ok_or_else(|| FetchError::Shape("missing `result` field".into()))
}

fn rpc_batch(cluster: &Cluster, batch: &[Value]) -> Result<Vec<Value>, FetchError> {
    let body = Value::Array(batch.to_vec());
    let full = send_with_retry(cluster, &body)?;
    let responses: Vec<Value> = serde_json::from_value(full)
        .map_err(|e| FetchError::Shape(format!("parse batch response: {e}")))?;
    Ok(responses)
}

/// Single POST with retry/backoff on 429. Every error string runs
/// through `redact_error_message` so API keys in URLs never leak into
/// the FetchError body (and from there into stderr / CI logs).
fn send_with_retry(cluster: &Cluster, body: &Value) -> Result<Value, FetchError> {
    let url = cluster.url();
    let mut attempt = 0u32;
    loop {
        let resp = ureq::post(url)
            .set("content-type", "application/json")
            .send_json(body.clone());
        match resp {
            Ok(r) => {
                return r.into_json::<Value>().map_err(|e| {
                    FetchError::Shape(crate::redact::redact_error_message(&format!(
                        "parse response: {e}"
                    )))
                });
            }
            Err(ureq::Error::Status(429, _)) if attempt < RPC_MAX_RETRIES => {
                let backoff = 1u64 << attempt; // 1s, 2s, 4s
                eprintln!(
                    "warn: RPC 429 from {}; retrying in {backoff}s (attempt {}/{})",
                    crate::redact::redact_rpc_url(url),
                    attempt + 1,
                    RPC_MAX_RETRIES
                );
                std::thread::sleep(std::time::Duration::from_secs(backoff));
                attempt += 1;
            }
            Err(ureq::Error::Status(429, _)) => {
                // Exhausted retries — surface a friendlier message
                // than ureq's default so the user knows *why*.
                return Err(FetchError::Http(format!(
                    "{}: rate limit exceeded after {} retries — lower --limit or switch to a paid RPC tier",
                    crate::redact::redact_rpc_url(url),
                    RPC_MAX_RETRIES
                )));
            }
            Err(e) => {
                return Err(FetchError::Http(crate::redact::redact_error_message(
                    &e.to_string(),
                )));
            }
        }
    }
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
