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
use unplugged_ai::{keychain, EditRequest, Endpoint, ModelInfo, Provider, ToolStep, Usage};

use crate::editor::EditorState;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// Where an AI edit is aimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiTarget {
    /// Change the track the user is looking at.
    ThisTrack,
    /// Write a new part alongside it, with the current track as read-only context.
    NewTrack,
}

/// A proposal waiting for the user to accept or reject it.
pub struct PendingProposal {
    pub track: usize,
    /// Set when the proposal targets a track that does not exist yet; it is created at
    /// accept time, not before, so a rejected suggestion leaves no empty track behind.
    pub new_track_name: Option<String>,
    pub transaction: Transaction,
    /// The track's notes when the proposal was made.
    ///
    /// A transaction addresses notes by index, so any edit in between — a drag, an undo,
    /// a recorded take — would make it apply to the wrong notes. Comparing against this
    /// turns that from silent corruption into a refusal.
    pub base: Vec<Note>,
    /// The notes on offer: what the track would become. Held here, not only sent to the
    /// frontend, because they are what plays when the proposal is auditioned and what a
    /// hand adjustment replaces.
    pub preview: Vec<Note>,
    /// True once the user has moved something. The model's own transaction is a minimal
    /// diff and is kept while it is still accurate; an adjusted proposal no longer
    /// matches it and is applied as a replacement instead.
    pub edited: bool,
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// Which service, which model, and where it lives. Not secret, so it is a plain file
/// next to the projects — the key is the only thing here that belongs in the Keychain,
/// and it is not here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiPreferences {
    #[serde(default)]
    pub provider: Provider,
    /// The model chosen for each provider, kept apart: a local model id means nothing to
    /// Anthropic, and switching back and forth should not lose either choice.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub local_model: Option<String>,
    /// Where the local server is. Empty means the LM Studio default.
    #[serde(default)]
    pub local_url: String,
}

impl AiPreferences {
    /// The model for the provider in use.
    fn current_model(&self) -> Option<String> {
        match self.provider {
            Provider::Anthropic => self.model.clone(),
            Provider::LmStudio => self.local_model.clone(),
        }
    }

    fn set_current_model(&mut self, model: String) {
        match self.provider {
            Provider::Anthropic => self.model = Some(model),
            Provider::LmStudio => self.local_model = Some(model),
        }
    }

    fn base_url(&self) -> String {
        unplugged_ai::provider::normalise(&self.local_url)
    }

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
    pub provider: Provider,
    /// The model for the provider in use, not for whichever was configured last.
    pub model: Option<String>,
    pub local_url: String,
    /// False when the chosen provider still needs something before it can be asked:
    /// a key for Anthropic, a reachable server for LM Studio.
    pub ready: bool,
}

fn status(state: &AppState) -> AiStatus {
    let prefs = state.ai_prefs.lock().ok();
    let (provider, model, local_url) = prefs
        .as_ref()
        .map(|p| (p.provider, p.current_model(), p.base_url()))
        .unwrap_or_default();

    let has_key = keychain::has_api_key();
    AiStatus {
        has_key,
        key_hint: keychain::key_hint(),
        key_persists: keychain::is_persistent(),
        provider,
        // A local provider is ready as soon as a model is picked; whether the server is
        // actually up is answered by asking it, not by guessing here.
        ready: model.is_some() && (!provider.needs_api_key() || has_key),
        model,
        local_url,
    }
}

