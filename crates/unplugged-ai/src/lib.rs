//! Phase 6: AI-assisted editing.
//!
//! The whole feature is one function, [`propose_edit`]. It runs an Anthropic tool-use
//! loop against a scratch copy of a track and returns a proposal — a transaction plus a
//! diff — which the caller shows the user and then applies or throws away. Nothing here
//! mutates a project.
//!
//! Two properties are load-bearing and worth stating plainly:
//!
//! * **The API key never leaves this crate.** It is read from the Keychain immediately
//!   before a request and dropped after; it is never returned to `src-tauri`, never
//!   serialised, and never sent to the webview. The only thing the UI can learn about it
//!   is whether one exists and its last four characters.
//! * **The model cannot edit anything directly.** It emits tool calls that are parsed
//!   into a closed `ToolCall` enum in `unplugged-core`; anything it invents fails at that
//!   boundary and comes back to it as an error. The resulting notes go through the same
//!   command layer as a mouse drag.

pub mod anthropic;
pub mod error;
pub mod keychain;

use serde::Serialize;
use serde_json::{json, Value};

use unplugged_core::ai::{self, AiContext, ToolCall, Workspace};
use unplugged_core::command::Transaction;
use unplugged_core::Note;

pub use anthropic::{ModelInfo, Usage};
pub use error::{AiError, Result};

/// How many assistant turns the loop will take before giving up.
///
/// Each turn may contain several tool calls, so this is not a cap on tools — it is a cap
/// on round trips, which is what actually costs time and money.
const MAX_TURNS: usize = 12;

/// Hard cap on tool calls across the whole conversation.
const MAX_TOOL_CALLS: usize = 48;

const MAX_TOKENS: u32 = 8192;

/// Notes included in the model's view of the track.
///
/// A longer track is described by its first thousand notes plus a count. The model can
/// still act on the rest through `select_notes`, which works on ranges rather than on
/// what it can see.
const NOTES_IN_CONTEXT: usize = 1000;

/// What the caller asks for.
pub struct EditRequest<'a> {
    pub prompt: &'a str,
    pub model: &'a str,
    pub context: AiContext,
    /// The notes the tools operate on. Empty when writing into a new track.
    pub notes: &'a [Note],
    /// The editor's current selection, as indices into `notes`.
    pub selection: &'a [usize],
    pub track_name: &'a str,
    /// A track shown to the model for context but which it cannot touch.
    ///
    /// This is what makes "add a bass line under this" expressible. The tools are
    /// single-track by design — that is what keeps the diff and the transaction simple —
    /// so writing a *new* part means an empty workspace plus the existing part as
    /// something to write against. Without it the model would be composing blind.
    pub reference: Option<(&'a str, &'a [Note])>,
}

/// One tool call and what came back, for the console.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStep {
    pub tool: String,
    pub input: Value,
    pub result: String,
    pub ok: bool,
}

/// A proposed edit, not yet applied to anything.
#[derive(Debug, Clone, Serialize)]
pub struct EditProposal {
    /// Apply this — or do not — through the normal command layer.
    pub transaction: Transaction,
    pub diff: ai::NoteDiff,
    /// The track as it would be. Lets the piano roll draw the preview without
    /// speculatively applying anything.
    pub preview_notes: Vec<Note>,
    /// What the model said, for the user to read.
    pub narration: String,
    pub steps: Vec<ToolStep>,
    pub usage: Usage,
    /// True when the loop hit its turn limit rather than the model finishing.
    pub truncated: bool,
}

