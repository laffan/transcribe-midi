//! Phase 7: audio to MIDI, monophonic.
//!
//! **Monophonic only, and that is a design decision rather than an omission.** Polyphonic
//! transcription is a different problem — it needs spectral factorisation or a trained
//! model, not a pitch tracker — and a version of this that quietly did its best on a
//! chord would produce plausible-looking nonsense. Given a chord, YIN reports one pitch:
//! usually the loudest partial, sometimes a difference tone, never the chord. So the UI
//! says "one note at a time" and this module makes no attempt to hide it.
//!
//! The pipeline is four stages, each in its own module and tested against synthetic
//! signals a Linux CI host can generate:
//!
//! 1. **Framing** — overlapping windows, 10 ms apart.
//! 2. **Pitch** ([`pitch`]) — YIN per frame, giving frequency and confidence.
//! 3. **Onsets** ([`onset`]) — spectral flux, the only thing that can separate two
//!    repetitions of the same note.
//! 4. **Assembly** — segment at onsets and at voicing changes, take the median pitch of
//!    each segment, then optionally quantise.
//!
//! There is no audio I/O here and no platform code: the input is a slice of samples.

pub mod dsp;
pub mod onset;
pub mod pitch;
pub mod tempo;

use serde::{Deserialize, Serialize};

use unplugged_core::{Note, Ticks};

/// Analysis frame length. About 46 ms at 44.1 kHz.
///
/// Long enough to hold two periods of a low E (82 Hz) with room to spare, which YIN
/// needs, and short enough that a 16th note at 160 bpm still spans several frames.
pub const FRAME_SIZE: usize = 2048;

/// Hop between frames, in seconds.
pub const HOP_SECONDS: f64 = 0.01;

/// Pitch search range: roughly E1 to C7. Below this is rumble, above it is hiss.
pub const MIN_HZ: f64 = 41.0;
pub const MAX_HZ: f64 = 2100.0;

/// How confident YIN must be for a frame to count as pitched.
const MIN_CONFIDENCE: f32 = 0.55;

/// Level, relative to the recording's peak, below which a frame is silence.
///
/// Relative rather than absolute so a quiet take transcribes like a loud one.
const SILENCE_FLOOR: f32 = 0.02;

/// Shortest note kept, in frames. Below this it is a chirp between two notes rather than
/// a note — 60 ms is already faster than anything played deliberately.
const MIN_NOTE_FRAMES: usize = 6;

/// A pitch change this large starts a new note even without an onset. Half a semitone,
/// so ordinary vibrato does not fragment a held note.
const PITCH_BREAK_SEMITONES: f32 = 0.5;

// ---------------------------------------------------------------------------
// Options and results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TranscribeOptions {
    pub sample_rate: f64,
    pub ppq: u16,
    /// Tempo to place notes against. `None` estimates it from the recording.
    pub tempo_bpm: Option<f64>,
    /// Grid to snap to, in ticks. Zero leaves the performance where it was played.
    pub quantize_ticks: u32,
    /// MIDI channel for the produced notes.
    pub channel: u8,
}

impl TranscribeOptions {
    pub fn new(sample_rate: f64, ppq: u16) -> Self {
        TranscribeOptions {
            sample_rate,
            ppq,
            tempo_bpm: None,
            quantize_ticks: 0,
            channel: 0,
        }
    }
}

/// One detected note, with the evidence behind it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectedNote {
    pub note: Note,
    /// Where it was actually played, before quantisation.
    pub start_seconds: f64,
    pub duration_seconds: f64,
    /// Mean YIN confidence across the note, 0–1.
    pub confidence: f32,
    /// How far the measured pitch sat from equal temperament, in cents. Large values
    /// mean an out-of-tune source, not a detection failure.
    pub cents_off: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transcription {
    pub notes: Vec<DetectedNote>,
    /// The tempo used, whether given or estimated.
    pub tempo_bpm: f64,
    /// True when the tempo was estimated rather than supplied.
    pub tempo_estimated: bool,
    /// Confidence in the estimate; zero when it was supplied.
    pub tempo_confidence: f32,
    /// Length of the analysed audio.
    pub duration_seconds: f64,
    /// Fraction of the recording that held a detectable pitch. A low value usually means
    /// the microphone heard the room rather than the instrument.
    pub pitched_fraction: f32,
}

