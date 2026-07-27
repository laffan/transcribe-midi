//! Phase 7 commands: record from the microphone, transcribe, preview, commit.
//!
//! The flow deliberately mirrors the AI panel's, because it is the same shape of
//! interaction: something produces a proposal, the user looks at it, and only then does
//! it become an edit. Notes go in through the command layer as one transaction, so a
//! transcription undoes like anything else.
//!
//! **Monophonic only.** That is stated in the UI, not hidden: given a chord the pitch
//! tracker returns one pitch, and pretending otherwise would produce confident nonsense.
//!
//! Phase 9 changed what happens to the audio. Phase 7 analysed a take and dropped it,
//! which meant every setting was burned in at commit: decide the grid was wrong and your
//! only option was to play it again. The take is now **retained for the session** so the
//! waveform editor has something to draw and so the settings stay re-derivable. It is
//! still not a recording: it is evidence attached to a take, not material in the
//! arrangement — never mixed, bounced, exported, or written to disk.
//!
//! Playing any of it back — the recording, the notes, or both at once — lives in
//! [`crate::audition`], which owns the one place either can be started or stopped.

use std::sync::Mutex;

use serde::Serialize;
use tauri::State;
use unplugged_core::command::{Command, Transaction};
use unplugged_core::Note;
use unplugged_transcribe::{Analysis, DetectedNote, TranscribeOptions, Transcription};

use crate::editor::EditorState;
use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// A take being recorded, or one finished and waiting to be committed.
#[derive(Default)]
pub struct CaptureState {
    pub recording: bool,
    /// Captured mono samples, kept for as long as the take is open.
    ///
    /// In memory only. On-disk persistence is deliberately not here: two minutes at 48 kHz
    /// is ~23 MB, and putting that in the project directory needs a schema bump, a size
    /// budget and a lifecycle for takes whose notes were discarded. Recorded once and
    /// fine-tuned in the same sitting is the shape of the work, so a session-scoped take
    /// buys almost all of the value for none of that.
    pub samples: Vec<f32>,
    pub sample_rate: f64,
    /// The take's analysis, kept alongside the audio so the editor can redraw without
    /// re-running the pipeline.
    pub analysis: Option<Transcription>,
    /// The transcription awaiting accept or reject, and the track it was aimed at.
    pub pending: Option<(usize, Vec<Note>)>,
    /// Settings the current result was derived with, so re-deriving only needs the deltas.
    pub use_project_tempo: bool,
    pub quantize_ticks: u32,
}

pub type SharedCapture = Mutex<CaptureState>;

#[derive(Debug, Serialize)]
pub struct CaptureStatus {
    pub recording: bool,
    pub seconds: f64,
    pub sample_rate: f64,
    /// Peak level since the last poll, 0–1, for the meter.
    pub level: f32,
    /// True once the buffer is full and further audio is being dropped.
    pub at_limit: bool,
}

pub(crate) fn locked(state: &AppState) -> CommandResult<std::sync::MutexGuard<'_, CaptureState>> {
    state
        .capture
        .lock()
        .map_err(|_| CommandError::from("the capture lock was poisoned".to_string()))
}

#[tauri::command]
pub fn capture_start(state: State<'_, AppState>) -> CommandResult<CaptureStatus> {
    state.mic.start().map_err(|error| CommandError {
        code: if unplugged_audio::capture::is_permission_error(&error) {
            "microphone_denied"
        } else {
            "microphone"
        },
        message: error.to_string(),
    })?;

    // Nothing from the previous take may still be sounding over the new one.
    state.audition.stop();
    state.preview.stop();

    let mut capture = locked(&state)?;
    capture.samples.clear();
    capture.sample_rate = state.mic.sample_rate();
    capture.recording = true;
    capture.pending = None;

    Ok(status(&state, &capture))
}

