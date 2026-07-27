import type { BuildInfo } from "../../lib/types";

interface EditorTitlebarProps {
  name: string;
  ppq: number;
  dirty: boolean;
  build: BuildInfo | null;
  showConsole: boolean;
  onToggleConsole: () => void;
  onClose: () => void;
  onOpenSettings: () => void;
}

/**
 * Project name, build stamp, and the two ways out.
 *
 * The build stamp is here rather than only in Settings because once this runs as a plugin
 * this window is the entire UI — and "am I looking at the fix I just built?" is a question
 * a cached Audio Unit scan makes impossible to answer any other way.
 */
export function EditorTitlebar({
  name,
  ppq,
  dirty,
  build,
  showConsole,
  onToggleConsole,
  onClose,
  onOpenSettings,
}: EditorTitlebarProps) {
  return (
    <header className="editor__titlebar">
      <button
        className="btn btn--ghost btn--icon"
        onClick={onOpenSettings}
        aria-label="Settings"
        title="Settings"
      >
        ⚙
      </button>
      <button className="btn btn--ghost" onClick={onClose}>
        ← Projects
      </button>

      <div className="editor__title truncate">
        {name}
        {dirty && (
          <span className="editor__dirty" aria-label="Unsaved changes">
            •
          </span>
        )}
      </div>

      <div className="spacer" />

      <span className="editor__stat mono">{ppq} PPQ</span>
      {build && (
        <span
          className={`editor__stat mono ${build.dirty ? "editor__build--dirty" : ""}`}
          title={`${build.version} · ${build.commit}${build.dirty ? " (modified)" : ""} · ${build.profile} · built ${build.built_at}`}
        >
          {build.version} {build.commit}
          {build.dirty ? "+" : ""}
        </span>
      )}
      <button
        className={`btn btn--ghost ${showConsole ? "btn--active" : ""}`}
        onClick={onToggleConsole}
        aria-pressed={showConsole}
      >
        Console
      </button>
    </header>
  );
}
