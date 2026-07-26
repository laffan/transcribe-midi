import { useCallback, useEffect, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { Project } from "../../lib/types";
import { ConsolePanel } from "./ConsolePanel";
import "./Editor.css";

interface EditorProps {
  projectId: string;
  onClose: () => void;
  onOpenSettings: () => void;
}

export function Editor({ projectId, onClose, onOpenSettings }: EditorProps) {
  const [project, setProject] = useState<Project | null>(null);
  const [selectedTrackId, setSelectedTrackId] = useState<string | null>(null);
  const [showConsole, setShowConsole] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const loaded = await api.loadProject(projectId);
      setProject(loaded);
      setSelectedTrackId((current) =>
        current && loaded.tracks.some((t) => t.id === current) ? current : loaded.tracks[0]?.id ?? null,
      );
      setLoadError(null);
    } catch (error) {
      const message = errorMessage(error);
      setLoadError(message);
      logger.error(`Could not open project "${projectId}"`, message);
    }
  }, [projectId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function addTrack() {
    if (!project) return;
    try {
      const meta = await api.addTrack(project.manifest.id);
      logger.info(`Added ${meta.name}`);
      await reload();
      setSelectedTrackId(meta.id);
    } catch (error) {
      logger.error("Could not add track", errorMessage(error));
    }
  }

  async function deleteTrack(trackId: string) {
    if (!project) return;
    if (project.tracks.length <= 1) {
      logger.warn("A project must keep at least one track");
      return;
    }
    try {
      await api.deleteTrack(project.manifest.id, trackId);
      logger.info("Deleted track");
      await reload();
    } catch (error) {
      logger.error("Could not delete track", errorMessage(error));
    }
  }

  if (loadError) {
    return (
      <div className="editor__failure">
        <h2>Could not open this project</h2>
        <p className="muted">{loadError}</p>
        <button className="btn" onClick={onClose}>
          Back to Projects
        </button>
      </div>
    );
  }

  if (!project) {
    return <div className="editor__failure muted">Loading…</div>;
  }

  const { manifest } = project;
  const selectedTrack = project.tracks.find((t) => t.id === selectedTrackId) ?? null;

  return (
    <div className={`editor ${showConsole ? "editor--console" : ""}`}>
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

        <div className="editor__title truncate">{manifest.name}</div>

        <div className="spacer" />

        <span className="editor__stat mono">
          {manifest.tempo_bpm} BPM · {manifest.time_signature.numerator}/
          {manifest.time_signature.denominator} · {manifest.ppq} PPQ
        </span>
        <button
          className={`btn btn--ghost ${showConsole ? "btn--active" : ""}`}
          onClick={() => setShowConsole((v) => !v)}
          aria-pressed={showConsole}
        >
          Console
        </button>
      </header>

      <main className="editor__main">
        <section className="editor__tracks">
          <div className="editor__panel-head">
            <h2 className="editor__panel-title">Tracks</h2>
            <button className="btn btn--ghost" onClick={addTrack}>
              + Add
            </button>
          </div>

          <ul className="tracklist">
            {project.tracks.map((track) => (
              <li key={track.id}>
                <div
                  className={`tracklist__item ${track.id === selectedTrackId ? "tracklist__item--selected" : ""}`}
                  role="button"
                  tabIndex={0}
                  onClick={() => setSelectedTrackId(track.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelectedTrackId(track.id);
                    }
                  }}
                >
                  <span className="tracklist__swatch" style={{ background: track.color }} />
                  <span className="tracklist__name truncate">{track.name}</span>
                  <span className="tracklist__count mono muted">{track.notes.length}</span>
                  {project.tracks.length > 1 && (
                    <button
                      className="btn btn--ghost btn--icon tracklist__remove"
                      onClick={(e) => {
                        e.stopPropagation();
                        void deleteTrack(track.id);
                      }}
                      aria-label={`Delete ${track.name}`}
                    >
                      ✕
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>

          <div className="editor__roll">
            <Placeholder
              phase="Phase 3"
              title="Piano roll"
              detail="Draw, drag, resize, multi-select, grid snap and velocity editing land here, on top of the undoable command layer."
            />
          </div>
        </section>

        <aside className="editor__inspector">
          <div className="editor__panel-head">
            <h2 className="editor__panel-title">Track Inspector</h2>
          </div>

          {selectedTrack ? (
            <div className="inspector">
              <div className="field">
                <span className="field__label">Name</span>
                <div className="inspector__value">{selectedTrack.name}</div>
              </div>

              <div className="inspector__grid">
                <div className="field">
                  <span className="field__label">Channel</span>
                  <div className="inspector__value mono">{selectedTrack.channel + 1}</div>
                </div>
                <div className="field">
                  <span className="field__label">Notes</span>
                  <div className="inspector__value mono">{selectedTrack.notes.length}</div>
                </div>
              </div>

              <div className="field">
                <span className="field__label">Instrument</span>
                <div className="inspector__value">Built-in sampler</div>
                <span className="field__hint">AUv3 instruments arrive in Phase 9.</span>
              </div>

              <hr className="inspector__rule" />

              <Placeholder
                phase="Phase 6"
                title="AI prompt"
                detail="Describe an edit against the current selection; the change previews as a diff before it is applied."
                variant="compact"
              />
            </div>
          ) : (
            <p className="muted" style={{ padding: "var(--space-4)" }}>
              No track selected.
            </p>
          )}
        </aside>
      </main>

      <footer className="editor__bottom">
        <div className="editor__transport">
          <Placeholder
            phase="Phase 2"
            title="Transport"
            detail="Play / stop / record, loop brackets, count-in and metronome."
            variant="inline"
          />
        </div>
        <div className="editor__keyboard">
          <Placeholder
            phase="Phase 2"
            title="Two-octave keyboard"
            detail="Touch, click and Logic-style computer-keyboard mapping (A–L white, W/E/T/Y/U black, Z/X octave)."
            variant="inline"
          />
        </div>
      </footer>

      {showConsole && <ConsolePanel onClose={() => setShowConsole(false)} />}
    </div>
  );
}

/**
 * Marks a panel that a later phase fills in.
 *
 * `block` centres in a large empty area, `compact` stacks left-aligned in a sidebar, and
 * `inline` lays out on one row for the fixed-height bottom bar, where a wrapping
 * description would be clipped.
 */
function Placeholder({
  phase,
  title,
  detail,
  variant = "block",
}: {
  phase: string;
  title: string;
  detail: string;
  variant?: "block" | "compact" | "inline";
}) {
  return (
    <div className={`placeholder placeholder--${variant}`}>
      <span className="placeholder__phase">{phase}</span>
      <span className="placeholder__title">{title}</span>
      <span className="placeholder__detail">{detail}</span>
    </div>
  );
}
