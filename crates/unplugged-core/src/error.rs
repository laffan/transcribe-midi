use std::path::PathBuf;

/// Everything the core crate can fail with.
///
/// Kept as one enum rather than per-module errors: the Tauri command layer has to
/// flatten all of it into a string for the frontend anyway, and a single enum means
/// one `impl Display` to keep user-facing.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("project name cannot be empty")]
    EmptyName,

    #[error("project name is too long ({0} characters, maximum {max})", max = crate::model::MAX_NAME_LEN)]
    NameTooLong(usize),

    #[error("no project with id '{0}'")]
    ProjectNotFound(String),

    #[error("a project with id '{0}' already exists")]
    ProjectExists(String),

    #[error("no track with id '{0}'")]
    TrackNotFound(String),

    #[error("tempo {0} bpm is out of range ({min}–{max})", min = crate::model::MIN_TEMPO, max = crate::model::MAX_TEMPO)]
    TempoOutOfRange(f64),

    #[error("invalid time signature {0}/{1}: denominator must be a power of two between 1 and 64")]
    InvalidTimeSignature(u8, u8),

    #[error("invalid PPQ {0}: must be between 24 and 15360")]
    InvalidPpq(u16),

    #[error("MIDI pitch {0} is out of range (0–127)")]
    PitchOutOfRange(u16),

    #[error("MIDI channel {0} is out of range (0–15)")]
    ChannelOutOfRange(u16),

    #[error("note duration must be greater than zero")]
    ZeroDuration,

    #[error("project '{0}' has a newer schema (version {1}); this version of Unplugged understands up to {2}")]
    SchemaTooNew(String, u32, u32),

    #[error("could not read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path} is not valid project JSON: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("could not parse MIDI file {path}: {source}")]
    MidiParse {
        path: PathBuf,
        #[source]
        source: midly::Error,
    },

    #[error("could not write MIDI data: {0}")]
    MidiWrite(#[source] midly::Error),
}

impl CoreError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        CoreError::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
