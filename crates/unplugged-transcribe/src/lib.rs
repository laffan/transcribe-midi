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

/// Shortest note kept, in milliseconds. Below this it is a chirp between two notes rather
/// than a note — 60 ms is already faster than anything played deliberately.
const MIN_NOTE_MS: f32 = 60.0;

/// A pitch change this large starts a new note even without an onset. Half a semitone,
/// so ordinary vibrato does not fragment a held note.
const PITCH_BREAK_SEMITONES: f32 = 0.5;

// ---------------------------------------------------------------------------
// Options and results
// ---------------------------------------------------------------------------

/// The judgement calls in the pipeline, as numbers the user can move.
///
/// Every one of these was a constant, and every one of them is a guess about the source:
/// how percussive it is, how steady the singer's pitch is, how much room noise there is.
/// The defaults are the guess that suits a hummed line at a laptop, which is the common
/// case and not the only one — a plucked string wants a different split sensitivity, and
/// a voice with a wide vibrato wants a different pitch tolerance. Getting them wrong
/// produces a plausible-looking result, so they are worth exposing rather than worth
/// tuning once and hiding.
///
/// The take is retained for the session, so changing one of these re-reads what was
/// already performed rather than asking for it again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TranscribeTuning {
    /// How readily a repeated note is split from its neighbour. 0 needs an unmistakable
    /// attack; 1 splits on the slightest one. 0.5 is the tuning everything before this
    /// was fixed at.
    pub split_sensitivity: f32,
    /// Shortest note kept, in milliseconds. Anything briefer is a chirp between two
    /// notes rather than a note.
    pub min_note_ms: f32,
    /// How far the pitch must move, in semitones, before it counts as a new note rather
    /// than the same one wavering. Raise it for a singer with a wide vibrato; lower it
    /// to catch a legato slur that has no attack for the flux to see.
    pub pitch_tolerance_semitones: f32,
    /// Level below which a frame is silence, as a fraction of the take's own peak.
    /// Raise it to ignore room noise; lower it to keep a quiet tail.
    pub noise_floor: f32,
    /// How sure the pitch tracker must be for a frame to count as pitched. Lower it for
    /// a breathy or noisy source that is coming back with nothing.
    pub min_confidence: f32,
}

impl Default for TranscribeTuning {
    fn default() -> Self {
        TranscribeTuning {
            split_sensitivity: 0.5,
            min_note_ms: MIN_NOTE_MS,
            pitch_tolerance_semitones: PITCH_BREAK_SEMITONES,
            noise_floor: SILENCE_FLOOR,
            min_confidence: MIN_CONFIDENCE,
        }
    }
}

impl TranscribeTuning {
    /// Bring every dial inside the range the pipeline can actually work over.
    ///
    /// These arrive from the webview. A zero-length minimum note or a confidence of two
    /// does not crash anything, it just returns thousands of notes or none, which looks
    /// like a broken transcriber rather than a bad setting.
    pub fn clamped(self) -> Self {
        TranscribeTuning {
            split_sensitivity: clamp(self.split_sensitivity, 0.0, 1.0),
            min_note_ms: clamp(self.min_note_ms, 10.0, 1000.0),
            pitch_tolerance_semitones: clamp(self.pitch_tolerance_semitones, 0.1, 6.0),
            noise_floor: clamp(self.noise_floor, 0.0, 0.5),
            min_confidence: clamp(self.min_confidence, 0.1, 0.95),
        }
    }

    /// Shortest note, in frames, at this hop.
    fn min_note_frames(&self, hop_seconds: f64) -> usize {
        ((self.min_note_ms as f64 / 1000.0 / hop_seconds.max(1e-6)).round() as usize).max(1)
    }

    /// Onset parameters for this sensitivity.
    ///
    /// One multiplicative scale over all three thresholds, so the dial is monotone and
    /// nothing can cross zero: 0.5 reproduces [`onset::OnsetParams::default`] exactly,
    /// which is what every transcription before this was made with.
    fn onset_params(&self) -> onset::OnsetParams {
        let defaults = onset::OnsetParams::default();
        let scale = 2f32.powf(1.0 - 2.0 * self.split_sensitivity);
        onset::OnsetParams {
            delta: defaults.delta * scale,
            ratio: 1.0 + (defaults.ratio - 1.0) * scale,
            min_flux: defaults.min_flux * scale,
            ..defaults
        }
    }
}