/// Poll while recording: moves newly captured audio into the buffer and reports level.
///
/// Polled rather than pushed because the alternative is the microphone's callback thread
/// emitting an event thirty times a second across the IPC boundary, and nothing here is
/// on a timing path — the samples are timestamped by their position in the buffer, not
/// by when this happens to run.
#[tauri::command]
pub fn capture_poll(state: State<'_, AppState>) -> CommandResult<CaptureStatus> {
    let mut capture = locked(&state)?;
    if capture.recording {
        if capture.sample_rate <= 0.0 {
            capture.sample_rate = state.mic.sample_rate();
        }
        let mut samples = std::mem::take(&mut capture.samples);
        state.mic.drain(&mut samples);
        capture.samples = samples;
    }
    Ok(status(&state, &capture))
}

fn status(state: &AppState, capture: &CaptureState) -> CaptureStatus {
    let sample_rate = if capture.sample_rate > 0.0 {
        capture.sample_rate
    } else {
        state.mic.sample_rate()
    };

    let seconds = if sample_rate > 0.0 {
        capture.samples.len() as f64 / sample_rate
    } else {
        0.0
    };

    CaptureStatus {
        recording: capture.recording,
        seconds,
        sample_rate,
        level: state.mic.take_peak(),
        at_limit: seconds >= unplugged_audio::capture::MAX_CAPTURE_SECONDS,
    }
}

#[derive(Debug, Serialize)]
pub struct TranscriptionPreview {
    pub notes: Vec<DetectedNote>,
    pub tempo_bpm: f64,
    pub tempo_estimated: bool,
    pub tempo_confidence: f32,
    pub duration_seconds: f64,
    pub pitched_fraction: f32,
    /// Set when the recording was too quiet or too unpitched to trust. The UI shows it
    /// rather than silently returning three notes from a room recording.
    pub warning: Option<String>,
    /// The per-frame evidence, for the waveform editor to draw over.
    pub analysis: Analysis,
    /// Settings this result was derived with, so the editor's controls open in the state
    /// that produced what is on screen.
    pub use_project_tempo: bool,
    pub quantize_ticks: u32,
}

impl TranscriptionPreview {
    fn of(result: &Transcription, use_project_tempo: bool, quantize_ticks: u32) -> Self {
        // Thresholds are advisory: the notes are still returned and the user can accept
        // them. Refusing outright would be worse — sometimes a sparse take is exactly
        // what was played.
        let warning = if result.notes.is_empty() {
            Some("No notes were found. Play closer to the microphone, or check the input.".into())
        } else if result.pitched_fraction < 0.15 {
            Some(
                "Most of that recording had no clear pitch — check the result carefully."
                    .to_string(),
            )
        } else {
            None
        };

        TranscriptionPreview {
            notes: result.notes.clone(),
            tempo_bpm: result.tempo_bpm,
            tempo_estimated: result.tempo_estimated,
            tempo_confidence: result.tempo_confidence,
            duration_seconds: result.duration_seconds,
            pitched_fraction: result.pitched_fraction,
            warning,
            analysis: result.analysis.clone(),
            use_project_tempo,
            quantize_ticks,
        }
    }
}

/// Stop recording and transcribe what was captured.
///
/// `use_project_tempo` places the notes against the project's tempo instead of one
/// estimated from the recording — the right choice when the player was following the
/// click, and the wrong one when they were not, which is why it is the caller's decision.
#[tauri::command]
pub async fn capture_transcribe(
    state: State<'_, AppState>,
    track: usize,
    use_project_tempo: bool,
    quantize_ticks: u32,
) -> CommandResult<TranscriptionPreview> {
    state.mic.stop();

    let (samples, sample_rate) = {
        let mut capture = locked(&state)?;
        if capture.recording {
            let mut samples = std::mem::take(&mut capture.samples);
            state.mic.drain(&mut samples);
            capture.samples = samples;
        }
        capture.recording = false;
        (capture.samples.clone(), capture.sample_rate)
    };

    if sample_rate <= 0.0 || samples.is_empty() {
        return Err(CommandError::from(
            "nothing was recorded — check that a microphone is connected".to_string(),
        ));
    }

    transcribe_samples(&state, track, samples, sample_rate, use_project_tempo, quantize_ticks).await
}

