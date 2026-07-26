use serde::Serialize;
use unplugged_core::CoreError;

/// What a failing command sends to the frontend.
///
/// `CoreError` is not `Serialize` and should not become so — its variants carry
/// `std::io::Error` and filesystem paths. Flattening here keeps absolute paths from
/// leaking into the webview while still giving the console something actionable.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Stable machine-readable discriminator, for the frontend to branch on.
    pub code: &'static str,
    /// Human-readable, safe to show in the UI console.
    pub message: String,
}

impl From<CoreError> for CommandError {
    fn from(err: CoreError) -> Self {
        let code = match err {
            CoreError::EmptyName | CoreError::NameTooLong(_) => "invalid_name",
            CoreError::ProjectNotFound(_) => "project_not_found",
            CoreError::ProjectExists(_) => "project_exists",
            CoreError::TrackNotFound(_) => "track_not_found",
            CoreError::TempoOutOfRange(_) => "invalid_tempo",
            CoreError::InvalidTimeSignature(..) => "invalid_time_signature",
            CoreError::InvalidPpq(_) => "invalid_ppq",
            CoreError::PitchOutOfRange(_) | CoreError::ChannelOutOfRange(_) | CoreError::ZeroDuration => {
                "invalid_note"
            }
            CoreError::SchemaTooNew(..) => "schema_too_new",
            CoreError::Io { .. } => "io",
            CoreError::Json { .. } => "corrupt_project",
            CoreError::MidiParse { .. } | CoreError::MidiWrite(_) => "midi",
            CoreError::Invalid(_) => "invalid_request",
        };

        CommandError {
            code,
            message: err.to_string(),
        }
    }
}

impl From<unplugged_ai::AiError> for CommandError {
    fn from(err: unplugged_ai::AiError) -> Self {
        use unplugged_ai::AiError;

        // `no_api_key` is the one the frontend branches on: it is not a failure so much
        // as a setup step, and the AI panel offers to open Settings rather than showing
        // an error.
        let code = match err {
            AiError::NoApiKey => "no_api_key",
            AiError::Keychain(_) => "keychain",
            AiError::Transport(_) => "network",
            AiError::Api { .. } => "api",
            AiError::Protocol(_) => "protocol",
            AiError::ToolLimit(_) => "tool_limit",
            AiError::EmptyPrompt => "empty_prompt",
            AiError::Core(core) => return CommandError::from(core),
        };

        CommandError {
            code,
            message: err.to_string(),
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        CommandError {
            code: "internal",
            message,
        }
    }
}

pub type CommandResult<T> = std::result::Result<T, CommandError>;