/// The instructions the model works under.
///
/// Written to constrain rather than to encourage: it has one track, a tool list, and a
/// user who will see a diff before anything happens. Telling it that last part matters —
/// a model that believes its edits are final is markedly more timid than is useful here.
fn system_prompt(request: &EditRequest<'_>) -> String {
    let context = &request.context;
    let key = context
        .key
        .map(|key| key.name())
        .unwrap_or_else(|| "not set".to_string());

    format!(
        "You are the editing assistant inside Unplugged, a MIDI sequencer. You are \
         working on one track and you change it only by calling the tools provided.\n\n\
         Track: \"{track}\"\n\
         Tempo: {tempo:.0} bpm\n\
         Time signature: {num}/{den}\n\
         Resolution: {ppq} ticks per quarter note, {bar} ticks per bar\n\
         Key: {key}\n\
         Notes selected in the editor: {selected}\n\n\
         How to work:\n\
         - Tools act on the current selection. It starts as the user's editor selection, \
           or the whole track if they had nothing selected. Call select_notes to change it.\n\
         - Prefer the musical tools over insert_notes. \"Make it swing\" is quantize with \
           a triplet grid, not a hand-written note list.\n\
         - Do the smallest thing that satisfies the request. If the user asks to transpose, \
           transpose; do not also quantize because it looked untidy.\n\
         - If a request is ambiguous, take the most common reading and say which you took \
           in your final message. Do not ask a question — there is no way for the user to \
           answer mid-edit.\n\
         - If a request is impossible with these tools, say so plainly and change nothing.\n\n\
         The user sees a diff of everything you changed and accepts or rejects it as one \
         step, so a wrong guess costs them a click. Finish with one or two sentences \
         describing what you did, in a musician's terms rather than a programmer's.\n\n\
         {reference}The track you are editing, in full:\n{table}",
        track = request.track_name,
        tempo = context.tempo_bpm,
        num = context.time_signature.numerator,
        den = context.time_signature.denominator,
        ppq = context.ppq,
        bar = context.bar_ticks(),
        selected = if request.selection.is_empty() {
            "none — tools will act on the whole track".to_string()
        } else {
            request.selection.len().to_string()
        },
        table = ai::notes_table(request.notes, context, NOTES_IN_CONTEXT),
        reference = match request.reference {
            Some((name, notes)) if !notes.is_empty() => format!(
                "You are writing into a NEW, empty track. The existing track \"{name}\" is \
                 shown below for reference — write something that works against it. You \
                 cannot change it; your tools only affect the new track.\n\n\
                 \"{name}\", for reference only:\n{}\n\n",
                ai::notes_table(notes, context, NOTES_IN_CONTEXT),
            ),
            _ => String::new(),
        },
    )
}

/// Run the tool loop and return a proposal. Nothing is applied.
///
/// Blocking: this makes several HTTPS round trips and is expected to be called from a
/// worker thread, never from a UI or audio thread.
pub fn propose_edit(track: usize, request: &EditRequest<'_>) -> Result<EditProposal> {
    if request.prompt.trim().is_empty() {
        return Err(AiError::EmptyPrompt);
    }

    let api_key = match keychain::api_key() {
        Ok(key) => key,
        Err(keychain::KeychainError::Missing) => return Err(AiError::NoApiKey),
        Err(error) => return Err(error.into()),
    };

    let system = system_prompt(request);
    let tools = ai::tool_definitions(&request.context);
    let mut workspace = Workspace::new(request.context, request.notes, request.selection);

    let mut messages: Vec<Value> = vec![json!({
        "role": "user",
        "content": request.prompt.trim(),
    })];

    let mut steps: Vec<ToolStep> = Vec::new();
    let mut usage = Usage::default();
    let mut narration = String::new();
    let mut truncated = true;

    for _turn in 0..MAX_TURNS {
        let body = anthropic::send_message(
            &api_key,
            request.model,
            &system,
            &messages,
            &tools,
            MAX_TOKENS,
        )?;
        let reply = anthropic::parse_reply(&body)?;
        usage.add(&reply.usage);

        if !reply.text.is_empty() {
            narration = reply.text.clone();
        }

        if reply.tool_uses.is_empty() {
            truncated = false;
            break;
        }

        if steps.len() + reply.tool_uses.len() > MAX_TOOL_CALLS {
            return Err(AiError::ToolLimit(steps.len() + reply.tool_uses.len()));
        }

        // Echoed verbatim: thinking blocks carry signatures that any reconstruction of
        // the content array would invalidate.
        messages.push(json!({"role": "assistant", "content": reply.content}));

        let mut results = Vec::with_capacity(reply.tool_uses.len());
        for use_block in &reply.tool_uses {
            let (text, ok) = run_tool(&mut workspace, &use_block.name, &use_block.input);

            steps.push(ToolStep {
                tool: use_block.name.clone(),
                input: use_block.input.clone(),
                result: text.clone(),
                ok,
            });

            results.push(json!({
                "type": "tool_result",
                "tool_use_id": use_block.id,
                "content": text,
                // A failed tool comes back as an error the model can read and correct,
                // not as a dead conversation. Half of what makes the loop usable is that
                // "1/7 is not a note value" is something it can act on.
                "is_error": !ok,
            }));
        }

        messages.push(json!({"role": "user", "content": results}));
    }

    let diff = workspace.diff();
    let transaction = workspace.to_transaction(track, edit_label(request.prompt));

    Ok(EditProposal {
        transaction,
        diff,
        preview_notes: {
            let mut notes = workspace.notes();
            notes.sort_by_key(Note::order_key);
            notes
        },
        narration,
        steps,
        usage,
        truncated,
    })
}