/// `f32::clamp` panics on a NaN bound; this substitutes the low end instead, because a
/// NaN arriving from JSON should be a dull default rather than a crash.
fn clamp(value: f32, low: f32, high: f32) -> f32 {
    if value.is_nan() {
        low
    } else {
        value.clamp(low, high)
    }
}

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
    pub tuning: TranscribeTuning,
}

impl TranscribeOptions {
    pub fn new(sample_rate: f64, ppq: u16) -> Self {
        TranscribeOptions {
            sample_rate,
            ppq,
            tempo_bpm: None,
            quantize_ticks: 0,
            channel: 0,
            tuning: TranscribeTuning::default(),
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
    /// The per-frame evidence behind the notes.
    pub analysis: Analysis,
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
///
/// Public, and returned with the transcription, because these frames *are* the editor.
/// The pitch track is the line the notes sit on, confidence says which notes are worth a
/// second look, and level draws the envelope. Phase 7 computed all of this and threw it
/// away, which is why changing the grid meant recording again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// Detected fundamental in Hz. Zero when the frame is unpitched.
    pub frequency: f32,
    /// Fractional MIDI note number, so the pitch line is drawn where it was measured
    /// rather than where it was rounded to. Zero when unpitched.
    pub midi: f32,
    pub confidence: f32,
    pub level: f32,
}

impl Frame {
    fn voiced(&self, floor: f32, min_confidence: f32) -> bool {
        self.frequency > 0.0 && self.confidence >= min_confidence && self.level > floor
    }
}

/// Everything the transcription editor needs to draw over the waveform.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub frames: Vec<Frame>,
    /// Frame indices where an attack was detected. Note boundaries snap to these.
    pub onsets: Vec<usize>,
    /// Seconds between frames.
    pub hop_seconds: f64,
    /// The level below which a frame counts as silence, in the same units as
    /// `Frame::level`. Drawn as the noise floor.
    pub silence_floor: f32,
}

/// Transcribe mono samples into notes.
pub fn transcribe(samples: &[f32], options: TranscribeOptions) -> Transcription {
    transcribe_reporting(samples, options, &mut |_| {})
}

/// How much of the total time the pitch stage accounts for.
///
/// YIN over every frame is the dominant cost and spectral flux is most of the rest, so
/// the bar is split between them rather than by pipeline stage — the tempo estimate and
/// the assembly are a pass each over one value per frame and finish instantly.
const PITCH_SHARE: f32 = 0.65;
const FLUX_SHARE: f32 = 0.33;

/// [`transcribe`], reporting 0–1 as it goes.
///
/// The caller gets progress rather than a spinner because two minutes of audio is
/// several seconds of work, and a bar that is not moving is indistinguishable from an
/// app that has hung. `progress` is called from whatever thread this runs on.
pub fn transcribe_reporting(
    samples: &[f32],
    options: TranscribeOptions,
    progress: &mut dyn FnMut(f32),
) -> Transcription {
    let tuning = options.tuning.clamped();
    let hop = ((options.sample_rate * HOP_SECONDS).round() as usize).max(1);
    let duration_seconds = samples.len() as f64 / options.sample_rate.max(1.0);

    if samples.len() < FRAME_SIZE || options.sample_rate <= 0.0 {
        progress(1.0);
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

    let total = windows.len().max(1);
    let frames: Vec<Frame> = windows
        .iter()
        .enumerate()
        .map(|(index, window)| {
            if index % 16 == 0 {
                progress(index as f32 / total as f32 * PITCH_SHARE);
            }
            let estimate = pitch::yin(window, options.sample_rate, MIN_HZ, MAX_HZ);
            Frame {
                frequency: estimate.frequency,
                midi: pitch::hz_to_midi(estimate.frequency),
                confidence: estimate.confidence,
                level: dsp::rms(window),
            }
        })
        .collect();

    let peak = frames.iter().map(|f| f.level).fold(0.0f32, f32::max);
    let floor = peak * tuning.noise_floor;

    // -- onsets -----------------------------------------------------------
    let window = dsp::hann(FRAME_SIZE);
    let track = onset::detect_reporting(&windows, &window, tuning.onset_params(), &mut |done| {
        progress(PITCH_SHARE + done * FLUX_SHARE)
    });
    progress(PITCH_SHARE + FLUX_SHARE);

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
    let segments = segment(&frames, &track.onsets, floor, &tuning);
    let detected: Vec<DetectedNote> = segments
        .iter()
        .filter_map(|&(start, end)| {
            build_note(&frames, start, end, hop, tempo_bpm, &options, floor, &tuning)
        })
        .collect();
    // Segments cannot overlap, but quantisation can push one note's snapped start behind
    // its neighbour's snapped end. What was performed by one voice must come back
    // playable by one voice.
    let notes = one_voice(detected);

    let pitched = frames
        .iter()
        .filter(|f| f.voiced(floor, tuning.min_confidence))
        .count();
    progress(1.0);

    Transcription {
        notes,
        tempo_bpm,
        tempo_estimated,
        tempo_confidence,
        duration_seconds,
        pitched_fraction: pitched as f32 / frames.len().max(1) as f32,
        analysis: Analysis {
            frames,
            onsets: track.onsets,
            hop_seconds: hop as f64 / options.sample_rate,
            silence_floor: floor,
        },
    }
}

/// Min/max pairs over `buckets` equal spans of `samples`.
///
/// What a waveform view actually draws. A bucket's *extremes* rather than its mean or its
/// RMS: at any zoom where one pixel covers hundreds of samples, averaging turns a
/// percussive attack into a low bump and the eye loses exactly the feature it is looking
/// for. Min and max keep the transient.
///
/// The range is given in seconds so the caller can ask for what is on screen rather than
/// receiving the whole take at every zoom level.
pub fn peaks(
    samples: &[f32],
    sample_rate: f64,
    from_seconds: f64,
    to_seconds: f64,
    buckets: usize,
) -> Vec<(f32, f32)> {
    if samples.is_empty() || buckets == 0 || sample_rate <= 0.0 || to_seconds <= from_seconds {
        return Vec::new();
    }

    let start = ((from_seconds.max(0.0) * sample_rate) as usize).min(samples.len());
    let end = ((to_seconds.max(0.0) * sample_rate).ceil() as usize).min(samples.len());
    if end <= start {
        return Vec::new();
    }

    let span = end - start;
    (0..buckets)
        .map(|bucket| {
            let low = start + span * bucket / buckets;
            // At least one sample per bucket: asking for more buckets than there are
            // samples is a legitimate thing for a zoomed-in view to do, and it should
            // repeat samples rather than return empty columns.
            let high = (start + span * (bucket + 1) / buckets).max(low + 1).min(end);

            let slice = &samples[low..high];
            slice.iter().fold((f32::MAX, f32::MIN), |(min, max), &s| {
                (min.min(s), max.max(s))
            })
        })
        .collect()
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
/// Apply the monophonic rule to detected notes, keeping their evidence.
///
/// The decision belongs to `unplugged_core::monophony` — it is the same rule the fine-tune
/// editor's drags go through, and two copies of it would drift. Only the note is trimmed:
/// `start_seconds` and `duration_seconds` are measurements of the performance, and a
/// measurement does not change because a grid moved a note.
fn one_voice(detected: Vec<DetectedNote>) -> Vec<DetectedNote> {
    let notes: Vec<Note> = detected.iter().map(|d| d.note).collect();
    unplugged_core::monophony::flatten_indexed(&notes)
        .into_iter()
        .map(|(index, duration_ticks)| {
            let mut kept = detected[index];
            kept.note.duration_ticks = duration_ticks;
            kept
        })
        .collect()
}

fn segment(
    frames: &[Frame],
    onsets: &[usize],
    floor: f32,
    tuning: &TranscribeTuning,
) -> Vec<(usize, usize)> {
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
        let voiced = frame.voiced(floor, tuning.min_confidence);

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
        let moved = (midi - reference).abs() >= tuning.pitch_tolerance_semitones;

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

#[allow(clippy::too_many_arguments)]
fn build_note(
    frames: &[Frame],
    start: usize,
    end: usize,
    hop: usize,
    tempo_bpm: f64,
    options: &TranscribeOptions,
    floor: f32,
    tuning: &TranscribeTuning,
) -> Option<DetectedNote> {
    let seconds_per_frame = hop as f64 / options.sample_rate;
    let min_frames = tuning.min_note_frames(seconds_per_frame);

    if end.saturating_sub(start) < min_frames {
        return None;
    }

    // Skip the first two frames: an attack transient is inharmonic and drags the pitch
    // estimate around. What is left is the steady part, which is what was played.
    let body_start = (start + 2).min(end);
    let voiced: Vec<&Frame> = frames[body_start..end]
        .iter()
        .filter(|frame| frame.voiced(floor, tuning.min_confidence))
        .collect();

    if voiced.len() < min_frames / 2 {
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
mod tests;