/// The endpoint for the provider in use, with the key read only if one is needed.
///
/// This is the *only* place outside a request that touches the Keychain, and it is
/// immediately before one.
fn endpoint(state: &AppState) -> CommandResult<(Endpoint, Option<String>)> {
    let (provider, model, base_url) = {
        let prefs = state
            .ai_prefs
            .lock()
            .map_err(|_| CommandError::from("the AI settings lock was poisoned".to_string()))?;
        (prefs.provider, prefs.current_model(), prefs.base_url())
    };

    let endpoint = match provider {
        Provider::Anthropic => Endpoint::anthropic(keychain::api_key().map_err(|_| CommandError {
            code: "no_key",
            message: "add an Anthropic API key in Settings → AI first".to_string(),
        })?),
        Provider::LmStudio => Endpoint::lm_studio(base_url),
    };

    Ok((endpoint, model))
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
        // Entering a key is a statement about which provider you mean.
        prefs.provider = Provider::Anthropic;
        if prefs.model.is_none() {
            prefs.model = default.clone();
        }
        prefs.save(&state.data_dir);
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

/// Fetch the model list from the provider in use.
///
/// Live rather than hardcoded, as the spec requires: a baked-in list is wrong the week a
/// model ships, which models a given key can reach is not knowable from here, and for a
/// local server "what is loaded" is a question only the server can answer.
#[tauri::command]
pub async fn ai_models(state: State<'_, AppState>) -> CommandResult<AiModelsResponse> {
    let (endpoint, _) = endpoint(&state)?;
    let (models, default) =
        tauri::async_runtime::spawn_blocking(move || unplugged_ai::available_models(&endpoint))
            .await
            .map_err(|e| CommandError::from(format!("the model list did not finish: {e}")))??;

    Ok(AiModelsResponse { models, default })
}

#[tauri::command]
pub fn ai_set_model(state: State<'_, AppState>, model: String) -> CommandResult<AiStatus> {
    if let Ok(mut prefs) = state.ai_prefs.lock() {
        prefs.set_current_model(model);
        prefs.save(&state.data_dir);
    }
    Ok(status(&state))
}

/// Choose a provider, and where a local one lives.
///
/// The URL is stored even when Anthropic is selected: a user who set up LM Studio, went
/// back to the cloud, and returned should not have to type it again.
#[tauri::command]
pub fn ai_set_provider(
    state: State<'_, AppState>,
    provider: Provider,
    local_url: String,
) -> CommandResult<AiStatus> {
    if let Ok(mut prefs) = state.ai_prefs.lock() {
        prefs.provider = provider;
        prefs.local_url = unplugged_ai::provider::normalise(&local_url);
        prefs.save(&state.data_dir);
    }
    // A proposal made against the previous provider is still applicable — it is only
    // notes — so it deliberately survives this.
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
    target: AiTarget,
) -> CommandResult<AiProposal> {
    let (endpoint, model) = endpoint(&state)?;
    let model = model.ok_or_else(|| CommandError {
        code: "no_model",
        message: "choose a model in Settings → AI first".to_string(),
    })?;

    // Snapshot everything the loop needs, then drop the lock: the request takes seconds
    // and holding the editor lock across it would freeze every other command.
    let (context, notes, track_name, reference, target_index) = {
        let guard = state
            .open
            .lock()
            .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))?;
        let open = guard
            .as_ref()
            .ok_or_else(|| CommandError::from("no project is open".to_string()))?;
        let source = open
            .tracks()
            .get(track)
            .ok_or_else(|| CommandError::from(format!("no track at index {track}")))?;

        // A new part is written into an empty workspace with the visible track supplied
        // as context, and lands at the index the track will occupy once it is created.
        let (notes, reference, index) = match target {
            AiTarget::ThisTrack => (source.notes.clone(), None, track),
            AiTarget::NewTrack => (
                Vec::new(),
                Some((source.meta.name.clone(), source.notes.clone())),
                open.tracks().len(),
            ),
        };

        (
            AiContext {
                ppq: open.manifest.ppq,
                time_signature: open.manifest.time_signature,
                tempo_bpm: open.manifest.tempo_bpm,
                channel: source.meta.channel,
                // The track's key hint is what makes "harmonise a third above" work
                // without the user restating the key in every prompt.
                key: source.meta.key_hint.as_deref().and_then(Key::parse),
            },
            notes,
            source.meta.name.clone(),
            reference,
            index,
        )
    };

    let base: Vec<Note> = notes.clone();
    let prompt_for_name = prompt.clone();
    let proposal = tauri::async_runtime::spawn_blocking(move || {
        unplugged_ai::propose_edit(
            target_index,
            &EditRequest {
                prompt: &prompt,
                endpoint: &endpoint,
                model: &model,
                context,
                notes: &notes,
                selection: &selection,
                track_name: &track_name,
                reference: reference
                    .as_ref()
                    .map(|(name, notes)| (name.as_str(), notes.as_slice())),
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
                track: target_index,
                new_track_name: match target {
                    AiTarget::NewTrack => Some(new_track_name(&prompt_for_name)),
                    AiTarget::ThisTrack => None,
                },
                transaction: proposal.transaction,
                base,
                preview: proposal.preview_notes.clone(),
                edited: false,
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

/// What changes when the user adjusts a proposal by hand.
///
/// Only the parts that can change: the narration, the tool steps and the token counts
/// describe how the proposal was arrived at, and moving a note does not revise history.
#[derive(Debug, Serialize)]
pub struct AiEdit {
    pub diff: NoteDiff,
    pub preview_notes: Vec<Note>,
    pub summary: String,
    pub empty: bool,
}

/// Replace the notes a proposal is offering.
///
/// Validated rather than trusted, like every other note that arrives from the webview.
/// The diff is recomputed here because the one that came with the proposal describes the
/// model's work, and after an adjustment that is no longer what is on offer.
#[tauri::command]
pub fn ai_set_notes(state: State<'_, AppState>, notes: Vec<Note>) -> CommandResult<AiEdit> {
    for note in &notes {
        note.validate()?;
    }
    let mut notes = notes;
    notes.sort_by_key(Note::order_key);

    let mut guard = state
        .pending_ai
        .lock()
        .map_err(|_| CommandError::from("the proposal lock was poisoned".to_string()))?;
    let pending = guard
        .as_mut()
        .ok_or_else(|| CommandError::from("there is no proposal to adjust".to_string()))?;

    let diff = unplugged_core::diff::between(&pending.base, &notes);
    pending.preview = notes.clone();
    pending.edited = true;

    Ok(AiEdit {
        summary: diff.summary(),
        empty: diff.is_empty(),
        diff,
        preview_notes: notes,
    })
}

/// Name a track after the request that produced it.
///
/// The user's own words, trimmed to something that fits a track list. Better than
/// "Track 3": in an app where parts are described rather than played, what a part *is*
/// is usually exactly what was asked for.
fn new_track_name(prompt: &str) -> String {
    const MAX: usize = 24;
    let first_line = prompt.trim().lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "New part".to_string();
    }
    if first_line.chars().count() <= MAX {
        return first_line.to_string();
    }
    // Cut at a word boundary where there is one nearby, so the label reads as words
    // rather than a truncation.
    let short: String = first_line.chars().take(MAX).collect();
    match short.rfind(' ') {
        Some(space) if space >= MAX / 2 => format!("{}…", &short[..space]),
        _ => format!("{}…", short.trim_end()),
    }
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

    if let Some(name) = &proposal.new_track_name {
        // Created now rather than when the proposal was made, so a rejected suggestion
        // leaves no empty track behind. The index it was written against is the one it
        // gets, which the check below confirms.
        if open.tracks().len() != proposal.track {
            return Err(CommandError {
                code: "stale_proposal",
                message: "the track list changed while the suggestion was being prepared \
                          — ask again"
                    .to_string(),
            });
        }
        open.add_track(name.clone());
        let _ = state.audio.ensure_tracks(open.tracks().len());
    }

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

    let transaction = if proposal.edited {
        // The model's transaction is a minimal diff against the notes it was given, and
        // the user has since moved things it does not know about. Replacing the track's
        // notes wholesale is coarser and correct; it is still one undo step.
        replacement(&proposal)
    } else {
        proposal.transaction
    };

    let outcome = open.session.apply(transaction)?;
    open.dirty = true;
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

/// An adjusted proposal, as a transaction: take out what was there, put in what is on
/// offer. Delete first, because a command's indices address the list as it was.
fn replacement(proposal: &PendingProposal) -> Transaction {
    let mut commands = Vec::new();
    if !proposal.base.is_empty() {
        commands.push(unplugged_core::command::Command::Delete {
            track: proposal.track,
            indices: (0..proposal.base.len()).collect(),
        });
    }
    if !proposal.preview.is_empty() {
        commands.push(unplugged_core::command::Command::Insert {
            track: proposal.track,
            notes: proposal.preview.clone(),
        });
    }
    Transaction::new(proposal.transaction.label.clone(), commands)
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
    fn a_new_track_is_named_after_the_request() {
        assert_eq!(new_track_name("add a bass line"), "add a bass line");
        assert_eq!(new_track_name("  "), "New part");
        assert_eq!(new_track_name("pad chords\nsecond line"), "pad chords");

        let long = new_track_name("a walking bass line under the melody in the same key");
        assert!(long.chars().count() <= 25, "{long}");
        assert!(long.ends_with('…'));
        assert!(!long.contains("  "), "cut at a word, not mid-word: {long}");
    }

    #[test]
    fn preferences_survive_a_round_trip_and_tolerate_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("unplugged-ai-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        // Nothing written yet: the default is no model, not a failure.
        assert!(AiPreferences::load(&dir).model.is_none());

        let prefs = AiPreferences {
            provider: Provider::LmStudio,
            model: Some("claude-sonnet-5".into()),
            local_model: Some("qwen2.5-coder".into()),
            local_url: "http://localhost:1234/v1".into(),
        };
        prefs.save(&dir);

        let loaded = AiPreferences::load(&dir);
        assert_eq!(loaded.provider, Provider::LmStudio);
        assert_eq!(loaded.model.as_deref(), Some("claude-sonnet-5"));
        // Each provider keeps its own choice: switching back must not have to ask again.
        assert_eq!(loaded.current_model().as_deref(), Some("qwen2.5-coder"));

        // A corrupt file falls back to the default rather than refusing to start.
        std::fs::write(dir.join("ai.json"), b"{not json").unwrap();
        assert!(AiPreferences::load(&dir).model.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_settings_file_from_before_providers_existed_still_loads() {
        // The field did not exist when these were written; it must default rather than
        // make the file unreadable, which would silently lose the user's model.
        let prefs: AiPreferences =
            serde_json::from_str(r#"{"model":"claude-sonnet-5"}"#).unwrap();
        assert_eq!(prefs.provider, Provider::Anthropic);
        assert_eq!(prefs.current_model().as_deref(), Some("claude-sonnet-5"));
        assert_eq!(prefs.base_url(), unplugged_ai::LM_STUDIO_DEFAULT_URL);
    }

    #[test]
    fn the_model_follows_the_provider_in_use() {
        let mut prefs = AiPreferences::default();
        prefs.set_current_model("claude-sonnet-5".into());
        prefs.provider = Provider::LmStudio;
        assert_eq!(prefs.current_model(), None, "a cloud model id means nothing locally");

        prefs.set_current_model("qwen2.5-coder".into());
        prefs.provider = Provider::Anthropic;
        assert_eq!(prefs.current_model().as_deref(), Some("claude-sonnet-5"));
    }
}
