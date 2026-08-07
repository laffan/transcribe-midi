/**
 * A canned take, for driving the Listen review stage in a browser.
 *
 * The mock backend refuses everything that needs Rust, and for transcription that left a
 * hole: the review stage — the waveform, the pitch trace, the notes you drag, the dials —
 * could not be opened at all without a Mac, which is exactly the surface that most needs
 * looking at while it is being changed.
 *
 * So this is a **fixture, not an analysis**. Nothing here listens to a microphone or
 * measures a pitch: the phrase, the trace under it and the response to the dials are all
 * written down in advance, deterministic, and deliberately simple. It exists so the view
 * can be driven, not so transcription can be judged — every real answer still comes from
 * `unplugged-transcribe`, which is where the tests are.
 */

import type {
  Analysis,
  DetectedNote,
  Frame,
  TranscribeTuning,
  TranscriptionPreview,
  WaveformPeaks,
} from "./types";

/** Where the fixture's "performance" sits, in seconds. */
const HOP = 0.01;
const TEMPO_PLAYED = 96;
const SILENCE_FLOOR = 0.004;

interface Phrase {
  pitch: number;
  start: number;
  duration: number;
  /** Distance from equal temperament, in cents. One note is deliberately way off. */
  cents: number;
  confidence: number;
}

/**
 * A hummed line: up the triad and back down, with a held note at the top.
 *
 * Two of them earn their place. The E is 44 cents flat — far enough to show up in the
 * readout and on the trace without being a different note — and the long G is what the
 * split dial has something to do to.
 */
const PHRASE: Phrase[] = [
  { pitch: 60, start: 0.35, duration: 0.55, cents: 6, confidence: 0.92 },
  { pitch: 62, start: 1.0, duration: 0.5, cents: -12, confidence: 0.88 },
  { pitch: 64, start: 1.6, duration: 0.6, cents: -44, confidence: 0.71 },
  { pitch: 67, start: 2.35, duration: 1.3, cents: 4, confidence: 0.95 },
  { pitch: 64, start: 3.8, duration: 0.45, cents: -18, confidence: 0.83 },
  { pitch: 62, start: 4.35, duration: 0.4, cents: 9, confidence: 0.79 },
  { pitch: 60, start: 4.85, duration: 0.9, cents: -3, confidence: 0.9 },
];

export const TAKE_SECONDS = 6.1;

/** Fractional MIDI at a moment inside a note: the offset, plus a slow vibrato. */
function tracedMidi(phrase: Phrase, seconds: number): number {
  const into = seconds - phrase.start;
  const vibrato = Math.sin(into * 2 * Math.PI * 5.5) * 0.08 * Math.min(1, into / 0.25);
  return phrase.pitch + phrase.cents / 100 + vibrato;
}

function phraseAt(seconds: number, phrases: Phrase[]): Phrase | null {
  return phrases.find((p) => seconds >= p.start && seconds < p.start + p.duration) ?? null;
}

/**
 * Level envelope: an attack, a body, and a decay into the room.
 *
 * Deliberately **quiet** — it peaks around a fifteenth of full scale, which is what a
 * hummed line at arm's length from a tablet actually records at. A fixture bounced at
 * studio level would have hidden the bug this file exists to catch: the waveform lane
 * drew absolute amplitude, so on a device the take was a flat line and looked broken.
 */
function levelAt(seconds: number, phrases: Phrase[]): number {
  const phrase = phraseAt(seconds, phrases);
  if (!phrase) return 0.0012;
  const into = seconds - phrase.start;
  const left = phrase.start + phrase.duration - seconds;
  const attack = Math.min(1, into / 0.04);
  const release = Math.min(1, left / 0.12);
  return 0.006 + 0.062 * attack * release;
}

function analysisOf(phrases: Phrase[]): Analysis {
  const frames: Frame[] = [];
  for (let index = 0; index * HOP < TAKE_SECONDS; index += 1) {
    const seconds = index * HOP;
    const phrase = phraseAt(seconds, phrases);
    const level = levelAt(seconds, phrases);
    const midi = phrase ? tracedMidi(phrase, seconds) : 0;
    frames.push({
      frequency: midi > 0 ? 440 * 2 ** ((midi - 69) / 12) : 0,
      midi,
      confidence: phrase ? phrase.confidence : 0.1,
      level,
    });
  }

  return {
    frames,
    onsets: phrases.map((p) => Math.round(p.start / HOP)),
    hop_seconds: HOP,
    silence_floor: SILENCE_FLOOR,
  };
}