impl Transcription {
    pub fn notes(&self) -> Vec<Note> {
        self.notes.iter().map(|detected| detected.note).collect()
    }
}

// ---------------------------------------------------------------------------
// The pipeline
// ---------------------------------------------------------------------------

/// One frame's worth of measurements.
#[derive(Debug, Clone, Copy)]
struct Frame {
    frequency: f32,
    confidence: f32,
    level: f32,
}

impl Frame {
    fn voiced(&self, floor: f32) -> bool {
        self.frequency > 0.0 && self.confidence >= MIN_CONFIDENCE && self.level > floor
    }
}

/// Transcribe mono samples into notes.
pub fn transcribe(samples: &[f32], options: TranscribeOptions) -> Transcription {
    let hop = ((options.sample_rate * HOP_SECONDS).round() as usize).max(1);
    let duration_seconds = samples.len() as f64 / options.sample_rate.max(1.0);

    if samples.len() < FRAME_SIZE || options.sample_rate <= 0.0 {
        return Transcription {
            tempo_bpm: options.tempo_bpm.unwrap_or(unplugged_core::DEFAULT_TEMPO),
            duration_seconds,
            ..Default::default()
        };
    }

    // -- frame ------------------------------------------------------------
    let windows: Vec<&[f32]> = (0..)
        .map(|index| index * hop)
        .take_while(|&start| start + FRAME_SIZE <= samples.len())
        .map(|start| &samples[start..start + FRAME_SIZE])
        .collect();

    let frames: Vec<Frame> = windows
        .iter()
        .map(|window| {
            let estimate = pitch::yin(window, options.sample_rate, MIN_HZ, MAX_HZ);
            Frame {
                frequency: estimate.frequency,
                confidence: estimate.confidence,
                level: dsp::rms(window),
            }
        })
        .collect();

    let peak = frames.iter().map(|f| f.level).fold(0.0f32, f32::max);
    let floor = peak * SILENCE_FLOOR;

    // -- onsets -----------------------------------------------------------
    let window = dsp::hann(FRAME_SIZE);
    let track = onset::detect(&windows, &window, onset::OnsetParams::default());

    // -- tempo ------------------------------------------------------------
    let frames_per_second = options.sample_rate / hop as f64;
    let (tempo_bpm, tempo_estimated, tempo_confidence) = match options.tempo_bpm {
        Some(given) => (given, false, 0.0),
        None => match tempo::estimate(&track.flux, frames_per_second) {
            Some(estimate) => (tempo::tidy(estimate.bpm), true, estimate.confidence),
            None => (unplugged_core::DEFAULT_TEMPO, true, 0.0),
        },
    };

    // -- assemble ---------------------------------------------------------
    let segments = segment(&frames, &track.onsets, floor);
    let notes = segments
        .iter()
        .filter_map(|&(start, end)| build_note(&frames, start, end, hop, tempo_bpm, &options, floor))
        .collect();

    let pitched = frames.iter().filter(|f| f.voiced(floor)).count();

    Transcription {
        notes,
        tempo_bpm,
        tempo_estimated,
        tempo_confidence,
        duration_seconds,
        pitched_fraction: pitched as f32 / frames.len().max(1) as f32,
    }
}

