//! JSON-line RPC envelope for `homn.sock`.
//!
//! Wire format: one JSON object per line, with a `Request` carrying `id`, `method`, `params`, and
//! a matching `Response` carrying the same `id` plus either a `result` or an `error`.
//!
//! See [`docs/technical/ipc-protocol.md`](../../../docs/technical/ipc-protocol.md) and
//! [`specs/001-policy-engine/contracts/hook-protocol.md`](../../../specs/001-policy-engine/contracts/hook-protocol.md).

use serde::{Deserialize, Serialize};

/// A request envelope: `{id, method, params}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Caller-chosen correlation id (ULID recommended).
    pub id: String,
    /// Dotted method name (`decisions.create`, `policies.reload`, `ping`, …).
    pub method: String,
    /// Method-specific parameters as JSON.
    #[serde(default = "default_params")]
    pub params: serde_json::Value,
}

fn default_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// A response envelope: either `{id, result}` or `{id, error}` — never both.
///
/// Encoded as an untagged enum so the wire format is `{id, result}` xor `{id, error}` rather than
/// the verbose serde-tagged form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    /// A successful response carrying a result payload.
    Ok {
        /// Echoed correlation id.
        id: String,
        /// Method-specific result payload.
        result: serde_json::Value,
    },
    /// An error response carrying a structured error object.
    Err {
        /// Echoed correlation id.
        id: String,
        /// Structured error.
        error: ErrorObject,
    },
}

impl Response {
    /// Construct a success response.
    pub fn ok(id: impl Into<String>, result: serde_json::Value) -> Self {
        Self::Ok {
            id: id.into(),
            result,
        }
    }

    /// Construct an error response.
    pub fn err(id: impl Into<String>, err: ErrorObject) -> Self {
        Self::Err {
            id: id.into(),
            error: err,
        }
    }

    /// Borrow the response's correlation id.
    pub fn id(&self) -> &str {
        match self {
            Self::Ok { id, .. } | Self::Err { id, .. } => id,
        }
    }
}

/// Structured error returned to RPC callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ErrorObject {
    /// Stable error code (`unknown_method`, `policy_unavailable`, `internal`, …).
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

impl ErrorObject {
    /// Construct an [`ErrorObject`] from a code + message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Top-level errors the RPC layer surfaces internally (before they're flattened to wire).
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    /// The peer sent a malformed JSON line.
    #[error("invalid frame: {0}")]
    InvalidFrame(#[from] serde_json::Error),
    /// The peer requested a method the daemon doesn't expose.
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    /// I/O failure on the underlying socket.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trips() {
        let req = Request {
            id: "01HXY".into(),
            method: "decisions.create".into(),
            params: json!({"tool_name": "Bash"}),
        };
        let s = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn request_params_default_to_empty_object() {
        let req: Request = serde_json::from_str("{\"id\":\"a\",\"method\":\"ping\"}").unwrap();
        assert_eq!(req.id, "a");
        assert_eq!(req.method, "ping");
        assert_eq!(req.params, json!({}));
    }

    #[test]
    fn response_ok_serializes_with_result_field() {
        let resp = Response::ok("01HXY", json!({"pong": true}));
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["id"], "01HXY");
        assert_eq!(v["result"]["pong"], true);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn response_err_serializes_with_error_field() {
        let resp = Response::err("01HXY", ErrorObject::new("internal", "boom"));
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["id"], "01HXY");
        assert_eq!(v["error"]["code"], "internal");
        assert_eq!(v["error"]["message"], "boom");
        assert!(v.get("result").is_none());
    }

    #[test]
    fn response_round_trips_both_variants() {
        let ok = Response::ok("a", json!({"x": 1}));
        let ok_s = serde_json::to_string(&ok).unwrap();
        let parsed_ok: Response = serde_json::from_str(&ok_s).unwrap();
        assert_eq!(parsed_ok, ok);

        let err = Response::err("b", ErrorObject::new("nope", "no"));
        let err_s = serde_json::to_string(&err).unwrap();
        let parsed_err: Response = serde_json::from_str(&err_s).unwrap();
        assert_eq!(parsed_err, err);
    }

    #[test]
    fn response_id_accessor() {
        assert_eq!(Response::ok("a", json!(null)).id(), "a");
        assert_eq!(Response::err("b", ErrorObject::new("x", "y")).id(), "b");
    }
}