/// The shared body: analyse `samples`, store the result as the pending take.
async fn transcribe_samples(
    state: &State<'_, AppState>,
    track: usize,
    samples: Vec<f32>,
    sample_rate: f64,
    use_project_tempo: bool,
    quantize_ticks: u32,
) -> CommandResult<TranscriptionPreview> {
    let (ppq, channel, project_tempo) = {
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
        (open.manifest.ppq, target.meta.channel, open.manifest.tempo_bpm)
    };

    let options = TranscribeOptions {
        sample_rate,
        ppq,
        tempo_bpm: use_project_tempo.then_some(project_tempo),
        quantize_ticks,
        channel,
    };

    // Analysis of a two-minute take is seconds of work, so it runs off the command
    // thread — blocking there would freeze every other command including the transport.
    let result =
        tauri::async_runtime::spawn_blocking(move || unplugged_transcribe::transcribe(&samples, options))
            .await
            .map_err(|e| CommandError::from(format!("transcription did not finish: {e}")))?;

    let preview = TranscriptionPreview::of(&result, use_project_tempo, quantize_ticks);

    {
        let mut capture = locked(state)?;
        // The samples stay. They are what the waveform editor draws and what a change of
        // grid or tempo re-reads, and both of those are the point of this phase.
        capture.pending = if result.notes.is_empty() {
            None
        } else {
            Some((track, result.notes()))
        };
        capture.use_project_tempo = use_project_tempo;
        capture.quantize_ticks = quantize_ticks;
        capture.analysis = Some(result);
    }

    Ok(preview)
}

/// Re-derive the current take with different settings.
///
/// The difference between this and `capture_transcribe` is only where the audio comes
/// from — and that is the whole point. Changing the grid, or switching between the
/// estimated tempo and the project's, used to mean playing the phrase again.
#[tauri::command]
pub async fn capture_retranscribe(
    state: State<'_, AppState>,
    use_project_tempo: bool,
    quantize_ticks: u32,
) -> CommandResult<TranscriptionPreview> {
    let (samples, sample_rate, track) = {
        let capture = locked(&state)?;
        let track = capture
            .pending
            .as_ref()
            .map(|(track, _)| *track)
            .or_else(|| capture.analysis.as_ref().map(|_| 0))
            .ok_or_else(|| CommandError::from("there is no take to re-read".to_string()))?;
        (capture.samples.clone(), capture.sample_rate, track)
    };

    if samples.is_empty() || sample_rate <= 0.0 {
        return Err(CommandError::from(
            "the take is no longer available — record or open a file again".to_string(),
        ));
    }

    transcribe_samples(&state, track, samples, sample_rate, use_project_tempo, quantize_ticks).await
}

/// Load an audio file as the current take.
///
/// The microphone is not the main way audio arrives. A voice memo, a bounce, a stem
/// someone sent — and, once this is a plugin, whatever the host hands over.
#[tauri::command]
pub async fn capture_load_file(
    state: State<'_, AppState>,
    path: String,
    track: usize,
    use_project_tempo: bool,
    quantize_ticks: u32,
) -> CommandResult<TranscriptionPreview> {
    state.mic.stop();

    let (samples, sample_rate) = tauri::async_runtime::spawn_blocking(move || {
        unplugged_audio::decode_file(&path)
    })
    .await
    .map_err(|e| CommandError::from(format!("decoding did not finish: {e}")))?
    .map_err(|e| CommandError {
        code: "decode",
        message: e.to_string(),
    })?;

    {
        let mut capture = locked(&state)?;
        capture.recording = false;
        capture.samples = samples.clone();
        capture.sample_rate = sample_rate;
        capture.pending = None;
    }

    transcribe_samples(&state, track, samples, sample_rate, use_project_tempo, quantize_ticks).await
}

/// Peaks over a window of the retained take, for drawing the waveform at any zoom.
///
/// Min/max per bucket rather than the whole buffer: sending two minutes of 48 kHz audio
/// to the webview to draw one screen would be twenty megabytes for a few hundred pixels.
#[tauri::command]
pub fn capture_waveform(
    state: State<'_, AppState>,
    from_seconds: f64,
    to_seconds: f64,
    buckets: usize,
) -> CommandResult<Vec<(f32, f32)>> {
    let capture = locked(&state)?;
    Ok(unplugged_transcribe::peaks(
        &capture.samples,
        capture.sample_rate,
        from_seconds,
        to_seconds,
        buckets.min(4096),
    ))
}