/// Dispatch one tool call, turning both parse failures and execution failures into text
/// the model can read.
fn run_tool(workspace: &mut Workspace, name: &str, input: &Value) -> (String, bool) {
    let call: ToolCall = match serde_json::from_value(json!({"name": name, "input": input})) {
        Ok(call) => call,
        Err(error) => {
            return (
                format!("Could not read the call to \"{name}\": {error}"),
                false,
            )
        }
    };

    match workspace.apply(&call) {
        Ok(summary) => (summary, true),
        Err(error) => (error.to_string(), false),
    }
}

/// Undo-stack label. The user's own words, trimmed to fit a menu item.
fn edit_label(prompt: &str) -> String {
    const MAX: usize = 40;
    let prompt = prompt.trim();
    let first_line = prompt.lines().next().unwrap_or(prompt).trim();

    if first_line.chars().count() <= MAX {
        return format!("AI: {first_line}");
    }
    let short: String = first_line.chars().take(MAX - 1).collect();
    format!("AI: {}…", short.trim_end())
}

/// The models this key can use, newest first, plus the one to select by default.
pub fn available_models() -> Result<(Vec<ModelInfo>, Option<String>)> {
    let api_key = match keychain::api_key() {
        Ok(key) => key,
        Err(keychain::KeychainError::Missing) => return Err(AiError::NoApiKey),
        Err(error) => return Err(error.into()),
    };

    let models = anthropic::list_models(&api_key)?;
    let default = anthropic::default_model(&models);
    Ok((models, default))
}

