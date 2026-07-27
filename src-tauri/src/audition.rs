//! Hearing a take back: as the notes it became, as the sound it came from, or both.
//!
//! Phase 9 could only play the recording. That answers "what did I play?" and leaves the
//! more useful question — "is this transcription right?" — to be answered by accepting the
//! notes and listening to them afterwards, which is the wrong order. The notes are what is
//! being judged, so the notes are what plays by default, and the recording is there to
//! compare against.
//!
//! The two sources are driven by different clocks and that is deliberate. The recording
//! goes through the preview player, which is real audio playback with a sample clock. The
//! notes go through this module's scheduler thread, which walks
//! [`unplugged_core::audition`]'s boundaries against a wall clock and sounds them on the
//! sampler by the same path as the on-screen keyboard. Nothing here runs on the audio
//! thread; a couple of milliseconds of jitter is inaudible in a phrase you are auditioning
//! and is the price of not disturbing the project's transport to play four bars back.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{Manager, State};
use unplugged_core::audition::{schedule, span_seconds, AuditionEvent};

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// What the user asked to hear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditionSource {
    /// The transcription, played by the sampler. The default: it is the thing on trial.
    Midi,
    /// The recording the transcription came from.
    Take,
    Both,
}

impl AuditionSource {
    fn plays_midi(self) -> bool {
        matches!(self, AuditionSource::Midi | AuditionSource::Both)
    }

    fn plays_take(self) -> bool {
        matches!(self, AuditionSource::Take | AuditionSource::Both)
    }
}

/// How long the scheduler sleeps between checks while waiting for the next boundary.
///
/// It is also how long a stop takes to silence a held note, which is why it is small.
const TICK: Duration = Duration::from_millis(2);

/// A run in progress, so position can be answered without asking the thread.
struct Run {
    generation: u64,
    started: Instant,
    from_seconds: f64,
    until_seconds: f64,
}

/// Plays the pending notes on the sampler, off the audio thread.
///
/// Cancellation is a counter rather than a flag: `stop` followed immediately by `play`
/// must not let the outgoing thread's last boundary land inside the incoming one's run.
#[derive(Default)]
pub struct Audition {
    generation: AtomicU64,
    clock: Mutex<Option<Run>>,
}

impl Audition {
    /// Start sounding `events`, which must already be relative to `from_seconds`.
    pub fn play(
        &self,
        app: &tauri::AppHandle,
        track: u16,
        events: Vec<AuditionEvent>,
        from_seconds: f64,
        until_seconds: f64,
    ) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let started = Instant::now();

        if let Ok(mut clock) = self.clock.lock() {
            *clock = Some(Run {
                generation,
                started,
                from_seconds,
                until_seconds,
            });
        }

        let app = app.clone();
        std::thread::spawn(move || run(app, generation, track, started, events));
    }

    /// Silence and forget the current run. Held notes are released by the thread within
    /// [`TICK`], which is also what makes this safe to call from a command.
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut clock) = self.clock.lock() {
            *clock = None;
        }
    }

    /// Where playback has reached, or `None` when nothing is running.
    pub fn position(&self) -> Option<f64> {
        let guard = self.clock.lock().ok()?;
        let run = guard.as_ref()?;
        let at = run.from_seconds + run.started.elapsed().as_secs_f64();
        (at <= run.until_seconds).then_some(at)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Clear the clock once a run reaches its end, leaving a later run's alone.
    fn finish(&self, generation: u64) {
        if let Ok(mut clock) = self.clock.lock() {
            if clock.as_ref().is_some_and(|run| run.generation == generation) {
                *clock = None;
            }
        }
    }
}

/// The scheduler thread.
///
/// It reaches the audio engine through the app handle rather than holding a reference,
/// because `AppState` is owned by Tauri and outlives every thread that borrows it this
/// way — the playhead publisher in `lib.rs` does the same.
fn run(
    app: tauri::AppHandle,
    generation: u64,
    track: u16,
    started: Instant,
    events: Vec<AuditionEvent>,
) {
    // What this run has sounded and not yet released, so a cancellation does not leave a
    // note hanging on the sampler until the user presses Panic.
    let mut held: Vec<(u8, u8)> = Vec::new();

    for event in events {
        let target = started + Duration::from_secs_f64(event.at_seconds);

        loop {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            if !state.audition.is_current(generation) {
                release(&state, track, &held);
                return;
            }

            let now = Instant::now();
            if now >= target {
                break;
            }
            std::thread::sleep(TICK.min(target - now));
        }

        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        if event.on {
            let _ = state
                .audio
                .note_on(track, event.pitch, event.velocity, event.channel);
            held.push((event.pitch, event.channel));
        } else {
            let _ = state.audio.note_off(track, event.pitch, event.channel);
            if let Some(index) = held.iter().position(|h| *h == (event.pitch, event.channel)) {
                held.remove(index);
            }
        }
    }

    if let Some(state) = app.try_state::<AppState>() {
        release(&state, track, &held);
        if state.audition.is_current(generation) {
            state.audition.finish(generation);
        }
    }
}