/// Replace the pending notes with ones the user adjusted in the editor.
///
/// Validated here rather than trusted: these arrive from the webview, and a note that
/// breaks the model's invariants must not reach the command layer.
#[tauri::command]
pub fn capture_set_notes(state: State<'_, AppState>, notes: Vec<Note>) -> CommandResult<usize> {
    for note in &notes {
        note.validate()?;
    }

    let mut capture = locked(&state)?;
    let track = capture
        .pending
        .as_ref()
        .map(|(track, _)| *track)
        .ok_or_else(|| CommandError::from("there is no transcription to adjust".to_string()))?;

    let mut notes = notes;
    notes.sort_by_key(Note::order_key);
    let count = notes.len();
    capture.pending = Some((track, notes));
    Ok(count)
}

/// Commit the pending transcription as a single undoable insert.
#[tauri::command]
pub fn capture_accept(state: State<'_, AppState>) -> CommandResult<EditorState> {
    let (track, notes) = {
        let mut capture = locked(&state)?;
        capture
            .pending
            .take()
            .ok_or_else(|| CommandError::from("there is no transcription to apply".to_string()))?
    };

    let mut guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))?;
    let open = guard
        .as_mut()
        .ok_or_else(|| CommandError::from("no project is open".to_string()))?;

    if track >= open.tracks().len() {
        return Err(CommandError::from(
            "that track no longer exists".to_string(),
        ));
    }

    let outcome = open
        .session
        .apply(Transaction::single(
            "Transcribe",
            Command::Insert { track, notes },
        ))?;

    // The take has become notes. Holding twenty-odd megabytes for a result the user has
    // already committed would be the retention turning into a leak.
    state.audition.stop();
    state.preview.stop();
    if let Ok(mut capture) = state.capture.lock() {
        capture.samples = Vec::new();
        capture.analysis = None;
    }
    open.dirty = true;
    state.audio.set_timeline(open.timeline());

    Ok(EditorState::of(open, outcome.affected, outcome.track))
}

/// Discard the take, whether it is still recording or already transcribed.
#[tauri::command]
pub fn capture_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    state.mic.stop();
    state.audition.stop();
    state.preview.stop();
    let mut capture = locked(&state)?;
    capture.recording = false;
    capture.samples = Vec::new();
    capture.analysis = None;
    capture.pending = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use unplugged_transcribe::DetectedNote;

    fn detected(pitch: u8) -> DetectedNote {
        DetectedNote {
            note: Note::new(pitch, 0, 480, 90, 0).unwrap(),
            start_seconds: 0.0,
            duration_seconds: 0.5,
            confidence: 0.9,
            cents_off: 3.0,
        }
    }

    #[test]
    fn an_empty_result_is_reported_as_such() {
        let preview = TranscriptionPreview::of(
            &Transcription {
                tempo_bpm: 120.0,
                ..Default::default()
            },
            true,
            0,
        );
        assert!(preview.warning.is_some());
        assert!(preview.notes.is_empty());
    }

    #[test]
    fn a_mostly_unpitched_recording_is_flagged_but_still_returned() {
        let preview = TranscriptionPreview::of(
            &Transcription {
                notes: vec![detected(60), detected(64)],
                tempo_bpm: 120.0,
                pitched_fraction: 0.05,
                ..Default::default()
            },
            true,
            120,
        );

        assert!(preview.warning.is_some(), "the user should be told");
        assert_eq!(preview.notes.len(), 2, "but the notes are still offered");
    }

    #[test]
    fn a_clean_recording_carries_no_warning() {
        let preview = TranscriptionPreview::of(
            &Transcription {
                notes: vec![detected(60)],
                tempo_bpm: 120.0,
                tempo_estimated: true,
                tempo_confidence: 0.8,
                duration_seconds: 4.0,
                pitched_fraction: 0.7,
                analysis: Default::default(),
            },
            false,
            240,
        );
        assert!(preview.warning.is_none());
        assert!(preview.tempo_estimated);
        // The settings come back with the result so the editor's controls open showing
        // what actually produced what is on screen.
        assert!(!preview.use_project_tempo);
        assert_eq!(preview.quantize_ticks, 240);
    }
}
