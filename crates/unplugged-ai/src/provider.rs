//! Which service the tool loop is talking to, and the shape of a conversation with it.
//!
//! The loop used to build Anthropic JSON directly. That was fine while there was one
//! provider and wrong the moment there were two, because the two disagree about nearly
//! everything above the transport: tool schemas, where a tool result goes, whether the
//! assistant's turn is an object or an array of blocks.
//!
//! So the loop now speaks in [`Turn`]s and a provider serialises them. The one thing that
//! does *not* get normalised is the assistant's own reply: it goes back in the next
//! request as [`Reply::echo`], verbatim, because Anthropic's thinking blocks carry
//! signatures that any reconstruction would invalidate. Each provider hands back whatever
//! it needs to receive, and the loop only carries it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AiError;
use crate::{anthropic, openai};

/// Where a request goes. LM Studio's default; Anthropic's is fixed in its own module.
pub const LM_STUDIO_DEFAULT_URL: &str = "http://localhost:1234/v1";

/// Which service to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Anthropic,
    /// Any OpenAI-compatible server on this machine. Named for the one this was built
    /// and tested against rather than "local" or "openai", because what it can actually
    /// do depends on the server, and pretending otherwise would set the wrong
    /// expectation about, say, Ollama.
    LmStudio,
}

impl Provider {
    /// Whether a key must be present before a request can be made at all.
    ///
    /// A local server needs none, and demanding one would put a Keychain prompt and a
    /// sign-up in front of a feature that runs entirely on the user's machine.
    pub fn needs_api_key(self) -> bool {
        matches!(self, Provider::Anthropic)
    }
}

/// A configured provider: everything a request needs except the model and the prompt.
pub struct Endpoint {
    pub provider: Provider,
    /// Ignored by Anthropic, which has one address.
    pub base_url: String,
    /// Empty for a provider that needs no key.
    pub api_key: String,
}

impl Endpoint {
    pub fn anthropic(api_key: String) -> Self {
        Endpoint { provider: Provider::Anthropic, base_url: String::new(), api_key }
    }

    pub fn lm_studio(base_url: String) -> Self {
        Endpoint {
            provider: Provider::LmStudio,
            base_url: normalise(&base_url),
            api_key: String::new(),
        }
    }
}

/// Trim a base URL into something two paths can be joined onto.
///
/// Users paste what LM Studio shows them, which is sometimes `http://localhost:1234`,
/// sometimes with `/v1`, and often with a trailing slash. All three should work rather
/// than produce a 404 the user has to guess at.
pub fn normalise(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return LM_STUDIO_DEFAULT_URL.to_string();
    }
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// One turn of the conversation, in neither provider's dialect.
pub enum Turn {
    /// What the user asked for. Only ever the first turn.
    Prompt(String),
    /// The assistant's reply, in the form its own provider needs it back in.
    Reply(Value),
    /// What the tools it called had to say.
    ToolResults(Vec<ToolResult>),
}

pub struct ToolResult {
    pub id: String,
    pub text: String,
    /// A failed tool comes back as an error the model can read and correct, not as a
    /// dead conversation. Half of what makes the loop usable is that "1/7 is not a note
    /// value" is something it can act on.
    pub ok: bool,
}

/// A tool call pulled out of an assistant message.
#[derive(Debug, Clone)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// A model as reported by the provider's model list.
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

/// What one round trip produced.
pub struct Reply {
    /// The assistant's turn, to be handed back verbatim. Opaque to everything but the
    /// provider that made it.
    pub echo: Value,
    pub text: String,
    pub tool_uses: Vec<ToolUse>,
    pub usage: Usage,
}

/// The models this endpoint can reach.
///
/// Fetched at runtime rather than hardcoded, as the spec requires — and for a local
/// server it is the only way to know at all, since what is loaded is the user's business.
pub fn models(endpoint: &Endpoint) -> Result<Vec<ModelInfo>, AiError> {
    match endpoint.provider {
        Provider::Anthropic => anthropic::list_models(&endpoint.api_key),
        Provider::LmStudio => openai::list_models(&endpoint.base_url),
    }
}

/// Pick a default from a live list.
pub fn default_model(endpoint: &Endpoint, models: &[ModelInfo]) -> Option<String> {
    match endpoint.provider {
        Provider::Anthropic => anthropic::default_model(models),
        // Nothing to prefer: which local model is best for this is the user's judgement,
        // and a name-based guess would be wrong as often as right.
        Provider::LmStudio => models.first().map(|model| model.id.clone()),
    }
}

/// One round trip.
pub fn send(
    endpoint: &Endpoint,
    model: &str,
    system: &str,
    turns: &[Turn],
    tools: &[Value],
    max_tokens: u32,
) -> Result<Reply, AiError> {
    match endpoint.provider {
        Provider::Anthropic => {
            anthropic::send(&endpoint.api_key, model, system, turns, tools, max_tokens)
        }
        Provider::LmStudio => {
            openai::send(&endpoint.base_url, model, system, turns, tools, max_tokens)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_cloud_provider_demands_a_key() {
        assert!(Provider::Anthropic.needs_api_key());
        assert!(!Provider::LmStudio.needs_api_key());
    }

    #[test]
    fn a_base_url_is_accepted_in_the_forms_people_paste() {
        for given in [
            "http://localhost:1234",
            "http://localhost:1234/",
            "http://localhost:1234/v1",
            "http://localhost:1234/v1/",
            "  http://localhost:1234/v1  ",
        ] {
            assert_eq!(normalise(given), "http://localhost:1234/v1", "for {given:?}");
        }
    }

    #[test]
    fn an_empty_url_falls_back_to_the_default_rather_than_a_broken_one() {
        assert_eq!(normalise("   "), LM_STUDIO_DEFAULT_URL);
    }

    #[test]
    fn a_url_on_another_host_is_kept() {
        // A server on the network is a legitimate setup, and this must not rewrite it.
        assert_eq!(normalise("http://studio.local:9000"), "http://studio.local:9000/v1");
    }

    #[test]
    fn the_local_default_is_the_first_model_because_there_is_nothing_to_prefer() {
        let endpoint = Endpoint::lm_studio(LM_STUDIO_DEFAULT_URL.into());
        let models = vec![
            ModelInfo { id: "qwen2.5-coder".into(), display_name: String::new() },
            ModelInfo { id: "llama-3.1-8b".into(), display_name: String::new() },
        ];
        assert_eq!(default_model(&endpoint, &models).as_deref(), Some("qwen2.5-coder"));
        assert_eq!(default_model(&endpoint, &[]), None);
    }

    #[test]
    fn usage_accumulates_across_turns() {
        let mut total = Usage::default();
        total.add(&Usage { input_tokens: 10, output_tokens: 2 });
        total.add(&Usage { input_tokens: 5, output_tokens: 3 });
        assert_eq!(total, Usage { input_tokens: 15, output_tokens: 5 });
    }
}