/// Split the frame track into candidate notes.
///
/// Three things end a note: silence or loss of pitch, a detected onset, and a sustained
/// pitch change. The third is needed because a legato slur from C to G has no attack for
/// the flux to see, and without it the pair would come out as one note at whichever
/// pitch happened to be the median.
///
/// The onset frame index is used as the boundary directly, which places the note a frame
/// or two early — the attack is somewhere inside the window that first saw it. In
/// exchange, a note is never clipped at the front, which is far more audible than a
/// little silence before it.
fn segment(frames: &[Frame], onsets: &[usize], floor: f32) -> Vec<(usize, usize)> {
    let is_onset = {
        let mut flags = vec![false; frames.len()];
        for &index in onsets {
            if index < flags.len() {
                flags[index] = true;
            }
        }
        flags
    };

    let mut segments = Vec::new();
    let mut open: Option<usize> = None;
    let mut reference = 0.0f32;

    for (index, frame) in frames.iter().enumerate() {
        let voiced = frame.voiced(floor);

        let Some(start) = open else {
            if voiced {
                open = Some(index);
                reference = pitch::hz_to_midi(frame.frequency);
            }
            continue;
        };

        if !voiced {
            segments.push((start, index));
            open = None;
            continue;
        }

        let midi = pitch::hz_to_midi(frame.frequency);
        let moved = (midi - reference).abs() >= PITCH_BREAK_SEMITONES;

        if is_onset[index] || moved {
            segments.push((start, index));
            open = Some(index);
            reference = midi;
        } else {
            // Track slowly, so a note that drifts a little is not eventually judged
            // against a stale reference from its very first frame.
            reference = reference * 0.85 + midi * 0.15;
        }
    }

    if let Some(start) = open {
        segments.push((start, frames.len()));
    }

    segments
}

fn build_note(
    frames: &[Frame],
    start: usize,
    end: usize,
    hop: usize,
    tempo_bpm: f64,
    options: &TranscribeOptions,
    floor: f32,
) -> Option<DetectedNote> {
    if end.saturating_sub(start) < MIN_NOTE_FRAMES {
        return None;
    }

    // Skip the first two frames: an attack transient is inharmonic and drags the pitch
    // estimate around. What is left is the steady part, which is what was played.
    let body_start = (start + 2).min(end);
    let voiced: Vec<&Frame> = frames[body_start..end]
        .iter()
        .filter(|frame| frame.voiced(floor))
        .collect();

    if voiced.len() < MIN_NOTE_FRAMES / 2 {
        return None;
    }

    // Median rather than mean: a single octave-error frame would drag a mean half an
    // octave, and the median simply ignores it.
    let midi_values: Vec<f32> = voiced
        .iter()
        .map(|frame| pitch::hz_to_midi(frame.frequency))
        .collect();
    let median_midi = dsp::median(&midi_values);
    let pitch_number = median_midi.round().clamp(0.0, 127.0) as u8;
    let cents_off = (median_midi - median_midi.round()) * 100.0;

    let confidence = voiced.iter().map(|frame| frame.confidence).sum::<f32>() / voiced.len() as f32;

    // Velocity from the loudest frame in the note, which is the attack.
    let peak_level = voiced.iter().map(|frame| frame.level).fold(0.0f32, f32::max);
    let velocity = level_to_velocity(peak_level, floor);

    let seconds_per_frame = hop as f64 / options.sample_rate;
    let start_seconds = start as f64 * seconds_per_frame;
    let duration_seconds = (end - start) as f64 * seconds_per_frame;

    let ticks_per_second = tempo_bpm / 60.0 * options.ppq as f64;
    let mut start_ticks = (start_seconds * ticks_per_second).round().max(0.0) as Ticks;
    let mut duration_ticks = (duration_seconds * ticks_per_second).round().max(1.0) as Ticks;

    if options.quantize_ticks > 0 {
        let grid = options.quantize_ticks as f64;
        start_ticks = ((start_ticks as f64 / grid).round() * grid) as Ticks;
        // Lengths snap too, but never below one grid step — a note rounded to zero
        // length cannot be written to a MIDI file at all.
        duration_ticks =
            (((duration_ticks as f64 / grid).round() * grid) as Ticks).max(options.quantize_ticks);
    }

    Some(DetectedNote {
        note: Note::new(
            pitch_number,
            start_ticks,
            duration_ticks.max(1),
            velocity,
            options.channel,
        )
        .ok()?,
        start_seconds,
        duration_seconds,
        confidence,
        cents_off,
    })
}

