import { DEFAULT_TUNING, type TranscribeTuning } from "../../lib/types";

interface TuningDialsProps {
  /** The settings on the dials, which are not necessarily the ones on screen. */
  draft: TranscribeTuning;
  /** What produced the result currently drawn, for the "changed" marker. */
  applied: TranscribeTuning;
  disabled: boolean;
  onChange: (tuning: TranscribeTuning) => void;
}

interface Dial {
  key: keyof TranscribeTuning;
  label: string;
  min: number;
  max: number;
  step: number;
  /** How to read the number back to the user. */
  format: (value: number) => string;
  hint: string;
}

/**
 * The dials, in the order you would reach for them when a result is wrong.
 *
 * Each one is named for the symptom rather than for the stage of the pipeline it belongs
 * to: nobody looking at a bad transcription is thinking "the spectral flux threshold is
 * too high", they are thinking "it heard one note where I played two".
 */
const DIALS: Dial[] = [
  {
    key: "split_sensitivity",
    label: "Split repeated notes",
    min: 0,
    max: 1,
    step: 0.05,
    format: (value) => `${Math.round(value * 100)}%`,
    hint: "Up if one note came back where you played two. Down if a single note was chopped up.",
  },
  {
    key: "min_note_ms",
    label: "Shortest note",
    min: 10,
    max: 500,
    step: 10,
    format: (value) => `${Math.round(value)} ms`,
    hint: "Anything briefer than this is dropped as a chirp between notes.",
  },
  {
    key: "pitch_tolerance_semitones",
    label: "Pitch tolerance",
    min: 0.1,
    max: 3,
    step: 0.1,
    format: (value) => `${value.toFixed(1)} semitones`,
    hint: "How far the pitch may wander inside one note. Up for a wide vibrato; down to catch a slur.",
  },
  {
    key: "noise_floor",
    label: "Noise floor",
    min: 0,
    max: 0.2,
    step: 0.005,
    format: (value) => `${(value * 100).toFixed(1)}% of peak`,
    hint: "Up to ignore room noise; down to keep a quiet tail.",
  },
  {
    key: "min_confidence",
    label: "Pitch confidence",
    min: 0.1,
    max: 0.95,
    step: 0.05,
    format: (value) => value.toFixed(2),
    hint: "How sure the tracker must be. Down if a breathy take comes back empty.",
  },
];

/** Whether two tunings agree on every dial. */
export function sameTuning(a: TranscribeTuning, b: TranscribeTuning): boolean {
  return DIALS.every((dial) => a[dial.key] === b[dial.key]);
}

/**
 * How the take is read, rather than what was played.
 *
 * These were constants, and each one is a guess about the source — how percussive it is,
 * how steady the pitch is, how much room is in the recording. The defaults suit a hummed
 * line at a laptop; a plucked string or a breathy voice wants something else, and getting
 * it wrong produces a plausible-looking result rather than an obviously broken one.
 *
 * The dials only move a *draft*. Re-reading the take is seconds of work, and doing it on
 * every release meant a pass of the analysis for each dial touched on the way to the
 * setting you wanted — so nothing happens until the button over the editor is pressed.
 */
export function TuningDials({ draft, applied, disabled, onChange }: TuningDialsProps) {
  const set = (key: keyof TranscribeTuning, value: number) =>
    onChange({ ...draft, [key]: value });

  return (
    <details className="tuning">
      <summary className="tuning__summary">
        Analysis
        {!sameTuning(applied, DEFAULT_TUNING) && <span className="tuning__badge">adjusted</span>}
      </summary>

      <div className="tuning__grid">
        {DIALS.map((dial) => (
          <label key={dial.key} className="tuning__dial" title={dial.hint}>
            <span className="tuning__label">
              {dial.label}
              <span className="tuning__value mono">{dial.format(draft[dial.key])}</span>
            </span>
            <input
              type="range"
              min={dial.min}
              max={dial.max}
              step={dial.step}
              value={draft[dial.key]}
              disabled={disabled}
              onChange={(e) => set(dial.key, Number(e.target.value))}
            />
          </label>
        ))}
      </div>

      <div className="tuning__foot">
        <span className="field__hint">
          These re-read the take you already performed — nothing here ever asks for
          another. Press Re-process to hear the result.
        </span>
        <button
          className="btn btn--ghost"
          onClick={() => onChange(DEFAULT_TUNING)}
          disabled={disabled || sameTuning(draft, DEFAULT_TUNING)}
        >
          Reset
        </button>
      </div>
    </details>
  );
}
