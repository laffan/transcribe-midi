import "./Transport.css";

interface TransportProps {
  recording: boolean;
  loopRegion: [number, number] | null;
  metronome: boolean;
  dirty: boolean;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  onRecord: () => void;
  /** Audio-to-MIDI capture. A peer of MIDI record, not a panel. */
  listening: boolean;
  onListen: () => void;
  onToggleLoop: () => void;
  onToggleMetronome: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onSave: () => void;
  onPanic: () => void;
}

/**
 * The bar under the roll: the ways notes arrive, and what to do with the ones that are
 * already there.
 *
 * Play, the clock and the tempo moved to the toolbar, where the controls that concern the
 * whole project live. What is left is deliberately of one kind — Record and Listen are the
 * two ways a performance becomes notes, and the rest is the fate of an edit.
 */
export function Transport({
  recording,
  loopRegion,
  metronome,
  dirty,
  canUndo,
  canRedo,
  undoLabel,
  redoLabel,
  onRecord,
  listening,
  onListen,
  onToggleLoop,
  onToggleMetronome,
  onUndo,
  onRedo,
  onSave,
  onPanic,
}: TransportProps) {
  return (
    <div className="transport">
      <div className="transport__group">
        <button
          className={`btn btn--icon ${recording ? "btn--recording" : ""}`}
          onClick={onRecord}
          aria-label={recording ? "Stop recording" : "Record"}
          aria-pressed={recording}
          title={recording ? "Stop recording (R)" : "Record MIDI (R)"}
        >
          ●
        </button>
        <button
          className={`btn btn--listen ${listening ? "btn--listening" : ""}`}
          onClick={onListen}
          aria-pressed={listening}
          title="Listen — turn audio into notes (L)"
        >
          Listen
        </button>
      </div>

      <div className="transport__group">
        <button
          className={`btn btn--ghost ${loopRegion ? "btn--active" : ""}`}
          onClick={onToggleLoop}
          aria-pressed={loopRegion !== null}
          title={
            loopRegion
              ? "Looping — click to turn off"
              : "Loop the next two bars from the playhead"
          }
        >
          Loop
        </button>
        <button
          className={`btn btn--ghost ${metronome ? "btn--active" : ""}`}
          onClick={onToggleMetronome}
          aria-pressed={metronome}
          title="Metronome"
        >
          Click
        </button>
      </div>

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
