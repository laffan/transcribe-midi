use crate::keychain::KeychainError;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("no Anthropic API key is set — add one in Settings → AI")]
    NoApiKey,

    #[error("{0}")]
    Keychain(#[from] KeychainError),

    #[error("could not reach the Anthropic API: {0}")]
    Transport(String),

    /// The API answered, and said no. `message` is its own wording, which is almost
    /// always more useful than anything we would write over the top of it.
    #[error("Anthropic API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("unexpected response from the Anthropic API: {0}")]
    Protocol(String),

    #[error("{0}")]
    Core(#[from] unplugged_core::CoreError),

    #[error("the model called {0} tools without finishing — stopping before it runs away")]
    ToolLimit(usize),

    #[error("no prompt was given")]
    EmptyPrompt,
}

pub type Result<T> = std::result::Result<T, AiError>;
