//! Phase 6 commands: the AI panel, the key, and the model picker.
//!
//! Two things this module is careful about, both of them requirements rather than
//! preferences:
//!
//! * **The key never crosses into the webview.** `ai_set_key` takes one and hands it to
//!   the Keychain; nothing here ever sends one back. The frontend can learn that a key
//!   exists and see its last four characters, and that is all.
//! * **The proposal stays in Rust.** The model's work becomes a `Transaction`, and a
//!   `Transaction` is raw `Command`s — exactly the thing `EditRequest` exists to keep
//!   the webview from constructing. So the proposal is held here and the frontend gets
//!   only a diff to look at plus `ai_accept` / `ai_reject` to decide with.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;
use unplugged_core::ai::{AiContext, NoteDiff};
use unplugged_core::command::Transaction;
use unplugged_core::music::Key;
use unplugged_core::Note;
use unplugged_ai::{keychain, EditRequest, ModelInfo, ToolStep, Usage};

use crate::editor::EditorState;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// A proposal waiting for the user to accept or reject it.
pub struct PendingProposal {
    pub track: usize,
    pub transaction: Transaction,
    /// The track's notes when the proposal was made.
    ///
    /// A transaction addresses notes by index, so any edit in between — a drag, an undo,
    /// a recorded take — would make it apply to the wrong notes. Comparing against this
    /// turns that from silent corruption into a refusal.
    pub base: Vec<Note>,
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// The model choice. Not a secret, so it lives in a plain file next to the projects.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiPreferences {
    #[serde(default)]
    pub model: Option<String>,
}

impl AiPreferences {
    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("ai.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        std::fs::read(Self::path(data_dir))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, data_dir: &Path) {
        // Best effort. Losing a model preference is not worth failing a command over,
        // and there is nowhere useful to report it to.
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(Self::path(data_dir), json);
        }
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct AiStatus {
    pub has_key: bool,
    /// Last four characters only — never the key.
    pub key_hint: Option<String>,
    /// False on a build with no Keychain, where a key lasts until the app quits.
    pub key_persists: bool,
    pub model: Option<String>,
}

fn status(state: &AppState) -> AiStatus {
    AiStatus {
        has_key: keychain::has_api_key(),
        key_hint: keychain::key_hint(),
        key_persists: keychain::is_persistent(),
        model: state
            .ai_prefs
            .lock()
            .ok()
            .and_then(|prefs| prefs.model.clone()),
    }
}

#[tauri::command]
pub fn ai_status(state: State<'_, AppState>) -> CommandResult<AiStatus> {
    Ok(status(&state))
}

/// Verify a key against the API, then store it in the Keychain.
///
/// Verified before it is stored so a typo is reported here rather than the first time
/// someone tries to edit. The models come back from the same call, so the picker is
/// populated without a second round trip.
#[tauri::command]
pub async fn ai_set_key(
    state: State<'_, AppState>,
    key: String,
) -> CommandResult<AiModelsResponse> {
    let models = tauri::async_runtime::spawn_blocking(move || {
        let models = unplugged_ai::verify_key(&key)?;
        keychain::set_api_key(key.trim())?;
        Ok::<_, unplugged_ai::AiError>(models)
    })
    .await
    .map_err(|e| CommandError::from(format!("the key check did not finish: {e}")))??;

    let default = unplugged_ai::anthropic::default_model(&models);

    if let Ok(mut prefs) = state.ai_prefs.lock() {
        if prefs.model.is_none() {
            prefs.model = default.clone();
            prefs.save(&state.data_dir);
        }
    }

    Ok(AiModelsResponse { models, default })
}

#[tauri::command]
pub fn ai_clear_key(state: State<'_, AppState>) -> CommandResult<AiStatus> {
    keychain::clear_api_key().map_err(|e| CommandError::from(e.to_string()))?;
    // The pending proposal was made with that key; leaving it around to be accepted
    // afterwards would be confusing rather than dangerous, but it is still stale.
    if let Ok(mut pending) = state.pending_ai.lock() {
        *pending = None;
    }
    Ok(status(&state))
}

#[derive(Debug, Serialize)]
pub struct AiModelsResponse {
    pub models: Vec<ModelInfo>,
    pub default: Option<String>,
}

/// Fetch the model list from `GET /v1/models`.
///
/// Live rather than hardcoded, as the spec requires: a baked-in list is wrong the week a
/// model ships, and which models a given key can reach is not knowable from here.
#[tauri::command]
pub async fn ai_models() -> CommandResult<AiModelsResponse> {
    let (models, default) = tauri::async_runtime::spawn_blocking(unplugged_ai::available_models)
        .await
        .map_err(|e| CommandError::from(format!("the model list did not finish: {e}")))??;

    Ok(AiModelsResponse { models, default })
}

#[tauri::command]
pub fn ai_set_model(state: State<'_, AppState>, model: String) -> CommandResult<AiStatus> {
    if let Ok(mut prefs) = state.ai_prefs.lock() {
        prefs.model = Some(model);
        prefs.save(&state.data_dir);
    }
    Ok(status(&state))
}

// ---------------------------------------------------------------------------
// The edit loop
// ---------------------------------------------------------------------------

/// What the frontend sees of a proposal. Note the absence of the transaction.
#[derive(Debug, Serialize)]
pub struct AiProposal {
    pub diff: NoteDiff,
    /// The track as it would be, so the roll can draw the preview without applying it.
    pub preview_notes: Vec<Note>,
    pub narration: String,
    pub steps: Vec<ToolStep>,
    pub usage: Usage,
    /// The model ran out of turns rather than finishing.
    pub truncated: bool,
    /// "3 added, 1 changed", or "No change".
    pub summary: String,
    /// True when there is nothing to accept.
    pub empty: bool,
}

#[tauri::command]
pub async fn ai_propose(
    state: State<'_, AppState>,
    track: usize,
    prompt: String,
    selection: Vec<usize>,
) -> CommandResult<AiProposal> {
    let model = state
        .ai_prefs
        .lock()
        .ok()
        .and_then(|prefs| prefs.model.clone())
        .ok_or_else(|| CommandError {
            code: "no_model",
            message: "choose a model in Settings → AI first".to_string(),
        })?;

    // Snapshot everything the loop needs, then drop the lock: the request takes seconds
    // and holding the editor lock across it would freeze every other command.
    let (context, notes, track_name) = {
        let guard = state
            .open
            .lock()
            .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))?;
        let open = guard
            .as_ref()
            .ok_or_else(|| CommandError::from("no project is open".to_string()))?;
        let target = open
            .tracks()
            .get(track)
            .ok_or_else(|| CommandError::from(format!("no track at index {track}")))?;

