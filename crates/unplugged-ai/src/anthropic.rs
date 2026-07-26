//! Raw HTTP against the Anthropic Messages API.
//!
//! Raw rather than an SDK because there is no official Anthropic SDK for Rust. `ureq` is
//! a blocking, pure-Rust client; the tool loop already runs on its own thread, so there
//! is nothing an async runtime would buy us.
//!
//! The transport is **Apple-only**, for two reasons that happen to agree. The feature
//! itself is Apple-only — off-Apple there is no Keychain to hold a key — and `rustls`'s
//! crypto backends need a C compiler for the Apple targets, which the Linux build host
//! does not have, so pulling one in would cost us the cross-compile check that catches
//! API breakage before it reaches a Mac. `native-tls` on macOS and iOS is
//! Security.framework through pure-Rust bindings: no C to build, and certificate
//! verification follows the system trust store rather than a root list baked into the
//! binary.
//!
//! Everything that interprets a response is platform-independent and tested, because
//! that is the part with logic in it. Nothing in this module knows what a note is.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::AiError;

/// The API version header. Pinned, not derived from anything.
///
/// Only the Apple transport sends it; off-Apple nothing is sent at all.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
const API_VERSION: &str = "2023-06-01";

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Where to send requests. Overridable so a local stub can stand in.
fn base_url() -> String {
    std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

/// A response, reduced to the two things the rest of this file cares about.
struct HttpResponse {
    status: u16,
    body: Value,
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod transport {
    use std::time::Duration;

    use serde_json::Value;
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    use super::{HttpResponse, API_VERSION};
    use crate::error::AiError;

    /// Generous, because a request with extended thinking can legitimately take minutes
    /// and the alternative is a timeout the user reads as a bug.
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            // Read the body on a 4xx: the API's own error messages are the most useful
            // thing it can tell us, and a bare status code throws them away.
            .http_status_as_error(false)
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    // Security.framework's own roots, so an enterprise or MDM trust
                    // policy applies here as it does everywhere else on the device.
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent()
    }

    fn finish(
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<HttpResponse, AiError> {
        let mut response = result.map_err(|e| AiError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body: Value = response
            .body_mut()
            .read_json()
            .map_err(|e| AiError::Protocol(format!("the response was not JSON: {e}")))?;
        Ok(HttpResponse { status, body })
    }

    pub(super) fn get(url: &str, api_key: &str) -> Result<HttpResponse, AiError> {
        finish(
            agent()
                .get(url)
                .header("x-api-key", api_key)
                .header("anthropic-version", API_VERSION)
                .call(),
        )
    }

    pub(super) fn post(url: &str, api_key: &str, body: &Value) -> Result<HttpResponse, AiError> {
        finish(
            agent()
                .post(url)
                .header("x-api-key", api_key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json")
                .send_json(body),
        )
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod transport {
    use serde_json::Value;

    use super::HttpResponse;
    use crate::error::AiError;

    fn unavailable() -> AiError {
        AiError::Transport("AI editing needs the macOS or iOS build".into())
    }

    pub(super) fn get(_url: &str, _api_key: &str) -> Result<HttpResponse, AiError> {
        Err(unavailable())
    }

    pub(super) fn post(_url: &str, _key: &str, _body: &Value) -> Result<HttpResponse, AiError> {
        Err(unavailable())
    }
}

/// A model as reported by `GET /v1/models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
}

/// Token counts, surfaced in the panel so cost is visible rather than invisible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
    }
}

/// Either the JSON body, or a typed API error carrying the service's own wording.
fn interpret(response: HttpResponse) -> Result<Value, AiError> {
    if (200..300).contains(&response.status) {
        return Ok(response.body);
    }

    let message = response.body["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| response.body.to_string());

    Err(AiError::Api { status: response.status, message })
}

/// List the models this key can use.
///
/// Fetched at runtime rather than hardcoded, as the spec requires: a hardcoded list goes
/// stale the week a model ships, and the set a key can reach is not knowable from here.
pub fn list_models(api_key: &str) -> Result<Vec<ModelInfo>, AiError> {
    let url = format!("{}/v1/models?limit=100", base_url());
    let body = interpret(transport::get(&url, api_key)?)?;

    let data = body["data"]
        .as_array()
        .ok_or_else(|| AiError::Protocol("no model list in the response".into()))?;

    Ok(data
        .iter()
        .filter_map(|entry| serde_json::from_value::<ModelInfo>(entry.clone()).ok())
        .filter(|model| !model.id.is_empty())
        .collect())
}

/// Pick the default model from a live list.
///
/// The spec asks for the first Sonnet-class model returned; `/v1/models` returns newest
/// first, so that is the newest Sonnet. Falling back to the first model at all matters:
/// a key scoped to a single non-Sonnet model would otherwise land on no default and the
/// picker would open empty.
pub fn default_model(models: &[ModelInfo]) -> Option<String> {
    models
        .iter()
        .find(|model| model.id.to_ascii_lowercase().contains("sonnet"))
        .or_else(|| models.first())
        .map(|model| model.id.clone())
}

/// One `POST /v1/messages`.
///
/// `thinking` is requested as adaptive, which the current model generation supports and
/// older ones reject outright. Rather than maintain a table of which model ids allow it
/// — a table that is wrong the moment a model ships — a 400 that names `thinking` is
/// retried once without it. The user picked the model; they should not have to know this.
pub fn send_message(
    api_key: &str,
    model: &str,
    system: &str,
    messages: &[Value],
    tools: &[Value],
    max_tokens: u32,
) -> Result<Value, AiError> {
    let url = format!("{}/v1/messages", base_url());

    match interpret(transport::post(&url, api_key, &request_body(model, system, messages, tools, max_tokens, true))?)
    {
        Err(AiError::Api { status: 400, message }) if message.contains("thinking") => interpret(
            transport::post(
                &url,
                api_key,
                &request_body(model, system, messages, tools, max_tokens, false),
            )?,
        ),
        other => other,
    }
}

fn request_body(
    model: &str,
    system: &str,
    messages: &[Value],
    tools: &[Value],
    max_tokens: u32,
    thinking: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": messages,
        "tools": tools,
    });
    if thinking {
        body["thinking"] = json!({"type": "adaptive"});
    }
    body
}

/// A `tool_use` block pulled out of an assistant message.
#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// The tool calls, the visible text and the token usage from one response.
///
/// The assistant `content` array is carried alongside them untouched, because it has to
/// go back verbatim in the next request — thinking blocks carry signatures that any
/// reconstruction would invalidate.
pub struct Reply {
    pub content: Value,
    pub stop_reason: String,
    pub text: String,
    pub tool_uses: Vec<ToolUse>,
    pub usage: Usage,
}

pub fn parse_reply(body: &Value) -> Result<Reply, AiError> {
    let content = body
        .get("content")
        .cloned()
        .ok_or_else(|| AiError::Protocol("the reply had no content".into()))?;

    let blocks = content
        .as_array()
        .ok_or_else(|| AiError::Protocol("the reply's content was not a list".into()))?;

    let mut text = String::new();
    let mut tool_uses = Vec::new();

    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(chunk) = block["text"].as_str() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(chunk);
                }
            }
            Some("tool_use") => tool_uses.push(ToolUse {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                input: block["input"].clone(),
            }),
            // Thinking and redacted-thinking blocks travel through in `content` but are
            // deliberately not surfaced: they are the model's scratch work, not its
            // answer to the user.
            _ => {}
        }
    }

    Ok(Reply {
        content,
        stop_reason: body["stop_reason"].as_str().unwrap_or_default().to_string(),
        text,
        tool_uses,
        usage: serde_json::from_value(body["usage"].clone()).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> ModelInfo {
        ModelInfo { id: id.into(), display_name: id.into() }
    }

    #[test]
    fn the_default_is_the_first_sonnet_returned() {
        let models = vec![
            model("claude-opus-5"),
            model("claude-sonnet-5"),
            model("claude-sonnet-4-6"),
        ];
        assert_eq!(default_model(&models).as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn without_a_sonnet_the_first_model_is_used() {
        let models = vec![model("claude-opus-5"), model("claude-haiku-4-5")];
        assert_eq!(default_model(&models).as_deref(), Some("claude-opus-5"));
    }

    #[test]
    fn an_empty_list_has_no_default() {
        assert_eq!(default_model(&[]), None);
    }

    #[test]
    fn a_success_yields_the_body() {
        let response = HttpResponse { status: 200, body: json!({"ok": true}) };
        assert_eq!(interpret(response).unwrap(), json!({"ok": true}));
    }

    #[test]
    fn an_error_keeps_the_services_own_wording() {
        let response = HttpResponse {
            status: 401,
            body: json!({"type": "error", "error": {"type": "authentication_error",
                                                    "message": "invalid x-api-key"}}),
        };
        match interpret(response).unwrap_err() {
            AiError::Api { status, message } => {
                assert_eq!(status, 401);
                assert_eq!(message, "invalid x-api-key");
            }
            other => panic!("expected an API error, got {other}"),
        }
    }

    #[test]
    fn an_error_with_no_message_still_says_something() {
        let response = HttpResponse { status: 500, body: json!({"oops": 1}) };
        match interpret(response).unwrap_err() {
            AiError::Api { status, message } => {
                assert_eq!(status, 500);
                assert!(message.contains("oops"), "{message}");
            }
            other => panic!("expected an API error, got {other}"),
        }
    }

    #[test]
    fn thinking_is_requested_by_default_and_droppable() {
        let with = request_body("claude-sonnet-5", "sys", &[], &[], 8192, true);
        assert_eq!(with["thinking"], json!({"type": "adaptive"}));
        assert_eq!(with["max_tokens"], 8192);

        let without = request_body("claude-sonnet-5", "sys", &[], &[], 8192, false);
        assert!(without.get("thinking").is_none());
    }

    #[test]
    fn a_reply_splits_into_text_and_tool_calls() {
        let body = json!({
            "content": [
                {"type": "thinking", "thinking": "…", "signature": "abc"},
                {"type": "text", "text": "Transposing up a fifth."},
                {"type": "tool_use", "id": "tu_1", "name": "transpose", "input": {"semitones": 7}},
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 120, "output_tokens": 45},
        });

        let reply = parse_reply(&body).unwrap();
        assert_eq!(reply.text, "Transposing up a fifth.");
        assert_eq!(reply.stop_reason, "tool_use");
        assert_eq!(reply.tool_uses.len(), 1);
        assert_eq!(reply.tool_uses[0].name, "transpose");
        assert_eq!(reply.tool_uses[0].input["semitones"], 7);
        assert_eq!(reply.usage.input_tokens, 120);

        // The thinking block must survive into the echoed content, or the next request
        // is rejected for a missing signature.
        assert_eq!(reply.content.as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_reply_with_no_content_is_a_protocol_error() {
        assert!(parse_reply(&json!({"stop_reason": "end_turn"})).is_err());
    }

    #[test]
    fn usage_accumulates_across_turns() {
        let mut total = Usage::default();
        total.add(&Usage { input_tokens: 10, output_tokens: 5 });
        total.add(&Usage { input_tokens: 7, output_tokens: 3 });
        assert_eq!(total, Usage { input_tokens: 17, output_tokens: 8 });
    }
}
