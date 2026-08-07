import { useState } from "react";

import { Slider } from "../../components/Slider";
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
    format: (value) => `${value.toFixed(1)} st`,
    hint: "How far the pitch may wander inside one note. Up for a wide vibrato; down to catch a slur.",
  },
  {
    key: "noise_floor",
    label: "Noise floor",
    min: 0,
    max: 0.2,
    step: 0.005,
    format: (value) => `${(value * 100).toFixed(1)}%`,
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
 * **Open when the stage opens.** It was a closed disclosure on the theory that changing
 * how a take is *read* is rarer than moving a note that came back wrong. That theory
 * survived until someone had a wrong result in front of them: the dials are what fixes a
 * whole take at once, and a fold that hides them costs a press and, first, knowing they
 * are there at all. Collapsing is still there for when the canvas is what you want.
 *
 * The dials only move a *draft*. Re-reading the take is seconds of work, and doing it on
 * every release meant a pass of the analysis for each dial touched on the way to the
 * setting you wanted — so nothing happens until Re-process is pressed.
 */
export function TuningDials({ draft, applied, disabled, onChange }: TuningDialsProps) {
  const [open, setOpen] = useState(true);

  const set = (key: keyof TranscribeTuning, value: number) =>
    onChange({ ...draft, [key]: value });

  const pending = !sameTuning(draft, applied);

  return (
    <section className={`tuning ${pending ? "tuning--pending" : ""}`}>
      <header className="tuning__head">
        <button
          className="tuning__toggle"
          onClick={() => setOpen((v) => !v)}
          aria-expanded={open}
        >
          <span className={`tuning__chevron ${open ? "tuning__chevron--open" : ""}`}>▸</span>
          Analysis
        </button>

        {!sameTuning(applied, DEFAULT_TUNING) && (
          <span className="tuning__badge">adjusted</span>
        )}
        {pending && (
          <span className="tuning__badge tuning__badge--pending">
            not applied yet — press Re-process
          </span>
        )}

        <div className="spacer" />

        <button
          className="btn btn--ghost"
          onClick={() => onChange(DEFAULT_TUNING)}
          disabled={disabled || sameTuning(draft, DEFAULT_TUNING)}
          title="Put every dial back where it started"
        >
          Reset
        </button>
      </header>

      {open && (
        <div className="tuning__body">
          <div className="tuning__grid">
            {DIALS.map((dial) => (
              <Slider
                key={dial.key}
                label={dial.label}
                value={draft[dial.key]}
                min={dial.min}
                max={dial.max}
                step={dial.step}
                format={dial.format}
                hint={dial.hint}
                disabled={disabled}
                onChange={(value) => set(dial.key, value)}
              />
            ))}
          </div>

          <p className="tuning__note">
            These re-read the take you already performed — nothing here ever asks for
            another.
          </p>
        </div>
      )}
    </section>
  );
}