        (
            AiContext {
                ppq: open.manifest.ppq,
                time_signature: open.manifest.time_signature,
                tempo_bpm: open.manifest.tempo_bpm,
                channel: target.meta.channel,
                // The track's key hint is what makes "harmonise a third above" work
                // without the user restating the key in every prompt.
                key: target.meta.key_hint.as_deref().and_then(Key::parse),
            },
            target.notes.clone(),
            target.meta.name.clone(),
        )
    };

    let base: Vec<Note> = notes.clone();
    let proposal = tauri::async_runtime::spawn_blocking(move || {
        unplugged_ai::propose_edit(
            track,
            &EditRequest {
                prompt: &prompt,
                model: &model,
                context,
                notes: &notes,
                selection: &selection,
                track_name: &track_name,
            },
        )
    })
    .await
    .map_err(|e| CommandError::from(format!("the edit did not finish: {e}")))??;

    let empty = proposal.diff.is_empty();
    let summary = proposal.diff.summary();

    {
        let mut pending = state
            .pending_ai
            .lock()
            .map_err(|_| CommandError::from("the proposal lock was poisoned".to_string()))?;
        *pending = if empty {
            None
        } else {
            Some(PendingProposal {
                track,
                transaction: proposal.transaction,
                base,
            })
        };
    }

    Ok(AiProposal {
        diff: proposal.diff,
        preview_notes: proposal.preview_notes,
        narration: proposal.narration,
        steps: proposal.steps,
        usage: proposal.usage,
        truncated: proposal.truncated,
        summary,
        empty,
    })
}

/// Apply the pending proposal as one undoable transaction.
#[tauri::command]
pub fn ai_accept(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let proposal = {
        let mut pending = state
            .pending_ai
            .lock()
            .map_err(|_| CommandError::from("the proposal lock was poisoned".to_string()))?;
        pending
            .take()
            .ok_or_else(|| CommandError::from("there is no proposal to apply".to_string()))?
    };

    let mut guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))?;
    let open = guard
        .as_mut()
        .ok_or_else(|| CommandError::from("no project is open".to_string()))?;

    let current = open
        .tracks()
        .get(proposal.track)
        .ok_or_else(|| CommandError::from("that track no longer exists".to_string()))?;

    if current.notes != proposal.base {
        return Err(CommandError {
            code: "stale_proposal",
            message: "the track changed while the suggestion was being prepared — ask again"
                .to_string(),
        });
    }

    let outcome = open.session.apply(proposal.transaction)?;
    open.dirty = true;
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

/// Throw the pending proposal away.
#[tauri::command]
pub fn ai_reject(state: State<'_, AppState>) -> CommandResult<()> {
    let mut pending = state
        .pending_ai
        .lock()
        .map_err(|_| CommandError::from("the proposal lock was poisoned".to_string()))?;
    *pending = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_survive_a_round_trip_and_tolerate_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("unplugged-ai-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Nothing written yet: the default is no model, not a failure.
        assert!(AiPreferences::load(&dir).model.is_none());

        let prefs = AiPreferences { model: Some("claude-sonnet-5".into()) };
        prefs.save(&dir);
        assert_eq!(
            AiPreferences::load(&dir).model.as_deref(),
            Some("claude-sonnet-5")
        );

        // A corrupt file falls back to the default rather than refusing to start.
        std::fs::write(dir.join("ai.json"), b"{not json").unwrap();
        assert!(AiPreferences::load(&dir).model.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
