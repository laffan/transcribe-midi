import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, onPlayhead } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, EditRequest, ProjectManifest } from "../../lib/types";
import { ConsolePanel } from "./ConsolePanel";
import { OnScreenKeyboard } from "./OnScreenKeyboard";
import { PianoRoll } from "./PianoRoll";
import { Transport } from "./Transport";
import "./Editor.css";

interface EditorProps {
  projectId: string;
  onClose: () => void;
  onOpenSettings: () => void;
}

/** Velocity used by the on-screen keyboard. Becomes a setting in Phase 4. */
const KEYBOARD_VELOCITY = 100;

export function Editor({ projectId, onClose, onOpenSettings }: EditorProps) {
  const [manifest, setManifest] = useState<ProjectManifest | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [selectedTrack, setSelectedTrack] = useState(0);
  const [selection, setSelection] = useState<number[]>([]);
  const [playing, setPlaying] = useState(false);
  const [positionTicks, setPositionTicks] = useState(0);
  const [tempo, setTempo] = useState(120);
  const [showConsole, setShowConsole] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  // -- load ----------------------------------------------------------------

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const [project, state] = await Promise.all([
          api.loadProject(projectId),
          api.openProject(projectId),
        ]);
        if (cancelled) return;
        setManifest(project.manifest);
        setTempo(project.manifest.tempo_bpm);
        setEditor(state);
        setLoadError(null);
      } catch (error) {
        if (cancelled) return;
        const message = errorMessage(error);
        setLoadError(message);
        logger.error(`Could not open project "${projectId}"`, message);
      }
    })();

    return () => {
      cancelled = true;
      void api.closeProject().catch(() => {});
    };
  }, [projectId]);

  // -- playhead ------------------------------------------------------------

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    onPlayhead((event) => {
      setPositionTicks(event.position_ticks);
      setPlaying(event.playing);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error) => logger.error("Could not subscribe to the playhead", errorMessage(error)));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // -- editing -------------------------------------------------------------

  const applyEdit = useCallback(async (request: EditRequest) => {
    try {
      const state = await api.applyEdit(request);
      setEditor(state);
      // Rust reports where the edited notes ended up; indices shift when notes reorder,
      // so the selection is re-derived from that rather than kept locally.
      if (state.affected.length > 0) setSelection(state.affected);
    } catch (error) {
      logger.error("Edit failed", errorMessage(error));
    }
  }, []);

  const doUndo = useCallback(async () => {
    try {
      setEditor(await api.undo());
      setSelection([]);
    } catch (error) {
      logger.error("Undo failed", errorMessage(error));
    }
  }, []);

  const doRedo = useCallback(async () => {
    try {
      setEditor(await api.redo());
      setSelection([]);
    } catch (error) {
      logger.error("Redo failed", errorMessage(error));
    }
  }, []);

  const doSave = useCallback(async () => {
    try {
      setEditor(await api.saveOpenProject());
      logger.info("Project saved");
    } catch (error) {
      logger.error("Save failed", errorMessage(error));
    }
  }, []);

  // -- transport -----------------------------------------------------------

  const play = useCallback(async () => {
    try {
      const state = await api.transportPlay();
      setPlaying(state.playing);
    } catch (error) {
      logger.error("Could not start playback", errorMessage(error));
    }
  }, []);

  const stop = useCallback(async () => {
    try {
      const state = await api.transportStop();
      setPlaying(state.playing);
    } catch (error) {
      logger.error("Could not stop playback", errorMessage(error));
    }
  }, []);

  const seek = useCallback(async (tick: number) => {
    try {
      const state = await api.transportSeek(tick);
      setPositionTicks(state.position_ticks);
    } catch (error) {
      logger.error("Could not move the playhead", errorMessage(error));
    }
  }, []);

  // -- global shortcuts ----------------------------------------------------

  const playingRef = useRef(playing);
  playingRef.current = playing;

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }

      const mod = event.metaKey || event.ctrlKey;

      if (event.code === "Space") {
        event.preventDefault();
        void (playingRef.current ? stop() : play());
        return;
      }

      if (mod && event.key.toLowerCase() === "z") {
        event.preventDefault();
        void (event.shiftKey ? doRedo() : doUndo());
        return;
      }

      if (mod && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void doSave();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [play, stop, doUndo, doRedo, doSave]);

  // -- live notes ----------------------------------------------------------

  const track = editor?.tracks[selectedTrack] ?? null;

  const noteOn = useCallback(
    (pitch: number, velocity: number) => {
      void api
        .liveNoteOn(selectedTrack, pitch, velocity, track?.channel ?? 0)
        .catch((error) => logger.error("Note failed", errorMessage(error)));
    },
    [selectedTrack, track?.channel],
  );

  const noteOff = useCallback(
    (pitch: number) => {
      void api.liveNoteOff(selectedTrack, pitch, track?.channel ?? 0).catch(() => {});
    },
    [selectedTrack, track?.channel],
  );

  const previewNote = useCallback(
    (pitch: number) => {
      noteOn(pitch, KEYBOARD_VELOCITY);
      // Auditioning a note in the roll should be a blip, not a held tone.
      window.setTimeout(() => noteOff(pitch), 180);
    },
    [noteOn, noteOff],
  );

  // -- render --------------------------------------------------------------

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

  if (!manifest || !editor) {
    return <div className="editor__failure muted">Loading…</div>;
  }

  async function addTrack() {
    try {
      await api.addTrack(manifest!.id);
      const state = await api.openProject(manifest!.id);
      setEditor(state);
      setSelectedTrack(state.tracks.length - 1);
      setSelection([]);
      logger.info("Added track");
    } catch (error) {
      logger.error("Could not add track", errorMessage(error));
    }
  }

  async function deleteTrack(trackId: string) {
    if (editor!.tracks.length <= 1) {
      logger.warn("A project must keep at least one track");
      return;
    }
    if (editor!.dirty) {
      logger.warn("Save before removing a track — unsaved edits would be lost");
      return;
    }
    try {
      await api.deleteTrack(manifest!.id, trackId);
      const state = await api.openProject(manifest!.id);
      setEditor(state);
      setSelectedTrack(0);
      setSelection([]);
      logger.info("Deleted track");
    } catch (error) {
      logger.error("Could not delete track", errorMessage(error));
    }
  }

  return (
    <div className={`editor ${showConsole ? "editor--console" : ""}`}>
      <header className="editor__titlebar">
        <button className="btn btn--ghost btn--icon" onClick={onOpenSettings} aria-label="Settings" title="Settings">
          ⚙
        </button>
        <button className="btn btn--ghost" onClick={onClose}>
          ← Projects
        </button>

        <div className="editor__title truncate">
          {manifest.name}
          {editor.dirty && <span className="editor__dirty" aria-label="Unsaved changes">•</span>}
        </div>

        <div className="spacer" />

        <span className="editor__stat mono">{manifest.ppq} PPQ</span>
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
            {editor.tracks.map((t, index) => (
              <li key={t.id}>
                <div
                  className={`tracklist__item ${index === selectedTrack ? "tracklist__item--selected" : ""}`}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    setSelectedTrack(index);
                    setSelection([]);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setSelectedTrack(index);
                      setSelection([]);
                    }
                  }}
                >
                  <span className="tracklist__swatch" style={{ background: t.color }} />
                  <span className="tracklist__name truncate">{t.name}</span>
                  <span className="tracklist__count mono muted">{t.notes.length}</span>
                  {editor.tracks.length > 1 && (
                    <button
                      className="btn btn--ghost btn--icon tracklist__remove"
                      onClick={(e) => {
                        e.stopPropagation();
                        void deleteTrack(t.id);
                      }}
                      aria-label={`Delete ${t.name}`}
                    >
                      ✕
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>

          <div className="editor__roll">
            {track ? (
              <PianoRoll
                track={track}
                trackIndex={selectedTrack}
                ppq={manifest.ppq}
                timeSignature={manifest.time_signature}
                selection={selection}
                onSelectionChange={setSelection}
                onEdit={applyEdit}
                onPreviewNote={previewNote}
                playheadTicks={positionTicks}
                onScrub={seek}
              />
            ) : (
              <p className="muted">No track selected.</p>
            )}
          </div>
        </section>

        <aside className="editor__inspector">
          <div className="editor__panel-head">
            <h2 className="editor__panel-title">Track Inspector</h2>
          </div>

          {track ? (
            <div className="inspector">
              <div className="field">
                <span className="field__label">Name</span>
                <div className="inspector__value">{track.name}</div>
              </div>

              <div className="inspector__grid">
                <div className="field">
                  <span className="field__label">Channel</span>
                  <div className="inspector__value mono">{track.channel + 1}</div>
                </div>
                <div className="field">
                  <span className="field__label">Notes</span>
                  <div className="inspector__value mono">{track.notes.length}</div>
                </div>
              </div>

              <div className="field">
                <span className="field__label">Selected</span>
                <div className="inspector__value mono">{selection.length}</div>
              </div>

              <div className="field">
                <span className="field__label">Instrument</span>
                <div className="inspector__value">Built-in sampler</div>
                <span className="field__hint">AUv3 instruments arrive in Phase 9.</span>
              </div>

              <hr className="inspector__rule" />

              <div className="inspector__shortcuts">
                <span className="field__label">Shortcuts</span>
                <dl className="shortcuts">
                  <dt className="mono">Space</dt><dd>Play / stop</dd>
                  <dt className="mono">⌘Z / ⇧⌘Z</dt><dd>Undo / redo</dd>
                  <dt className="mono">⌘A</dt><dd>Select all</dd>
                  <dt className="mono">⌘C / ⌘X / ⌘V</dt><dd>Copy / cut / paste</dd>
                  <dt className="mono">⌘Q</dt><dd>Quantize selection</dd>
                  <dt className="mono">⌫</dt><dd>Delete selection</dd>
                  <dt className="mono">↑ ↓ ← →</dt><dd>Nudge (⇧ for octave / bar)</dd>
                  <dt className="mono">⌥click</dt><dd>Delete note</dd>
                  <dt className="mono">A–L, W/E/T/Y/U</dt><dd>Play keys</dd>
                  <dt className="mono">Z / X</dt><dd>Octave down / up</dd>
                </dl>
              </div>

              <hr className="inspector__rule" />

              <div className="placeholder placeholder--compact">
                <span className="placeholder__phase">Phase 6</span>
                <span className="placeholder__title">AI prompt</span>
                <span className="placeholder__detail">
                  Describe an edit against the current selection; the change previews as a diff
                  before it is applied.
                </span>
              </div>
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
          <Transport
            playing={playing}
            positionTicks={positionTicks}
            tempoBpm={tempo}
            ppq={manifest.ppq}
            timeSignature={manifest.time_signature}
            dirty={editor.dirty}
            canUndo={editor.can_undo}
            canRedo={editor.can_redo}
            undoLabel={editor.undo_label}
            redoLabel={editor.redo_label}
            onPlay={play}
            onStop={stop}
            onReturnToZero={() => void seek(0)}
            onTempoChange={(bpm) => {
              setTempo(bpm);
              void api.setTempo(bpm).catch((e) => logger.error("Tempo change failed", errorMessage(e)));
            }}
            onUndo={doUndo}
            onRedo={doRedo}
            onSave={doSave}
            onPanic={() => void api.panic().catch(() => {})}
          />
        </div>
        <div className="editor__keyboard">
          <OnScreenKeyboard
            velocity={KEYBOARD_VELOCITY}
            channel={track?.channel ?? 0}
            onNoteOn={noteOn}
            onNoteOff={noteOff}
          />
        </div>
      </footer>

      {showConsole && <ConsolePanel onClose={() => setShowConsole(false)} />}
    </div>
  );
}
