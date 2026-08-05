//! An OpenAI-compatible client, for LM Studio and anything that speaks the same dialect.
//!
//! The differences from Anthropic are all above the transport and all mechanical:
//!
//! * The system prompt is a message with `role: "system"`, not a top-level field.
//! * Tools are wrapped in `{"type": "function", "function": {...}}`, and the schema key
//!   is `parameters` rather than `input_schema`.
//! * A tool call's arguments arrive as a **JSON string**, not an object — the single
//!   most common way an integration with this API goes subtly wrong.
//! * A tool result is its own message with `role: "tool"`, one per call, rather than a
//!   block inside a user message.
//!
//! The assistant's message goes back verbatim, as with Anthropic, but for a duller
//! reason: some servers are strict about the `tool_calls` they see echoed, and rebuilding
//! it from parsed parts is a way to get that wrong for no benefit.
//!
//! Nothing here knows what a note is.

use serde_json::{json, Value};

use crate::error::AiError;
use crate::http;
use crate::provider::{ModelInfo, Reply, ToolUse, Turn, Usage};

/// No key: a local server does not want one, and sending a placeholder to a service that
/// *does* check would be worse than sending nothing.
fn headers() -> Vec<(&'static str, String)> {
    Vec::new()
}

/// `GET /models`. LM Studio reports what is loaded; `id` is what to send back.
pub fn list_models(base_url: &str) -> Result<Vec<ModelInfo>, AiError> {
    let body = http::interpret(http::get(&format!("{base_url}/models"), &headers())?)?;

    let data = body["data"]
        .as_array()
        .ok_or_else(|| AiError::Protocol("no model list in the response".into()))?;

    Ok(data
        .iter()
        .filter_map(|entry| entry["id"].as_str())
        .filter(|id| !id.is_empty())
        .map(|id| ModelInfo { id: id.to_string(), display_name: id.to_string() })
        .collect())
}

/// One `POST /chat/completions`.
pub fn send(
    base_url: &str,
    model: &str,
    system: &str,
    turns: &[Turn],
    tools: &[Value],
    max_tokens: u32,
) -> Result<Reply, AiError> {
    let body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": messages(system, turns),
        "tools": tool_schema(tools),
        // Explicit rather than default: a local model handed tools will otherwise
        // sometimes describe the call in prose instead of making it.
        "tool_choice": "auto",
        "stream": false,
    });

    let response = http::interpret(http::post(
        &format!("{base_url}/chat/completions"),
        &headers(),
        &body,
    )?)?;
    parse_reply(&response)
}

/// Turn the conversation into OpenAI's message list.
pub fn messages(system: &str, turns: &[Turn]) -> Vec<Value> {
    let mut out = vec![json!({"role": "system", "content": system})];

    for turn in turns {
        match turn {
            Turn::Prompt(text) => out.push(json!({"role": "user", "content": text})),
            Turn::Reply(echo) => out.push(echo.clone()),
            Turn::ToolResults(results) => {
                for result in results {
                    // No `is_error` in this dialect: a failure has to be readable as
                    // text, or the model cannot tell it went wrong.
                    let content = if result.ok {
                        result.text.clone()
                    } else {
                        format!("Error: {}", result.text)
                    };
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": result.id,
                        "content": content,
                    }));
                }
            }
        }
    }

    out
}

/// Rewrap Anthropic-shaped tool definitions as OpenAI functions.
///
/// One tool surface, described once, translated at the edge. The alternative — a second
/// set of definitions — is two places to forget when a tool changes.
pub fn tool_schema(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool["description"],
                    "parameters": tool["input_schema"],
                },
            })
        })
        .collect()
}

