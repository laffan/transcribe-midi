//! Phase 4: external MIDI input, live monitoring and loop recording.
//!
//! The routing rule throughout: **live input is heard immediately and recorded against
//! the audio clock.** Monitoring goes straight to the armed track's sampler, bypassing
//! the sequencer, while the recorder is stamped with `audio.position_ticks()` at the
//! moment the event arrives. That is the Phase 0 decision made concrete — `midir` does
//! not timestamp on iOS, so its clock is never consulted on any platform.

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, Manager, State};
use unplugged_core::command::{Command, Transaction};
use unplugged_core::Ticks;
use unplugged_midi::{MidiEvent, MidiPort};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct InputSettings {
    pub ports: Vec<MidiPort>,
    pub connected: Option<MidiPort>,
    pub channel: Option<u8>,
    pub armed_track: u16,
    pub keyboard_velocity: u8,
    pub count_in_bars: u8,
    pub metronome: bool,
    pub recording: bool,
}

/// Emitted when live input arrives, so the piano roll and keyboard can light up.
#[derive(Debug, Clone, Serialize)]
struct LiveNoteEvent {
    pitch: u8,
    velocity: u8,
    on: bool,
}

fn settings_snapshot(state: &AppState) -> InputSettings {
    let input = state.input.lock().ok();
    InputSettings {
        ports: state.midi.ports().unwrap_or_default(),
        connected: state.midi.connected(),
        channel: None, // Reported by `midi_set_channel`; the host owns the filter.
        armed_track: input.as_ref().map(|i| i.armed_track).unwrap_or(0),
        keyboard_velocity: input.as_ref().map(|i| i.keyboard_velocity).unwrap_or(100),
        count_in_bars: input.as_ref().map(|i| i.count_in_bars).unwrap_or(0),
        metronome: state.audio.metronome_enabled(),
        recording: input.as_ref().map(|i| i.recording).unwrap_or(false),
    }
}

#[tauri::command]
pub fn input_settings(state: State<'_, AppState>) -> CommandResult<InputSettings> {
    Ok(settings_snapshot(&state))
}

#[tauri::command]
pub fn midi_ports(state: State<'_, AppState>) -> CommandResult<Vec<MidiPort>> {
    state
        .midi
        .ports()
        .map_err(|e| CommandError::from(e.to_string()))
}

/// Connect to a MIDI input port and start monitoring.
#[tauri::command]
pub fn midi_connect(app: tauri::AppHandle, port_id: String) -> CommandResult<MidiPort> {
    let handle = app.clone();

    // The sink runs on midir's callback thread. It does three things and nothing else:
    // sound the note, stamp it for the recorder, and tell the UI.
    let sink: unplugged_midi::EventSink = Arc::new(move |event: MidiEvent| {
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        handle_live_event(&handle, &state, event);
    });

    let state = app.state::<AppState>();
    state
        .midi
        .connect(&port_id, sink)
        .map_err(|e| CommandError::from(e.to_string()))
}

#[tauri::command]
pub fn midi_disconnect(state: State<'_, AppState>) -> CommandResult<()> {
    state.midi.disconnect();
    // Anything held on the wire when the cable is pulled would hang otherwise.
    let _ = state.audio.all_notes_off();
    Ok(())
}

/// `None` accepts every channel.
#[tauri::command]
pub fn midi_set_channel(state: State<'_, AppState>, channel: Option<u8>) -> CommandResult<()> {
    state.midi.set_channel_filter(channel);
    Ok(())
}

/// Route live input and recording to this track.
#[tauri::command]
pub fn set_armed_track(state: State<'_, AppState>, track: u16) -> CommandResult<()> {
    let mut input = state
        .input
        .lock()
        .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;

    if input.recording {
        return Err(CommandError::from(
            "stop recording before changing the armed track".to_string(),
        ));
    }

    // Release anything sounding on the old track, or it hangs there forever.
    for (track, pitch, channel) in input.sounding.drain(..) {
        let _ = state.audio.note_off(track, pitch, channel);
    }
    input.armed_track = track;
    Ok(())
}

#[tauri::command]
pub fn set_keyboard_velocity(state: State<'_, AppState>, velocity: u8) -> CommandResult<()> {
    let mut input = state
        .input
        .lock()
        .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;
    input.keyboard_velocity = velocity.clamp(1, 127);
    Ok(())
}

#[tauri::command]
pub fn set_count_in_bars(state: State<'_, AppState>, bars: u8) -> CommandResult<()> {
    let mut input = state
        .input
        .lock()
        .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;
    input.count_in_bars = bars.min(8);
    Ok(())
}