/// Check a key by using it, before it is stored.
///
/// `GET /v1/models` is the cheapest authenticated call there is, so a bad key is
/// reported when it is typed rather than the first time an edit is attempted.
pub fn verify_key(candidate: &str) -> Result<Vec<ModelInfo>> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err(AiError::NoApiKey);
    }
    anthropic::list_models(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unplugged_core::music::{Key, Scale};
    use unplugged_core::TimeSignature;

    fn context() -> AiContext {
        AiContext {
            ppq: 480,
            time_signature: TimeSignature::new(4, 4).unwrap(),
            tempo_bpm: 120.0,
            channel: 0,
            key: Some(Key::new(0, Scale::Major)),
        }
    }

    fn request<'a>(prompt: &'a str, notes: &'a [Note], selection: &'a [usize]) -> EditRequest<'a> {
        EditRequest {
            prompt,
            model: "claude-sonnet-5",
            context: context(),
            notes,
            selection,
            track_name: "Piano",
            reference: None,
        }
    }

    #[test]
    fn an_empty_prompt_never_reaches_the_network() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let error = propose_edit(0, &request("   ", &notes, &[])).unwrap_err();
        assert!(matches!(error, AiError::EmptyPrompt));
    }

    #[test]
    fn the_system_prompt_carries_the_project_and_the_notes() {
        let notes = vec![
            Note::new(60, 0, 480, 96, 0).unwrap(),
            Note::new(64, 480, 480, 96, 0).unwrap(),
        ];
        let prompt = system_prompt(&request("make it swing", &notes, &[1]));

        assert!(prompt.contains("Piano"));
        assert!(prompt.contains("120 bpm"));
        assert!(prompt.contains("4/4"));
        assert!(prompt.contains("480 ticks per quarter"));
        assert!(prompt.contains("1920 ticks per bar"));
        assert!(prompt.contains("C major"));
        assert!(prompt.contains("C4") && prompt.contains("E4"), "the notes table");
        // The key itself must never be anywhere near this string.
        assert!(!prompt.contains("sk-ant"));
    }

    #[test]
    fn a_reference_track_is_shown_but_marked_untouchable() {
        let melody = vec![
            Note::new(72, 0, 480, 96, 0).unwrap(),
            Note::new(74, 480, 480, 96, 0).unwrap(),
        ];
        let mut request = request("add a bass line under this", &[], &[]);
        request.reference = Some(("Melody", &melody));

        let prompt = system_prompt(&request);
        assert!(prompt.contains("NEW, empty track"));
        assert!(prompt.contains("Melody"));
        assert!(prompt.contains("C5"), "the reference notes are listed");
        assert!(prompt.contains("cannot change it"), "and marked read-only");
    }

    #[test]
    fn no_reference_means_no_mention_of_one() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let prompt = system_prompt(&request("quantize", &notes, &[]));
        assert!(!prompt.contains("NEW, empty track"));
        assert!(!prompt.contains("reference"));
    }

    #[test]
    fn an_empty_selection_is_described_as_the_whole_track() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let prompt = system_prompt(&request("quantize", &notes, &[]));
        assert!(prompt.contains("whole track"), "{prompt}");
    }

    #[test]
    fn undo_labels_are_the_users_words_kept_short() {
        assert_eq!(edit_label("transpose up a fifth"), "AI: transpose up a fifth");
        assert_eq!(edit_label("first line\nsecond line"), "AI: first line");

        let long = edit_label(
            "make this passage considerably more interesting rhythmically than it is now",
        );
        assert!(long.chars().count() <= 45, "{long}");
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_tool_the_model_invented_comes_back_as_a_readable_error() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let mut workspace = Workspace::new(context(), &notes, &[]);

        let (text, ok) = run_tool(&mut workspace, "delete_the_project", &json!({}));
        assert!(!ok);
        assert!(text.contains("delete_the_project"), "{text}");
        assert_eq!(workspace.notes(), notes, "nothing was touched");
    }

    #[test]
    fn a_tool_with_bad_arguments_comes_back_as_a_readable_error() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let mut workspace = Workspace::new(context(), &notes, &[]);

        let (text, ok) = run_tool(&mut workspace, "transpose", &json!({"semitones": "a lot"}));
        assert!(!ok);
        assert!(text.contains("transpose"), "{text}");
        assert_eq!(workspace.notes(), notes);
    }

    #[test]
    fn a_valid_tool_runs_and_reports_what_it_did() {
        let notes = vec![Note::new(60, 0, 480, 96, 0).unwrap()];
        let mut workspace = Workspace::new(context(), &notes, &[]);

        let (text, ok) = run_tool(&mut workspace, "transpose", &json!({"semitones": 7}));
        assert!(ok, "{text}");
        assert_eq!(workspace.notes()[0].pitch, 67);
        assert!(text.contains("Transposed"), "{text}");
    }
}