/**
 * What the dials do to the result, as a caricature.
 *
 * Each rule is a one-line stand-in for a stage of the real pipeline, chosen so that
 * moving a dial visibly changes the answer — which is what the re-process affordance
 * needs in order to be exercisable. None of them is how the transcriber actually works.
 */
function tuned(tuning: TranscribeTuning): Phrase[] {
  let phrases = PHRASE.filter((p) => p.duration * 1000 >= tuning.min_note_ms);
  phrases = phrases.filter((p) => p.confidence >= tuning.min_confidence);

  if (tuning.split_sensitivity > 0.7) {
    // The held note comes back as two, which is the symptom the dial is named for.
    phrases = phrases.flatMap((p) =>
      p.duration < 0.9
        ? [p]
        : [
            { ...p, duration: p.duration / 2 - 0.02 },
            { ...p, start: p.start + p.duration / 2, duration: p.duration / 2 },
          ],
    );
  }

  if (tuning.pitch_tolerance_semitones < 0.3) {
    // A tight tolerance reads the vibrato as a pitch change and chops the long note.
    phrases = phrases.flatMap((p) =>
      p.duration < 0.6
        ? [p]
        : [
            { ...p, duration: 0.3 },
            { ...p, start: p.start + 0.32, duration: p.duration - 0.32 },
          ],
    );
  }

  return phrases;
}

export function mockPreview(
  ppq: number,
  useProjectTempo: boolean,
  projectTempo: number,
  quantizeTicks: number,
  tuning: TranscribeTuning,
): TranscriptionPreview {
  const phrases = tuned(tuning);
  const tempo = useProjectTempo ? projectTempo : TEMPO_PLAYED;
  const ticksPerSecond = (tempo / 60) * ppq;

  const notes: DetectedNote[] = phrases.map((phrase) => {
    const rawStart = phrase.start * ticksPerSecond;
    const start =
      quantizeTicks > 0 ? Math.round(rawStart / quantizeTicks) * quantizeTicks : Math.round(rawStart);
    return {
      note: {
        pitch: phrase.pitch,
        start_ticks: Math.max(0, start),
        duration_ticks: Math.max(1, Math.round(phrase.duration * ticksPerSecond)),
        velocity: 88,
        channel: 0,
      },
      start_seconds: phrase.start,
      duration_seconds: phrase.duration,
      confidence: phrase.confidence,
      cents_off: phrase.cents,
    };
  });

  return {
    notes,
    tempo_bpm: tempo,
    tempo_estimated: !useProjectTempo,
    tempo_confidence: 0.62,
    duration_seconds: TAKE_SECONDS,
    pitched_fraction: 0.71,
    warning: notes.length === 0 ? "Nothing came back — the dials are set too tight." : null,
    analysis: analysisOf(phrases),
    use_project_tempo: useProjectTempo,
    quantize_ticks: quantizeTicks,
    tuning,
  };
}

/** Min/max pairs over a window of the take, one pair per bucket. */
export function mockWaveform(from: number, to: number, buckets: number): WaveformPeaks {
  const span = Math.max(0.001, to - from);
  const peaks: WaveformPeaks = [];
  for (let bucket = 0; bucket < buckets; bucket += 1) {
    const seconds = from + (bucket / buckets) * span;
    const level = levelAt(seconds, PHRASE);
    // A waveform is not an envelope: the fill inside it wobbles, deterministically so
    // the picture is stable between redraws at the same zoom.
    const wobble = 0.55 + 0.45 * Math.abs(Math.sin(seconds * 137.5));
    const amplitude = Math.min(1, level * wobble * 1.4);
    peaks.push([-amplitude, amplitude]);
  }
  return peaks;
}

/**
 * A clock, so the playhead moves and looping can be seen to loop.
 *
 * Nothing sounds — the browser preview has never made a noise, by design — but position
 * is what the review stage draws and what the loop watches, and a stopped clock makes
 * both untestable.
 */
class MockAudition {
  private startedAt: number | null = null;
  private from = 0;

  play(fromSeconds: number): void {
    this.from = Math.max(0, fromSeconds);
    this.startedAt = performance.now();
  }

  stop(): void {
    this.startedAt = null;
  }

  position(): number | null {
    if (this.startedAt === null) return null;
    const at = this.from + (performance.now() - this.startedAt) / 1000;
    if (at >= TAKE_SECONDS) {
      this.startedAt = null;
      return null;
    }
    return at;
  }
}

export const mockAudition = new MockAudition();
