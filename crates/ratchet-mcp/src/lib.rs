//! Model Context Protocol server for ratchet.
//!
//! Exposes every ratchet capability (`readiness`, `check-upgrade`,
//! `observe`, rule catalogs) as MCP tools agents can call. Uses the
//! stdio transport spec'd at <https://modelcontextprotocol.io/> —
//! newline-delimited JSON-RPC over stdin/stdout, with protocol errors
//! reported via stderr so the on-the-wire stream stays machine-clean.
//!
//! Scope:
//! - `initialize` → we advertise `tools` capability, hand back our
//!   protocol version + server-info.
//! - `tools/list` → returns the catalog in [`tools::catalog`].
//! - `tools/call` → dispatches into [`tools::dispatch`]. Tool errors
//!   surface as `isError: true` content entries, not JSON-RPC errors,
//!   so agents can see and reason about them rather than abort the
//!   session.
//! - everything else returns `METHOD_NOT_FOUND`.

pub mod protocol;
pub mod tools;

use std::io::{BufRead, BufReader, Read, Write};

use anyhow::Result;
use protocol::{error, success, Request};
use serde_json::{json, Value};

/// Advertised MCP protocol version. Clients negotiate down to the
/// common version; this one matches Anthropic's Claude clients at the
/// time this crate shipped.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Server identity returned in the `initialize` handshake.
const SERVER_NAME: &str = "ratchet";

/// Drive the MCP server on the given stdin / stdout handles. Blocks
/// until the input stream ends (client disconnect) or an I/O error
/// surfaces. Returns `Ok(())` on a clean EOF.
pub fn run<R: Read, W: Write>(stdin: R, mut stdout: W) -> Result<()> {
    let reader = BufReader::new(stdin);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(req, &mut stdout)?,
            Err(e) => {
                // JSON-RPC 2.0 parse errors use id=null since the
                // request's id is unrecoverable at this layer.
                write_json(
                    &mut stdout,
                    &error(Value::Null, protocol::PARSE_ERROR, format!("parse: {e}")),
                )?;
            }
        }
    }
    Ok(())
}

fn handle_request<W: Write>(req: Request, stdout: &mut W) -> Result<()> {
    // Notifications (no id) never send a response. We still process
    // side-effects, but the return path is a no-op.
    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(Value::Null);

    let result = match req.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(req.params),
        "ping" => Ok(json!({})),
        "notifications/initialized" | "initialized" => {
            // Client-side acknowledgement. No reply needed.
            return Ok(());
        }
        other => Err(JsonRpcFailure::method_not_found(other)),
    };

    if is_notification {
        return Ok(());
    }

    match result {
        Ok(value) => write_json(stdout, &success(id, value)),
        Err(JsonRpcFailure {
            code,
            message,
            data: _,
        }) => write_json(stdout, &error(id, code, message)),
    }
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

fn handle_tools_list() -> Value {
    json!({ "tools": tools::catalog() })
}

fn handle_tools_call(params: Value) -> Result<Value, JsonRpcFailure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcFailure::invalid_params("tools/call: missing `name`"))?
        .to_string();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    // Tool failures surface as `isError: true` content blocks so the
    // calling agent sees them as first-class tool output — aborting
    // the JSON-RPC call would hide them from the agent's reasoning.
    match tools::dispatch(&name, args) {
        Ok(value) => Ok(json!({
            "content": [
                { "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }
            ],
            "isError": false,
            "structuredContent": value,
        })),
        Err(e) => Ok(json!({
            "content": [
                { "type": "text", "text": format!("{e:#}") }
            ],
            "isError": true,
        })),
    }
}

fn write_json<W: Write, T: serde::Serialize>(w: &mut W, v: &T) -> Result<()> {
    let mut line = serde_json::to_vec(v)?;
    line.push(b'\n');
    w.write_all(&line)?;
    w.flush()?;
    Ok(())
}

/// Lightweight local error for converting to a JSON-RPC error without
/// pulling in anyhow-everywhere — the outer run loop formats these
/// into proper `error` responses.
struct JsonRpcFailure {
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<Value>,
}

impl JsonRpcFailure {
    fn method_not_found(method: &str) -> Self {
        Self {
            code: protocol::METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        }
    }
    fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: protocol::INVALID_PARAMS,
            message: msg.into(),
            data: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn roundtrip(input: &str) -> String {
        let mut out = Vec::<u8>::new();
        run(Cursor::new(input.as_bytes()), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn initialize_handshake_returns_protocol_version() {
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let body = roundtrip(&format!("{req}\n"));
        assert!(body.contains("\"protocolVersion\":\"2024-11-05\""));
        assert!(body.contains("\"name\":\"ratchet\""));
    }

    #[test]
    fn tools_list_advertises_catalog() {
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let body = roundtrip(&format!("{req}\n"));
        assert!(body.contains("\"readiness\""));
        assert!(body.contains("\"check-upgrade\""));
        assert!(body.contains("\"observe-program\""));
    }

    #[test]
    fn tools_call_dispatches_to_rule_catalog() {
        // Transport is newline-delimited, so the request must fit on
        // one line. Keep the JSON compact here.
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list-rules-preflight","arguments":{}}}"#;
        let body = roundtrip(&format!("{req}\n"));
        assert!(body.contains("\"isError\":false"));
        assert!(body.contains("\"P001\""));
    }

    #[test]
    fn unknown_method_returns_error_response() {
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"does-not-exist"}"#;
        let body = roundtrip(&format!("{req}\n"));
        assert!(body.contains("\"code\":-32601"));
        assert!(body.contains("method not found"));
    }

    #[test]
    fn notifications_never_reply() {
        let req = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let body = roundtrip(&format!("{req}\n"));
        assert!(body.is_empty());
    }

    #[test]
    fn parse_errors_surface_with_null_id() {
        let body = roundtrip("not json\n");
        assert!(body.contains("\"code\":-32700"));
    }

    #[test]
    fn failing_tool_call_reports_is_error_instead_of_jsonrpc_error() {
        let req = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"readiness","arguments":{}}}"#;
        let body = roundtrip(&format!("{req}\n"));
        // Neither arg provided -> tool returns an error; the dispatch
        // layer wraps it as a content block, not a protocol error.
        assert!(body.contains("\"isError\":true"));
        assert!(!body.contains("\"code\":-32"));
    }
}
