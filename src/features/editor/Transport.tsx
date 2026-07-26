import type { TimeSignature } from "../../lib/types";
import "./Transport.css";

interface TransportProps {
  playing: boolean;
  positionTicks: number;
  tempoBpm: number;
  ppq: number;
  timeSignature: TimeSignature;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  onPlay: () => void;
  onStop: () => void;
  onReturnToZero: () => void;
  onTempoChange: (bpm: number) => void;
  onUndo: () => void;
  onRedo: () => void;
  onSave: () => void;
  onPanic: () => void;
}

/** Bars|beats|ticks, one-based, the way every DAW displays position. */
function formatPosition(ticks: number, ppq: number, ts: TimeSignature): string {
  const beatTicks = (ppq * 4) / ts.denominator;
  const barTicks = beatTicks * ts.numerator;

  const bar = Math.floor(ticks / barTicks) + 1;
  const beat = Math.floor((ticks % barTicks) / beatTicks) + 1;
  const tick = Math.floor(ticks % beatTicks);

  return `${bar}.${beat}.${String(tick).padStart(3, "0")}`;
}

export function Transport({
  playing,
  positionTicks,
  tempoBpm,
  ppq,
  timeSignature,
  dirty,
  canUndo,
  canRedo,
  undoLabel,
  redoLabel,
  onPlay,
  onStop,
  onReturnToZero,
  onTempoChange,
  onUndo,
  onRedo,
  onSave,
  onPanic,
}: TransportProps) {
  return (
    <div className="transport">
      <div className="transport__group">
        <button
          className="btn btn--icon"
          onClick={onReturnToZero}
          aria-label="Return to zero"
          title="Return to start"
        >
          ⏮
        </button>
        <button
          className={`btn btn--icon ${playing ? "btn--active" : ""}`}
          onClick={playing ? onStop : onPlay}
          aria-label={playing ? "Stop" : "Play"}
          title={playing ? "Stop (Space)" : "Play (Space)"}
        >
          {playing ? "⏹" : "▶"}
        </button>
      </div>

      <div className="transport__position mono" aria-label="Playhead position">
        {formatPosition(positionTicks, ppq, timeSignature)}
      </div>

      <label className="transport__tempo">
        <span className="transport__label">BPM</span>
        <input
          className="input transport__tempo-input mono"
          type="number"
          min={20}
          max={300}
          step={1}
          value={Math.round(tempoBpm)}
          onChange={(e) => {
            const value = Number(e.target.value);
            if (Number.isFinite(value) && value >= 20 && value <= 300) onTempoChange(value);
          }}
        />
      </label>

      <span className="transport__sig mono muted">
        {timeSignature.numerator}/{timeSignature.denominator}
      </span>

      <div className="spacer" />

      <div className="transport__group">
        <button
          className="btn btn--ghost"
          onClick={onUndo}
          disabled={!canUndo}
          title={undoLabel ? `Undo ${undoLabel}` : "Nothing to undo"}
        >
          Undo
        </button>
        <button
          className="btn btn--ghost"
          onClick={onRedo}
          disabled={!canRedo}
          title={redoLabel ? `Redo ${redoLabel}` : "Nothing to redo"}
        >
          Redo
        </button>
      </div>

      <button
        className="btn btn--ghost"
        onClick={onPanic}
        title="Silence all notes — use if a note gets stuck"
      >
        Panic
      </button>

      <button className={`btn ${dirty ? "btn--primary" : ""}`} onClick={onSave} disabled={!dirty}>
        {dirty ? "Save" : "Saved"}
      </button>
    </div>
  );
}