pub fn parse_reply(body: &Value) -> Result<Reply, AiError> {
    let message = body["choices"][0]["message"].clone();
    if !message.is_object() {
        return Err(AiError::Protocol("the reply had no message".into()));
    }

    let text = message["content"].as_str().unwrap_or_default().to_string();

    let mut tool_uses = Vec::new();
    if let Some(calls) = message["tool_calls"].as_array() {
        for call in calls {
            let function = &call["function"];
            // Arguments arrive as a JSON *string*. A model that emits something
            // unparseable is common enough that it must not end the conversation: an
            // empty object reaches the tool, which reports what was wrong with it, and
            // the model gets a chance to correct itself.
            let input = function["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .or_else(|| function["arguments"].as_object().map(|_| function["arguments"].clone()))
                .unwrap_or_else(|| json!({}));

            tool_uses.push(ToolUse {
                id: call["id"].as_str().unwrap_or_default().to_string(),
                name: function["name"].as_str().unwrap_or_default().to_string(),
                input,
            });
        }
    }

    Ok(Reply {
        echo: message,
        text,
        tool_uses,
        usage: Usage {
            input_tokens: body["usage"]["prompt_tokens"].as_u64().unwrap_or_default(),
            output_tokens: body["usage"]["completion_tokens"].as_u64().unwrap_or_default(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolResult;

    #[test]
    fn the_system_prompt_leads_the_message_list() {
        let out = messages("be helpful", &[Turn::Prompt("a ii-V-I".into())]);
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[0]["content"], "be helpful");
        assert_eq!(out[1]["role"], "user");
        assert_eq!(out[1]["content"], "a ii-V-I");
    }

    #[test]
    fn each_tool_result_is_its_own_message_keyed_to_its_call() {
        let out = messages(
            "s",
            &[Turn::ToolResults(vec![
                ToolResult { id: "call_1".into(), text: "ok".into(), ok: true },
                ToolResult { id: "call_2".into(), text: "1/7 is not a note value".into(), ok: false },
            ])],
        );

        assert_eq!(out.len(), 3, "system plus one per result");
        assert_eq!(out[1]["role"], "tool");
        assert_eq!(out[1]["tool_call_id"], "call_1");
        assert_eq!(out[1]["content"], "ok");
        assert!(
            out[2]["content"].as_str().unwrap().starts_with("Error:"),
            "a failure has to read as one — there is no is_error flag in this dialect"
        );
    }

    #[test]
    fn the_assistant_turn_goes_back_exactly_as_it_arrived() {
        let echo = json!({"role": "assistant", "content": null, "tool_calls": [{"id": "x"}]});
        let out = messages("s", &[Turn::Reply(echo.clone())]);
        assert_eq!(out[1], echo);
    }

    #[test]
    fn tools_are_rewrapped_as_functions_with_their_schema_renamed() {
        let anthropic = vec![json!({
            "name": "add_notes",
            "description": "Add notes",
            "input_schema": {"type": "object", "properties": {"notes": {"type": "array"}}},
        })];

        let converted = tool_schema(&anthropic);
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["function"]["name"], "add_notes");
        assert_eq!(converted[0]["function"]["description"], "Add notes");
        assert_eq!(
            converted[0]["function"]["parameters"],
            anthropic[0]["input_schema"],
            "the schema travels unchanged — only the key it sits under differs"
        );
    }

    #[test]
    fn arguments_arrive_as_a_json_string_and_are_parsed() {
        let reply = parse_reply(&json!({
            "choices": [{"message": {
                "role": "assistant",
                "content": "adding a third",
                "tool_calls": [{
                    "id": "call_9",
                    "type": "function",
                    "function": {"name": "add_notes", "arguments": "{\"pitch\": 64}"},
                }],
            }}],
            "usage": {"prompt_tokens": 120, "completion_tokens": 8},
        }))
        .unwrap();

        assert_eq!(reply.text, "adding a third");
        assert_eq!(reply.tool_uses.len(), 1);
        assert_eq!(reply.tool_uses[0].name, "add_notes");
        assert_eq!(reply.tool_uses[0].input["pitch"], 64);
        assert_eq!(reply.usage, Usage { input_tokens: 120, output_tokens: 8 });
    }

    #[test]
    fn arguments_that_are_already_an_object_are_accepted_too() {
        // Not to spec, but servers do it, and refusing would strand a working setup.
        let reply = parse_reply(&json!({
            "choices": [{"message": {
                "tool_calls": [{"id": "c", "function": {"name": "t", "arguments": {"pitch": 60}}}],
            }}],
        }))
        .unwrap();
        assert_eq!(reply.tool_uses[0].input["pitch"], 60);
    }

    #[test]
    fn unparseable_arguments_reach_the_tool_rather_than_ending_the_conversation() {
        let reply = parse_reply(&json!({
            "choices": [{"message": {
                "tool_calls": [{"id": "c", "function": {"name": "t", "arguments": "{not json"}}],
            }}],
        }))
        .unwrap();

        assert_eq!(reply.tool_uses.len(), 1);
        assert_eq!(reply.tool_uses[0].input, json!({}), "the tool will say what was wrong");
    }

    #[test]
    fn a_plain_answer_with_no_tool_calls_ends_the_loop() {
        let reply = parse_reply(&json!({
            "choices": [{"message": {"role": "assistant", "content": "I would not change it."}}],
        }))
        .unwrap();
        assert!(reply.tool_uses.is_empty());
        assert_eq!(reply.text, "I would not change it.");
    }

    #[test]
    fn a_reply_with_no_message_is_a_protocol_error() {
        assert!(parse_reply(&json!({"choices": []})).is_err());
    }
}