/// Map a peak level to a MIDI velocity.
///
/// Logarithmic, because loudness is: a linear map puts almost everything played at a
/// normal level into the top of the range and makes the result sound machine-flat.
fn level_to_velocity(level: f32, floor: f32) -> u8 {
    let reference = floor.max(1e-6);
    if level <= reference {
        return 1;
    }
    // 40 dB of range spread across the velocity scale.
    let db = 20.0 * (level / reference).log10();
    let scaled = (db / 40.0).clamp(0.0, 1.0);
    (1.0 + scaled * 126.0).round().clamp(1.0, 127.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const SAMPLE_RATE: f64 = 44100.0;
    const PPQ: u16 = 480;

    fn options() -> TranscribeOptions {
        TranscribeOptions::new(SAMPLE_RATE, PPQ)
    }

    /// A note with harmonics and a percussive envelope — close enough to a plucked or
    /// struck instrument for the pipeline to behave as it would on a real take.
    fn note_signal(midi: u8, seconds: f64, amplitude: f32) -> Vec<f32> {
        let hz = pitch::midi_to_hz(midi as f32);
        let samples = (SAMPLE_RATE * seconds) as usize;
        (0..samples)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let envelope = (-3.0 * t / seconds as f32).exp();
                amplitude
                    * envelope
                    * ((2.0 * PI * hz * t).sin()
                        + 0.5 * (2.0 * PI * hz * 2.0 * t).sin()
                        + 0.25 * (2.0 * PI * hz * 3.0 * t).sin())
                    / 1.75
            })
            .collect()
    }

    fn melody(pitches: &[u8], seconds_each: f64) -> Vec<f32> {
        let mut signal = Vec::new();
        for &midi in pitches {
            signal.extend(note_signal(midi, seconds_each, 0.7));
        }
        signal
    }

    #[test]
    fn transcribes_a_simple_melody() {
        let played = [60u8, 62, 64, 65, 67];
        let result = transcribe(&melody(&played, 0.5), options());

        assert_eq!(
            result.notes().iter().map(|n| n.pitch).collect::<Vec<_>>(),
            played,
            "detected {:?}",
            result
                .notes
                .iter()
                .map(|n| (n.note.pitch, n.start_seconds))
                .collect::<Vec<_>>()
        );

        for (index, detected) in result.notes.iter().enumerate() {
            let expected = index as f64 * 0.5;
            assert!(
                (detected.start_seconds - expected).abs() < 0.07,
                "note {index} starts at {:.3}s, expected {expected:.3}s",
                detected.start_seconds
            );
            assert!(detected.confidence > 0.7);
            assert!(detected.cents_off.abs() < 30.0, "{}", detected.cents_off);
        }
    }

    #[test]
    fn separates_repeated_notes_at_the_same_pitch() {
        // Pitch alone cannot see these boundaries; the flux onsets are what find them.
        let result = transcribe(&melody(&[60, 60, 60, 60], 0.45), options());
        assert_eq!(result.notes.len(), 4, "{:?}", result.notes());
    }

    #[test]
    fn silence_produces_nothing() {
        let result = transcribe(&vec![0.0f32; 44100 * 2], options());
        assert!(result.notes.is_empty());
        assert_eq!(result.pitched_fraction, 0.0);
        assert!((result.duration_seconds - 2.0).abs() < 0.01);
    }

    #[test]
    fn a_buffer_shorter_than_one_frame_is_handled() {
        let result = transcribe(&vec![0.1f32; 100], options());
        assert!(result.notes.is_empty());
        assert!(result.tempo_bpm > 0.0, "a usable tempo is still reported");
    }

    #[test]
    fn leading_silence_does_not_become_a_note() {
        let mut signal = vec![0.0f32; 22050];
        signal.extend(note_signal(64, 0.6, 0.7));

        let result = transcribe(&signal, options());
        assert_eq!(result.notes.len(), 1, "{:?}", result.notes());
        assert_eq!(result.notes[0].note.pitch, 64);
        assert!(
            (result.notes[0].start_seconds - 0.5).abs() < 0.07,
            "started at {:.3}s",
            result.notes[0].start_seconds
        );
    }

    #[test]
    fn a_legato_slur_still_becomes_two_notes() {
        // No attack between them, so there is no onset to find — the pitch break is the
        // only evidence, which is exactly why segmentation looks for it.
        let mut signal = Vec::new();
        let hz_a = pitch::midi_to_hz(60.0);
        let hz_b = pitch::midi_to_hz(67.0);
        let half = (SAMPLE_RATE * 0.6) as usize;
        let mut phase = 0.0f32;

        for i in 0..half * 2 {
            let hz = if i < half { hz_a } else { hz_b };
            phase += 2.0 * PI * hz / SAMPLE_RATE as f32;
            signal.push(0.6 * phase.sin());
        }

        let result = transcribe(&signal, options());
        let pitches: Vec<u8> = result.notes().iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, vec![60, 67], "{:?}", result.notes());
    }

    #[test]
    fn vibrato_does_not_fragment_a_held_note() {
        let hz = pitch::midi_to_hz(69.0);
        let samples = (SAMPLE_RATE * 1.2) as usize;
        let mut phase = 0.0f32;
        let signal: Vec<f32> = (0..samples)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                // ±25 cents at 5 Hz — an ordinary singing or string vibrato.
                let bend = 1.0 + 0.0145 * (2.0 * PI * 5.0 * t).sin();
                phase += 2.0 * PI * hz * bend / SAMPLE_RATE as f32;
                0.6 * phase.sin()
            })
            .collect();

        let result = transcribe(&signal, options());
        assert_eq!(result.notes.len(), 1, "{:?}", result.notes());
        assert_eq!(result.notes[0].note.pitch, 69);
    }

    #[test]
    fn a_given_tempo_is_used_verbatim() {
        let mut settings = options();
        settings.tempo_bpm = Some(96.0);

        let result = transcribe(&melody(&[60, 62], 0.5), settings);
        assert_eq!(result.tempo_bpm, 96.0);
        assert!(!result.tempo_estimated);

        // At 96 bpm a quarter note is 625 ms, so 500 ms is 384 ticks.
        let second = result.notes[1].note.start_ticks as i64;
        assert!((second - 384).abs() <= 16, "second note at {second} ticks");
    }

    #[test]
    fn quantization_snaps_to_the_grid() {
        let mut settings = options();
        settings.tempo_bpm = Some(120.0);
        settings.quantize_ticks = 240; // eighth notes

        // At 120 bpm, 0.45 s is 432 ticks — deliberately off the eighth-note grid.
        let result = transcribe(&melody(&[60, 62, 64], 0.45), settings);
        assert!(!result.notes.is_empty());
        for detected in &result.notes {
            assert_eq!(
                detected.note.start_ticks % 240,
                0,
                "note at {} is off the grid",
                detected.note.start_ticks
            );
            assert!(detected.note.duration_ticks >= 240);
        }
    }

    #[test]
    fn velocity_follows_loudness() {
        let mut signal = note_signal(60, 0.5, 0.9);
        signal.extend(note_signal(62, 0.5, 0.15));

        let result = transcribe(&signal, options());
        assert_eq!(result.notes.len(), 2, "{:?}", result.notes());
        assert!(
            result.notes[0].note.velocity > result.notes[1].note.velocity + 10,
            "{} vs {}",
            result.notes[0].note.velocity,
            result.notes[1].note.velocity
        );
        assert!(result.notes.iter().all(|n| n.note.velocity >= 1));
    }

    #[test]
    fn the_notes_come_back_valid_and_ordered() {
        let result = transcribe(&melody(&[55, 60, 64, 67, 72], 0.4), options());
        let notes = result.notes();

        assert!(!notes.is_empty());
        for note in &notes {
            note.validate()
                .expect("every produced note must be representable in SMF");
        }
        for pair in notes.windows(2) {
            assert!(
                pair[0].order_key() <= pair[1].order_key(),
                "notes must come back sorted"
            );
        }
    }

    #[test]
    fn a_chord_is_not_pretended_to_be_understood() {
        // Three simultaneous pitches. The contract is that this yields a monophonic line
        // — never three notes at once — because the UI promises one note at a time and
        // silently returning a wrong chord would be worse than returning less.
        let samples = (SAMPLE_RATE * 1.0) as usize;
        let signal: Vec<f32> = (0..samples)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.3 * ((2.0 * PI * 261.6 * t).sin()
                    + (2.0 * PI * 329.6 * t).sin()
                    + (2.0 * PI * 392.0 * t).sin())
            })
            .collect();

        let result = transcribe(&signal, options());
        let simultaneous = result
            .notes()
            .windows(2)
            .filter(|pair| pair[0].start_ticks == pair[1].start_ticks)
            .count();
        assert_eq!(simultaneous, 0, "no two notes may start together");
    }
}
