// Structured error type for Wise API failures. We carry the parsed body
// (if any) so the agent gets the full error context in `wise --pretty`.

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("wise api error {status} ({code}): {message}")]
pub struct WiseError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub body: Option<Value>,
}

impl WiseError {
    /// Render as the JSON shape we use on stderr for `--json` output.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "error": {
                "status": self.status,
                "code": self.code,
                "message": self.message,
                "details": self.body,
            }
        })
    }
}
