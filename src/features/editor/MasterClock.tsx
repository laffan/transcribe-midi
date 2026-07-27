import { useState } from "react";

import { MAX_TEMPO, MIN_TEMPO, type TimeSignature } from "../../lib/types";
import { formatPosition, noteName } from "./timeFormat";

interface MasterClockProps {
  positionTicks: number;
  ppq: number;
  timeSignature: TimeSignature;
  tempoBpm: number;
  inCountIn: boolean;
  /** Pitch sounding at the playhead on the selected track, or null in a gap. */
  notePitch: number | null;
  onTempoChange: (bpm: number) => void;
}

type Readout = "bars" | "note";

/** How much a nudge moves the tempo. One bpm is the unit people think in. */
const TEMPO_STEP = 1;

/**
 * The clock the whole app is keeping.
 *
 * A DAW's display is a single panel that says where you are and how fast — Logic's LCD,
 * and every LCD since — so this is one inset panel rather than three controls that happen
 * to sit near each other. The readout switches between the bar you are in and the note
 * under the playhead, because those are the two ways a position gets talked about, and
 * for an app whose input is a sung line the second one is often the more useful.
 */
export function MasterClock({
  positionTicks,
  ppq,
  timeSignature,
  tempoBpm,
  inCountIn,
  notePitch,
  onTempoChange,
}: MasterClockProps) {
  const [readout, setReadout] = useState<Readout>("bars");

  const nudge = (delta: number) => {
    const next = Math.round(tempoBpm) + delta;
    if (next >= MIN_TEMPO && next <= MAX_TEMPO) onTempoChange(next);
  };

  return (
    <div className="clock">
      <button
        className="clock__readout"
        onClick={() => setReadout((current) => (current === "bars" ? "note" : "bars"))}
        title={
          readout === "bars"
            ? "Bar · beat · tick — click to show the note at the playhead"
            : "The note at the playhead — click to show the position"
        }
      >
        <span className="clock__label">{readout === "bars" ? "Bar · Beat" : "Note"}</span>
        <span className="clock__value mono">
          {readout === "bars"
            ? formatPosition(positionTicks, ppq, timeSignature)
            : (notePitch === null ? "—" : noteName(notePitch))}
        </span>
      </button>

      <div className="clock__tempo">
        <span className="clock__label">Tempo</span>
        <div className="clock__stepper">
          <button
            className="clock__nudge"
            onClick={() => nudge(-TEMPO_STEP)}
            disabled={Math.round(tempoBpm) <= MIN_TEMPO}
            aria-label="Slower"
          >
            −
          </button>
          <input
            className="clock__bpm mono"
            type="number"
            min={MIN_TEMPO}
            max={MAX_TEMPO}
            step={TEMPO_STEP}
            value={Math.round(tempoBpm)}
            aria-label="Tempo in beats per minute"
            onChange={(e) => {
              const value = Number(e.target.value);
              if (Number.isFinite(value) && value >= MIN_TEMPO && value <= MAX_TEMPO) {
                onTempoChange(value);
              }
            }}
          />
          <button
            className="clock__nudge"
            onClick={() => nudge(TEMPO_STEP)}
            disabled={Math.round(tempoBpm) >= MAX_TEMPO}
            aria-label="Faster"
          >
            +
          </button>
        </div>
      </div>

      <div className="clock__sig">
        <span className="clock__label">Sig</span>
        <span className="clock__value clock__value--quiet mono">
          {timeSignature.numerator}/{timeSignature.denominator}
        </span>
      </div>

      {inCountIn && <span className="clock__countin">count-in</span>}
    </div>
  );
}
