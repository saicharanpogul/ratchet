//! JSON-RPC 2.0 message types for the MCP stdio transport.
//!
//! The stream is newline-delimited: each line on stdin is one
//! `Request`, each line on stdout is one `Response` or
//! `Notification`. We deliberately keep this skinny rather than
//! reaching for a full JSON-RPC crate — the set of shapes MCP uses is
//! small, and pinning our own types here keeps the upgrade story
//! clean when the spec evolves.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC request received from the client.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// Absent when the request is actually a notification. MCP
    /// notifications are rare in the current flows (just `initialized`),
    /// but the transport handles both cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// A successful response paired with the request `id` it answers.
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub result: Value,
}

/// An error response. `code`/`message` follow the JSON-RPC 2.0 reserved
/// ranges; MCP extensions use codes outside `-32768..-32000`.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub error: ErrorObject,
}

#[derive(Debug, Serialize)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

pub fn success(id: Value, result: Value) -> SuccessResponse {
    SuccessResponse {
        jsonrpc: "2.0",
        id,
        result,
    }
}

pub fn error(id: Value, code: i32, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse {
        jsonrpc: "2.0",
        id,
        error: ErrorObject {
            code,
            message: message.into(),
            data: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_parses_with_and_without_params() {
        let r: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert_eq!(r.method, "tools/list");
        assert_eq!(r.id, Some(json!(1)));
        assert_eq!(r.params, Value::Null);

        let r2: Request = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"x":1}}"#,
        )
        .unwrap();
        assert_eq!(r2.params, json!({"x": 1}));
    }

    #[test]
    fn success_carries_id_and_result() {
        let resp = success(json!("abc"), json!({"ok": true}));
        let body = serde_json::to_string(&resp).unwrap();
        assert!(body.contains(r#""id":"abc""#));
        assert!(body.contains(r#""result":{"ok":true}"#));
    }

    #[test]
    fn error_has_canonical_shape() {
        let resp = error(json!(null), METHOD_NOT_FOUND, "unknown");
        let body = serde_json::to_string(&resp).unwrap();
        assert!(body.contains(r#""code":-32601"#));
        assert!(body.contains(r#""message":"unknown""#));
    }
}