fn release(state: &AppState, track: u16, held: &[(u8, u8)]) {
    for (pitch, channel) in held {
        let _ = state.audio.note_off(track, *pitch, *channel);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Play the pending take from `fromSeconds` through whichever sources were asked for.
///
/// Succeeds if *any* requested source started. Off-Apple the preview player refuses and
/// the sampler is silent, so "Both" would otherwise be an error on a machine where the
/// notes are still perfectly reviewable on screen.
#[tauri::command]
pub fn capture_audition_play(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    from_seconds: f64,
    source: AuditionSource,
) -> CommandResult<()> {
    let from_seconds = from_seconds.max(0.0);

    // Whatever was playing is replaced, not layered.
    state.audition.stop();
    state.preview.stop();

    let (notes, track, tempo_bpm, samples, sample_rate) = {
        let capture = crate::transcribe::locked(&state)?;
        let (track, notes) = match capture.pending.as_ref() {
            Some((track, notes)) => (*track, notes.clone()),
            None => (0, Vec::new()),
        };
        let tempo_bpm = capture
            .analysis
            .as_ref()
            .map(|analysis| analysis.tempo_bpm)
            .unwrap_or_default();
        (
            notes,
            track,
            tempo_bpm,
            capture.samples.clone(),
            capture.sample_rate,
        )
    };

    let mut started = false;
    let mut refusal: Option<String> = None;

    if source.plays_take() {
        if samples.is_empty() || sample_rate <= 0.0 {
            refusal = Some("there is no recording to play".to_string());
        } else {
            match state
                .preview
                .load(&samples, sample_rate)
                .and_then(|()| state.preview.play(from_seconds))
            {
                Ok(()) => started = true,
                Err(error) => refusal = Some(error.to_string()),
            }
        }
    }

    if source.plays_midi() {
        let ppq = project_ppq(&state)?;
        let ticks_per_second = (tempo_bpm / 60.0) * f64::from(ppq);
        let events = schedule(&notes, ticks_per_second, from_seconds);

        if events.is_empty() {
            refusal.get_or_insert_with(|| "there are no notes to play".to_string());
        } else {
            let until = span_seconds(&notes, ticks_per_second);
            state
                .audition
                .play(&app, track as u16, events, from_seconds, until);
            started = true;
        }
    }

    if started {
        Ok(())
    } else {
        Err(CommandError::from(
            refusal.unwrap_or_else(|| "there is nothing to play".to_string()),
        ))
    }
}

#[tauri::command]
pub fn capture_audition_stop(state: State<'_, AppState>) -> CommandResult<()> {
    state.audition.stop();
    state.preview.stop();
    Ok(())
}

/// Where playback has reached, or `None` when stopped.
///
/// The recording's own clock wins when it is playing: it is a sample clock, and when both
/// sources are running it is the one the ear is following.
#[tauri::command]
pub fn capture_audition_position(state: State<'_, AppState>) -> CommandResult<Option<f64>> {
    Ok(state.preview.position().or_else(|| state.audition.position()))
}

fn project_ppq(state: &AppState) -> CommandResult<u16> {
    let guard = state
        .open
        .lock()
        .map_err(|_| CommandError::from("the editor state lock was poisoned".to_string()))?;
    guard
        .as_ref()
        .map(|open| open.manifest.ppq)
        .ok_or_else(|| CommandError::from("no project is open".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_source_names_exactly_what_it_plays() {
        assert!(AuditionSource::Midi.plays_midi() && !AuditionSource::Midi.plays_take());
        assert!(AuditionSource::Take.plays_take() && !AuditionSource::Take.plays_midi());
        assert!(AuditionSource::Both.plays_midi() && AuditionSource::Both.plays_take());
    }

    #[test]
    fn the_source_arrives_from_the_webview_in_snake_case() {
        let source: AuditionSource = serde_json::from_str("\"both\"").unwrap();
        assert_eq!(source, AuditionSource::Both);
        assert!(serde_json::from_str::<AuditionSource>("\"Both\"").is_err());
    }

    #[test]
    fn a_stopped_audition_reports_no_position() {
        let audition = Audition::default();
        assert_eq!(audition.position(), None);
        audition.stop();
        assert_eq!(audition.position(), None);
    }

    #[test]
    fn stopping_makes_the_running_generation_stale() {
        let audition = Audition::default();
        let generation = audition.generation.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(audition.is_current(generation));
        audition.stop();
        assert!(
            !audition.is_current(generation),
            "the thread must see its run has been replaced"
        );
    }

    #[test]
    fn a_finished_run_does_not_clear_a_later_ones_clock() {
        let audition = Audition::default();
        // Two runs in a row: the first finishing late must not silence the second.
        let stale = audition.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = audition.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *audition.clock.lock().unwrap() = Some(Run {
            generation: current,
            started: Instant::now(),
            from_seconds: 0.0,
            until_seconds: 60.0,
        });

        audition.finish(stale);
        assert!(audition.position().is_some(), "the current run is still running");
        audition.finish(current);
        assert_eq!(audition.position(), None);
    }

    #[test]
    fn a_run_reports_nothing_once_it_has_passed_its_end() {
        let audition = Audition::default();
        *audition.clock.lock().unwrap() = Some(Run {
            generation: 1,
            started: Instant::now(),
            from_seconds: 4.0,
            until_seconds: 2.0,
        });
        assert_eq!(
            audition.position(),
            None,
            "past the end is stopped, not a playhead off the end of the take"
        );
    }
}
