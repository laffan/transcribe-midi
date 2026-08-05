import type { BuildInfo, TimeSignature } from "../../lib/types";
import { MasterClock } from "./MasterClock";
import "./Toolbar.css";

interface ToolbarProps {
  projectName: string;
  dirty: boolean;
  build: BuildInfo | null;
  onBack: () => void;

  showKeyboard: boolean;
  onToggleKeyboard: () => void;
  showHistory: boolean;
  onToggleHistory: () => void;

  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  redoLabel: string | null;
  onUndo: () => void;
  onRedo: () => void;

  playing: boolean;
  onPlay: () => void;
  onPause: () => void;
  onStop: () => void;
  recording: boolean;
  onRecord: () => void;
  /** Audio-to-MIDI capture. A peer of Record, not a panel. */
  listening: boolean;
  onListen: () => void;
  loopRegion: [number, number] | null;
  onToggleLoop: () => void;
  metronome: boolean;
  onToggleMetronome: () => void;

  positionTicks: number;
  ppq: number;
  timeSignature: TimeSignature;
  tempoBpm: number;
  inCountIn: boolean;
  notePitch: number | null;
  onTempoChange: (bpm: number) => void;

  onPanic: () => void;
  onSave: () => void;
  onOpenSettings: () => void;
  showConsole: boolean;
  onToggleConsole: () => void;
  onImport: () => void;
  importing: boolean;
}

/**
 * Every top-level control, in one bar.
 *
 * The bottom bar used to hold the transport; then it held what was left of it. Splitting
 * the controls across two rows only ever answered "what was built when" — a user hunting
 * for Loop does not know that it arrived with the transport rather than with the toolbar.
 * So they are all here, sorted by what they do to the project:
 *
 * - **Left**: what is on screen, and the history you can walk back through.
 * - **Centre**: the transport, everything that makes notes, and the clock they run on.
 * - **Right**: what is open, and what happens to the result.
 *
 * What is left below the roll is the prompt bar and the keys — the two things you *type*
 * into, which is a different kind of surface from a button.
 */
export function Toolbar({
  projectName,
  dirty,
  build,
  onBack,
  showKeyboard,
  onToggleKeyboard,
  showHistory,
  onToggleHistory,
  canUndo,
  canRedo,
  undoLabel,
  redoLabel,
  onUndo,
  onRedo,
  playing,
  onPlay,
  onPause,
  onStop,
  recording,
  onRecord,
  listening,
  onListen,
  loopRegion,
  onToggleLoop,
  metronome,
  onToggleMetronome,
  positionTicks,
  ppq,
  timeSignature,
  tempoBpm,
  inCountIn,
  notePitch,
  onTempoChange,
  onPanic,
  onSave,
  onOpenSettings,
  showConsole,
  onToggleConsole,
  onImport,
  importing,
}: ToolbarProps) {
  return (
    <header className="toolbar">
      <div className="toolbar__side">
        <button className="btn btn--ghost" onClick={onBack} title="Back to the project list">
          ← Projects
        </button>

        <span className="toolbar__project truncate" title={projectName}>
          {projectName}
          {dirty && <span className="toolbar__dirty" aria-label="Unsaved changes">•</span>}
        </span>

        <div className="toolbar__group">
          <button
            className="btn btn--ghost btn--icon"
            onClick={onUndo}
            disabled={!canUndo}
            aria-label="Undo"
            title={undoLabel ? `Undo ${undoLabel} (⌘Z)` : "Nothing to undo"}
          >
            ↶
          </button>
          <button
            className="btn btn--ghost btn--icon"
            onClick={onRedo}
            disabled={!canRedo}
            aria-label="Redo"
            title={redoLabel ? `Redo ${redoLabel} (⇧⌘Z)` : "Nothing to redo"}
          >
            ↷
          </button>
        </div>

        <div className="toolbar__group">
          <button
            className={`btn btn--ghost ${showKeyboard ? "btn--active" : ""}`}
            onClick={onToggleKeyboard}
            aria-pressed={showKeyboard}
            title="Show or hide the on-screen keyboard"
          >
            Keys
          </button>
          <button
            className={`btn btn--ghost ${showHistory ? "btn--active" : ""}`}
            onClick={onToggleHistory}
            aria-pressed={showHistory}
            title="Show or hide the history panel"
          >
            History
          </button>
        </div>
      </div>

      <div className="toolbar__centre">
        <div className="toolbar__group">
          <button
            className={`btn btn--icon btn--transport ${playing ? "btn--active" : ""}`}
            onClick={onPlay}
            aria-label="Play"
            title="Play (Space)"
          >
            ▶
          </button>
          <button
            className="btn btn--icon btn--transport"
            onClick={onPause}
            disabled={!playing}
            aria-label="Pause"
            title="Pause — stay where you are"
          >
            ⏸
          </button>
          <button
            className="btn btn--icon btn--transport"
            onClick={onStop}
            aria-label="Stop"
            title="Stop and return to the start"
          >
            ⏹
          </button>
          <button
            className={`btn btn--icon btn--transport ${recording ? "btn--recording" : ""}`}
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

        <div className="toolbar__group">
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

        <MasterClock
          positionTicks={positionTicks}
          ppq={ppq}
          timeSignature={timeSignature}
          tempoBpm={tempoBpm}
          inCountIn={inCountIn}
          notePitch={notePitch}
          onTempoChange={onTempoChange}
        />
      </div>

      <div className="toolbar__side toolbar__side--end">
        {build && (
          <span
            className={`toolbar__build mono ${build.dirty ? "toolbar__build--dirty" : ""}`}
            title={`${build.version} · ${build.commit}${build.dirty ? " (modified)" : ""} · ${build.profile} · built ${build.built_at}`}
          >
            {build.version} {build.commit}
            {build.dirty ? "+" : ""}
          </span>
        )}

        <div className="toolbar__group">
          <button
            className="btn btn--ghost"
            onClick={onImport}
            disabled={importing}
            title="Import a MIDI file into this project"
          >
            {importing ? "Importing…" : "Import"}
          </button>
          <button
            className={`btn btn--ghost ${showConsole ? "btn--active" : ""}`}
            onClick={onToggleConsole}
            aria-pressed={showConsole}
            title="Show or hide the console"
          >
            Console
          </button>
          <button
            className="btn btn--ghost btn--icon toolbar__gear"
            onClick={onOpenSettings}
            aria-label="Settings"
            title="Settings"
          >
            ⚙
          </button>
        </div>

        <div className="toolbar__group">
          <button
            className="btn btn--ghost"
            onClick={onPanic}
            title="Silence all notes — use if a note gets stuck"
          >
            Panic
          </button>
          <button
            className={`btn ${dirty ? "btn--primary" : ""}`}
            onClick={onSave}
            disabled={!dirty}
            title={dirty ? "Save the project (⌘S)" : "Nothing to save"}
          >
            {dirty ? "Save" : "Saved"}
          </button>
        </div>
      </div>
    </header>
  );
}
