//! Raw HTTP against the Anthropic Messages API.
//!
//! Raw rather than an SDK because there is no official Anthropic SDK for Rust. The
//! transport itself lives in [`crate::http`], which every provider shares; what is here
//! is the dialect — the headers, the message shape, and how a reply comes apart.
//!
//! Everything in this module is platform-independent and tested, because it is the part
//! with logic in it. Nothing here knows what a note is.

use serde_json::{json, Value};

use crate::error::AiError;
use crate::http;
use crate::provider::{ModelInfo, Reply, ToolUse, Turn};

/// The API version header. Pinned, not derived from anything.
const API_VERSION: &str = "2023-06-01";

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Where to send requests. Overridable so a local stub can stand in.
fn base_url() -> String {
    std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

/// List the models this key can use.
///
/// Fetched at runtime rather than hardcoded, as the spec requires: a hardcoded list goes
/// stale the week a model ships, and the set a key can reach is not knowable from here.
pub fn list_models(api_key: &str) -> Result<Vec<ModelInfo>, AiError> {
    let url = format!("{}/v1/models?limit=100", base_url());
    let body = http::interpret(http::get(&url, &headers(api_key))?)?;

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

/// The headers every Anthropic request carries.
fn headers(api_key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-api-key", api_key.to_string()),
        ("anthropic-version", API_VERSION.to_string()),
    ]
}

/// Turn the conversation into Anthropic's message list.
///
/// The system prompt is *not* in here — it is a top-level field on the request, which is
/// the main structural difference from the OpenAI dialect.
pub fn messages(turns: &[Turn]) -> Vec<Value> {
    turns
        .iter()
        .map(|turn| match turn {
            Turn::Prompt(text) => json!({"role": "user", "content": text}),
            // Echoed verbatim: thinking blocks carry signatures that any reconstruction
            // of the content array would invalidate.
            Turn::Reply(echo) => json!({"role": "assistant", "content": echo}),
            Turn::ToolResults(results) => json!({
                "role": "user",
                "content": results
                    .iter()
                    .map(|result| json!({
                        "type": "tool_result",
                        "tool_use_id": result.id,
                        "content": result.text,
                        "is_error": !result.ok,
                    }))
                    .collect::<Vec<_>>(),
            }),
        })
        .collect()
}

/// One `POST /v1/messages`.
///
/// `thinking` is requested as adaptive, which the current model generation supports and
/// older ones reject outright. Rather than maintain a table of which model ids allow it
/// — a table that is wrong the moment a model ships — a 400 that names `thinking` is
/// retried once without it. The user picked the model; they should not have to know this.
pub fn send(
    api_key: &str,
    model: &str,
    system: &str,
    turns: &[Turn],
    tools: &[Value],
    max_tokens: u32,
) -> Result<Reply, AiError> {
    let url = format!("{}/v1/messages", base_url());
    let headers = headers(api_key);
    let messages = messages(turns);

    let body = match http::interpret(http::post(
        &url,
        &headers,
        &request_body(model, system, &messages, tools, max_tokens, true),
    )?) {
        Err(AiError::Api { status: 400, message }) if message.contains("thinking") => {
            http::interpret(http::post(
                &url,
                &headers,
                &request_body(model, system, &messages, tools, max_tokens, false),
            )?)
        }
        other => other,
    }?;

    parse_reply(&body)
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
        echo: content,
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
        assert_eq!(reply.tool_uses.len(), 1);
        assert_eq!(reply.tool_uses[0].name, "transpose");
        assert_eq!(reply.tool_uses[0].input["semitones"], 7);
        assert_eq!(reply.usage.input_tokens, 120);

        // The thinking block must survive into what is echoed back, or the next request
        // is rejected for a missing signature.
        assert_eq!(reply.echo.as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_reply_with_no_content_is_a_protocol_error() {
        assert!(parse_reply(&json!({"stop_reason": "end_turn"})).is_err());
    }



    #[test]
    fn the_conversation_becomes_anthropics_message_list() {
        use crate::provider::ToolResult;

        let out = messages(&[
            Turn::Prompt("a ii-V-I".into()),
            Turn::Reply(json!([{"type": "text", "text": "ok"}])),
            Turn::ToolResults(vec![ToolResult {
                id: "tu_1".into(),
                text: "1/7 is not a note value".into(),
                ok: false,
            }]),
        ]);

        assert_eq!(out.len(), 3, "the system prompt is a field here, not a message");
        assert_eq!(out[0], json!({"role": "user", "content": "a ii-V-I"}));
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(out[2]["role"], "user", "tool results ride in a user turn");
        assert_eq!(out[2]["content"][0]["type"], "tool_result");
        assert_eq!(out[2]["content"][0]["tool_use_id"], "tu_1");
        assert_eq!(out[2]["content"][0]["is_error"], true);
    }
}
