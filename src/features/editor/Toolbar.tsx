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

  playing: boolean;
  onPlay: () => void;
  onPause: () => void;
  onStop: () => void;

  positionTicks: number;
  ppq: number;
  timeSignature: TimeSignature;
  tempoBpm: number;
  inCountIn: boolean;
  notePitch: number | null;
  onTempoChange: (bpm: number) => void;

  onOpenSettings: () => void;
  showConsole: boolean;
  onToggleConsole: () => void;
  onImport: () => void;
  importing: boolean;
}

/**
 * The top-level controls: what is showing, what the transport is doing, what is open.
 *
 * Everything that is about the whole app rather than one note lives here, and the
 * transport lives here only. Play used to sit in the bottom bar beside Record and Listen,
 * which put "hear this project" in the same row as "make something new" — two different
 * kinds of action, and the reason the bottom bar kept growing. The bar below is now
 * exactly the ways notes come into being, plus what to do with the ones that exist.
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
  playing,
  onPlay,
  onPause,
  onStop,
  positionTicks,
  ppq,
  timeSignature,
  tempoBpm,
  inCountIn,
  notePitch,
  onTempoChange,
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
            className="btn btn--ghost btn--icon"
            onClick={onOpenSettings}
            aria-label="Settings"
            title="Settings"
          >
            ⚙
          </button>
        </div>
      </div>
    </header>
  );
}
