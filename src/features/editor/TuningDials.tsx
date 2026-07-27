import { useEffect, useState } from "react";

import { DEFAULT_TUNING, type TranscribeTuning } from "../../lib/types";

interface TuningDialsProps {
  tuning: TranscribeTuning;
  disabled: boolean;
  /** Called when a dial is *released*, not while it moves. */
  onCommit: (tuning: TranscribeTuning) => void;
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

function isDefault(tuning: TranscribeTuning): boolean {
  return DIALS.every((dial) => tuning[dial.key] === DEFAULT_TUNING[dial.key]);
}

/**
 * How the take is read, rather than what was played.
 *
 * These were constants, and each one is a guess about the source — how percussive it is,
 * how steady the pitch is, how much room is in the recording. The defaults suit a hummed
 * line at a laptop; a plucked string or a breathy voice wants something else, and getting
 * it wrong produces a plausible-looking result rather than an obviously broken one.
 *
 * Moving a dial re-reads the take that is already in memory, which is seconds of work, so
 * a change is sent when the control is *released* rather than on every pixel of a drag.
 */
export function TuningDials({ tuning, disabled, onCommit }: TuningDialsProps) {
  // Shown while dragging, before Rust has been asked for anything.
  const [draft, setDraft] = useState(tuning);

  // A re-derivation answers with the tuning it actually used, including any clamping.
  useEffect(() => setDraft(tuning), [tuning]);

  const commit = (next: TranscribeTuning) => {
    if (DIALS.every((dial) => next[dial.key] === tuning[dial.key])) return;
    onCommit(next);
  };

  return (
    <details className="tuning">
      <summary className="tuning__summary">
        Analysis
        {!isDefault(tuning) && <span className="tuning__badge">adjusted</span>}
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
              onChange={(e) => setDraft({ ...draft, [dial.key]: Number(e.target.value) })}
              onPointerUp={() => commit(draft)}
              onKeyUp={() => commit(draft)}
              onBlur={() => commit(draft)}
            />
          </label>
        ))}
      </div>

      <div className="tuning__foot">
        <span className="field__hint">
          Changing one of these re-reads the take you already performed — it never asks
          for another.
        </span>
        <button
          className="btn btn--ghost"
          onClick={() => onCommit(DEFAULT_TUNING)}
          disabled={disabled || isDefault(tuning)}
        >
          Reset
        </button>
      </div>
    </details>
  );
}
