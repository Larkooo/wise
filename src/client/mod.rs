// WiseClient: thin async wrapper over reqwest::Client.
//
// Responsibilities:
//   - Pin the base URL based on env (sandbox vs production)
//   - Attach Authorization + idempotency headers
//   - Parse JSON responses or surface a structured WiseError
//   - Provide convenience methods for GET / POST / PUT / PATCH / DELETE
//   - Expose raw streaming for the docs ask-ai SSE endpoint

use anyhow::{anyhow, Context as _, Result};
use reqwest::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
    Method, RequestBuilder, Response, StatusCode,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

use crate::config::Env;

pub mod error;
pub mod sse;

pub use error::WiseError;

const USER_AGENT: &str = concat!("wise-cli/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct WiseClient {
    http: reqwest::Client,
    env: Env,
    token: Option<String>,
}

impl WiseClient {
    pub fn new(env: Env, token: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .context("building http client")?;
        Ok(Self { http, env, token })
    }

    pub fn env(&self) -> Env {
        self.env
    }

    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn api_base(&self) -> &'static str {
        match self.env {
            Env::Sandbox => "https://api.wise-sandbox.com",
            Env::Production => "https://api.wise.com",
        }
    }

    /// Build a request with default headers (Auth, Accept).
    fn req(&self, method: Method, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.api_base(), path);
        let mut rb = self
            .http
            .request(method, url)
            .header(ACCEPT, "application/json");
        if let Some(t) = &self.token {
            rb = rb.header(AUTHORIZATION, format!("Bearer {}", t));
        }
        rb
    }

    /// GET path → JSON-decoded T.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.req(Method::GET, path).send().await?;
        decode_json(resp).await
    }

    /// GET with query params (any Serialize value).
    pub async fn get_query<T: DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
    ) -> Result<T> {
        let resp = self.req(Method::GET, path).query(query).send().await?;
        decode_json(resp).await
    }

    /// POST with a JSON body.
    pub async fn post<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .req(Method::POST, path)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await?;
        decode_json(resp).await
    }

    /// POST with a JSON body and an `X-idempotence-uuid` header.
    pub async fn post_idempotent<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let id = Uuid::new_v4().to_string();
        let resp = self
            .req(Method::POST, path)
            .header(CONTENT_TYPE, "application/json")
            .header("X-idempotence-uuid", id)
            .json(body)
            .send()
            .await?;
        decode_json(resp).await
    }

    /// POST form-urlencoded — used by /oauth/token.
    pub async fn post_form_basic<T: DeserializeOwned, F: Serialize + ?Sized>(
        &self,
        path: &str,
        form: &F,
        basic_user: &str,
        basic_pass: &str,
    ) -> Result<T> {
        let url = format!("{}{}", self.api_base(), path);
        let resp = self
            .http
            .post(url)
            .basic_auth(basic_user, Some(basic_pass))
            .header(ACCEPT, "application/json")
            .form(form)
            .send()
            .await?;
        decode_json(resp).await
    }

    /// PUT JSON.
    pub async fn put<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .req(Method::PUT, path)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await?;
        decode_json(resp).await
    }

    /// PUT with no body.
    pub async fn put_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.req(Method::PUT, path).send().await?;
        decode_json(resp).await
    }

    /// PATCH JSON.
    pub async fn patch<T: DeserializeOwned, B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .req(Method::PATCH, path)
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await?;
        decode_json(resp).await
    }

    /// DELETE — returns the parsed JSON body if any, or `Value::Null` on 204.
    pub async fn delete(&self, path: &str) -> Result<Value> {
        let resp = self.req(Method::DELETE, path).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(map_error(status, &bytes).into());
        }
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        }
    }

    /// GET → raw bytes (used for PDF receipts, statements, etc).
    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let resp = self.req(Method::GET, path).send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(map_error(status, &bytes).into());
        }
        Ok(bytes.to_vec())
    }

    /// POST → SSE stream of `Event`s. Used by the docs ask-ai endpoint.
    /// `base_override` lets us hit `docs.wise.com` instead of api.wise.com.
    pub async fn post_sse<B: Serialize + ?Sized>(
        &self,
        base_override: Option<&str>,
        path: &str,
        body: &B,
    ) -> Result<Response> {
        let base = base_override.unwrap_or(self.api_base());
        let url = format!("{}{}", base, path);
        let mut rb = self
            .http
            .post(url)
            .header(ACCEPT, "text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .json(body);
        if let Some(t) = &self.token {
            rb = rb.header(AUTHORIZATION, format!("Bearer {}", t));
        }
        let resp = rb.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let bytes = resp.bytes().await.unwrap_or_default();
            return Err(map_error(status, &bytes).into());
        }
        Ok(resp)
    }
}

/// Decode a Response into either T or a structured WiseError.
async fn decode_json<T: DeserializeOwned>(resp: Response) -> Result<T> {
    let status = resp.status();
    let bytes = resp.bytes().await?;
    if !status.is_success() {
        return Err(map_error(status, &bytes).into());
    }
    if bytes.is_empty() {
        // Some endpoints (DELETE, etc) return 204 — try to deserialize Null.
        return serde_json::from_value(Value::Null)
            .map_err(|e| anyhow!("expected JSON body, got empty response: {e}"));
    }
    serde_json::from_slice::<T>(&bytes).map_err(|e| {
        let preview = String::from_utf8_lossy(&bytes);
        let snippet: String = preview.chars().take(500).collect();
        anyhow!("decoding response body failed: {e}\nbody (truncated): {snippet}")
    })
}

/// Map an error response into a WiseError. Tries the common Wise shapes:
///   {"errors":[{"code":"...","message":"..."}]}
///   {"error":"...", "error_description":"..."}
///   plain text fallback
fn map_error(status: StatusCode, bytes: &[u8]) -> WiseError {
    let text = String::from_utf8_lossy(bytes).to_string();
    let parsed: Option<Value> = serde_json::from_slice(bytes).ok();

    let (code, message) = match parsed.as_ref() {
        Some(Value::Object(map)) => {
            if let Some(arr) = map.get("errors").and_then(|v| v.as_array()) {
                let first = arr.first();
                let code = first
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("wise_error")
                    .to_string();
                let msg = arr
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                (code, msg)
            } else if let Some(err) = map.get("error").and_then(|e| e.as_str()) {
                let desc = map
                    .get("error_description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                (err.to_string(), desc)
            } else if let Some(msg) = map.get("message").and_then(|m| m.as_str()) {
                ("wise_error".to_string(), msg.to_string())
            } else {
                ("wise_error".to_string(), text.clone())
            }
        }
        _ => ("wise_error".to_string(), text.clone()),
    };

    WiseError {
        status: status.as_u16(),
        code,
        message,
        body: parsed,
    }
}