#[tauri::command]
pub fn set_metronome(state: State<'_, AppState>, enabled: bool) -> CommandResult<()> {
    state.audio.set_metronome(enabled);
    Ok(())
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RecordResult {
    pub recording: bool,
    pub count_in_ticks: Ticks,
    pub captured_notes: usize,
}

/// Arm recording and roll the transport.
///
/// With a count-in the playhead is rewound by that many bars and the sequencer is told
/// to keep the timeline silent until the record point, so the player hears only the
/// click during the lead-in.
#[tauri::command]
pub fn record_start(state: State<'_, AppState>) -> CommandResult<RecordResult> {
    let (bar_ticks, quantize) = {
        let guard = state
            .open
            .lock()
            .map_err(|_| CommandError::from("editor state lock poisoned".to_string()))?;
        let open = guard
            .as_ref()
            .ok_or_else(|| CommandError::from("no project is open".to_string()))?;
        (
            open.manifest.time_signature.bar_ticks(open.manifest.ppq),
            0u32,
        )
    };

    let mut input = state
        .input
        .lock()
        .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;

    if input.recording {
        return Err(CommandError::from("already recording".to_string()));
    }

    let record_at = state.audio.position_ticks();
    let count_in_ticks = bar_ticks.saturating_mul(input.count_in_bars as u32);

    input.recorder.clear();
    input.recorder.set_quantize(quantize);
    input.record_start_tick = record_at;
    input.recording = true;

    if count_in_ticks > 0 {
        // Rewind by the count-in and suppress the timeline until the record point.
        state.audio.seek(record_at.saturating_sub(count_in_ticks));
        state.audio.set_count_in_until(Some(record_at));
    } else {
        state.audio.set_count_in_until(None);
    }

    state
        .audio
        .play()
        .map_err(|e| CommandError::from(e.to_string()))?;

    Ok(RecordResult {
        recording: true,
        count_in_ticks,
        captured_notes: 0,
    })
}

/// Stop recording and commit the take through the command layer.
///
/// Captured notes become a single `Insert` transaction, so the whole take is one undo
/// step — the same guarantee the spec requires of AI edits, for the same reason.
#[tauri::command]
pub fn record_stop(state: State<'_, AppState>) -> CommandResult<crate::editor::EditorState> {
    let position = state.audio.position_ticks();

    let (armed_track, notes) = {
        let mut input = state
            .input
            .lock()
            .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;

        if !input.recording {
            return Err(CommandError::from("not recording".to_string()));
        }
        input.recording = false;
        (input.armed_track, input.recorder.finish(position))
    };

    state.audio.set_count_in_until(None);
    state
        .audio
        .stop()
        .map_err(|e| CommandError::from(e.to_string()))?;

    let mut guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("editor state lock poisoned".to_string()))?;
    let open = guard
        .as_mut()
        .ok_or_else(|| CommandError::from("no project is open".to_string()))?;

    let outcome = if notes.is_empty() {
        Default::default()
    } else {
        let transaction = Transaction::single(
            "Record",
            Command::Insert {
                track: armed_track as usize,
                notes,
            },
        );
        let outcome = open.session.apply(transaction)?;
        open.dirty = true;
        state.audio.set_timeline(open.timeline());
        outcome
    };

    Ok(crate::editor::EditorState::of(open, outcome.affected, outcome.track))
}

/// Discard the take without committing it.
#[tauri::command]
pub fn record_cancel(state: State<'_, AppState>) -> CommandResult<()> {
    let mut input = state
        .input
        .lock()
        .map_err(|_| CommandError::from("input state lock poisoned".to_string()))?;

    input.recording = false;
    input.recorder.clear();
    let rewind_to = input.record_start_tick;
    drop(input);

    state.audio.set_count_in_until(None);
    let _ = state.audio.stop();
    state.audio.seek(rewind_to);
    Ok(())
}

// ---------------------------------------------------------------------------
// The live path
// ---------------------------------------------------------------------------

/// Handle one event from the MIDI callback thread.
fn handle_live_event(app: &tauri::AppHandle, state: &AppState, event: MidiEvent) {
    // Stamp against our own clock, never midir's — see the module docs.
    let tick = state.audio.position_ticks();

    let Ok(mut input) = state.input.lock() else {
        return;
    };
    let track = input.armed_track;

    match event {
        MidiEvent::NoteOn { pitch, velocity, channel } => {
            let _ = state.audio.note_on(track, pitch, velocity, channel);
            input.sounding.push((track, pitch, channel));
            if input.recording && !state.audio.in_count_in() {
                input.recorder.note_on(tick, pitch, velocity, channel);
            }
            let _ = app.emit("live-note", LiveNoteEvent { pitch, velocity, on: true });
        }
        MidiEvent::NoteOff { pitch, channel } => {
            let _ = state.audio.note_off(track, pitch, channel);
            input
                .sounding
                .retain(|(t, p, c)| !(*t == track && *p == pitch && *c == channel));
            if input.recording {
                input.recorder.note_off(tick, pitch, channel);
            }
            let _ = app.emit("live-note", LiveNoteEvent { pitch, velocity: 0, on: false });
        }
        // Controllers and pitch bend are monitored but not recorded: the model stores
        // notes only. Capturing them is a later phase's problem, and dropping them
        // silently here is better than half-recording a performance.
        MidiEvent::ControlChange { .. } | MidiEvent::PitchBend { .. } => {}
    }
}

/// Called from the playhead thread when the transport wraps, so a note held across the
/// loop boundary is closed and reopened rather than left dangling.
pub fn on_loop_wrap(state: &AppState, loop_end: Ticks, loop_start: Ticks) {
    if let Ok(mut input) = state.input.lock() {
        if input.recording {
            input.recorder.on_loop_wrap(loop_end, loop_start);
        }
    }
}
