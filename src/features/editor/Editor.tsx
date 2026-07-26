import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, onLiveNote, onPlayhead } from "../../lib/api";
import { logger } from "../../lib/console";
import type {
  AiProposal,
  EditorState,
  EditRequest,
  Note,
  NoteDiff,
  PlatformCapabilities,
  ProjectManifest,
} from "../../lib/types";
import { AiPanel } from "./AiPanel";
import { InterchangeBar } from "./InterchangeBar";
import { TranscribePanel } from "./TranscribePanel";
import { ConsolePanel } from "./ConsolePanel";
import { OnScreenKeyboard } from "./OnScreenKeyboard";
import { PianoRoll } from "./PianoRoll";
import { Transport } from "./Transport";
import "./Editor.css";

interface EditorProps {
  projectId: string;
  /** Changes when Settings closes, so input settings are re-read. */
  settingsRevision: number;
  onClose: () => void;
  onOpenSettings: () => void;
}

export function Editor({ projectId, settingsRevision, onClose, onOpenSettings }: EditorProps) {
  const [manifest, setManifest] = useState<ProjectManifest | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [selectedTrack, setSelectedTrack] = useState(0);
  const [selection, setSelection] = useState<number[]>([]);
  const [playing, setPlaying] = useState(false);
  const [positionTicks, setPositionTicks] = useState(0);
  const [tempo, setTempo] = useState(120);
  const [showConsole, setShowConsole] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);
  const [inCountIn, setInCountIn] = useState(false);
  const [loopRegion, setLoopRegion] = useState<[number, number] | null>(null);
  const [metronome, setMetronome] = useState(false);
  const [capabilities, setCapabilities] = useState<PlatformCapabilities | null>(null);
  /** Pitches currently sounding from live input, for keyboard feedback. */
  const [liveNotes, setLiveNotes] = useState<Set<number>>(new Set());
  /** Mirror of the Rust-side setting, for display only — Rust applies it. */
  const [keyboardVelocity, setKeyboardVelocity] = useState(100);
  /** An AI proposal being previewed on the roll. Null when there is nothing pending. */
  const [aiPreview, setAiPreview] = useState<AiProposal | null>(null);
  /** Transcribed notes awaiting review. Drawn on the roll the same way. */
  const [transcribePreview, setTranscribePreview] = useState<Note[] | null>(null);

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
      setInCountIn(event.in_count_in);
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

  // Live-input feedback. Purely cosmetic — the note has already sounded and, if the
  // transport is recording, been captured in Rust by the time this arrives. It fires
  // for the on-screen keyboard too, because both go through the same Rust path.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    onLiveNote((event) => {
      setLiveNotes((prev) => {
        const next = new Set(prev);
        if (event.on) next.add(event.pitch);
        else next.delete(event.pitch);
        return next;
      });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    api.platformCapabilities().then(setCapabilities).catch(() => setCapabilities(null));
  }, []);

  useEffect(() => {
    api
      .inputSettings()
      .then((settings) => {
        setKeyboardVelocity(settings.keyboard_velocity);
        setMetronome(settings.metronome);
      })
      .catch(() => {});
  }, [settingsRevision]);

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

  const toggleRecord = useCallback(async () => {
    try {
      if (recording) {
        setEditor(await api.recordStop());
        setRecording(false);
        logger.info("Take committed — undo it like any other edit");
      } else {
        const result = await api.recordStart();
        setRecording(result.recording);
        if (result.count_in_ticks > 0) logger.info("Counting in…");
      }
    } catch (error) {
      setRecording(false);
      logger.error("Recording failed", errorMessage(error));
    }
  }, [recording]);

  const toggleLoop = useCallback(async () => {
    // Default to two bars from the playhead: a loop has to come from somewhere, and
    // dragging brackets before there is anything to loop is more ceremony than it is
    // worth. Once set, the region is visible in the ruler.
    const next: [number, number] | null = loopRegion
      ? null
      : (() => {
          const bar = (manifest!.ppq * 4 * manifest!.time_signature.numerator) /
            manifest!.time_signature.denominator;
          const start = Math.floor(positionTicks / bar) * bar;
          return [start, start + bar * 2];
        })();

    try {
      const state = await api.setLoopRegion(next);
      setLoopRegion(state.loop_region);
    } catch (error) {
      logger.error("Could not set the loop", errorMessage(error));
    }
  }, [loopRegion, manifest, positionTicks]);

  const toggleMetronome = useCallback(async () => {
    const next = !metronome;
    setMetronome(next);
    try {
      await api.setMetronome(next);
    } catch (error) {
      setMetronome(!next);
      logger.error("Could not toggle the metronome", errorMessage(error));
    }
  }, [metronome]);

  // Keep the armed track in step with the selected one, so recording lands where the
  // user is looking rather than on whichever track was armed last.
  useEffect(() => {
    if (recording) return;
    api.setArmedTrack(selectedTrack).catch(() => {});
  }, [selectedTrack, recording]);

  // -- global shortcuts ----------------------------------------------------

  const playingRef = useRef(playing);
  playingRef.current = playing;
  // Held in a ref so the key handler does not need re-binding on every state change.
  const toggleRecordRef = useRef(toggleRecord);
  toggleRecordRef.current = toggleRecord;

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
        return;
      }

      if (!mod && event.key.toLowerCase() === "r") {
        event.preventDefault();
        void toggleRecordRef.current();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [play, stop, doUndo, doRedo, doSave]);

  // -- live notes ----------------------------------------------------------

  const track = editor?.tracks[selectedTrack] ?? null;

  // No track argument: Rust routes live input to the armed track, and the effect above
  // keeps that in step with the selected one. Velocity is left to Rust so the Settings
  // value is the single source of truth.
  const noteOn = useCallback(
    (pitch: number) => {
      void api
        .liveNoteOn(pitch, track?.channel ?? 0)
        .catch((error) => logger.error("Note failed", errorMessage(error)));
    },
    [track?.channel],
  );

  const noteOff = useCallback(
    (pitch: number) => {
      void api.liveNoteOff(pitch, track?.channel ?? 0).catch(() => {});
    },
    [track?.channel],
  );

  const previewNote = useCallback(
    (pitch: number) => {
      noteOn(pitch);
      // Auditioning a note in the roll should be a blip, not a held tone.
      window.setTimeout(() => noteOff(pitch), 180);
    },
    [noteOn, noteOff],
  );

  // Both previews are the same thing to the roll: notes that would exist if accepted.
  // Only one can be open at a time — each panel clears the other's by replacing it.
  const rollPreview: NoteDiff | null = aiPreview
    ? aiPreview.diff
    : transcribePreview
      ? { added: transcribePreview, removed: [], changed: [] }
      : null;

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
                loopRegion={loopRegion}
                preview={rollPreview}
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

              <InterchangeBar
                trackIndex={selectedTrack}
                trackName={track.name}
                capabilities={capabilities}
                onImported={(state) => {
                  setEditor(state);
                  setSelection([]);
                }}
              />

              <hr className="inspector__rule" />

              <TranscribePanel
                trackIndex={selectedTrack}
                ppq={manifest.ppq}
                onApplied={(state) => {
                  setEditor(state);
                  setSelection(state.affected);
                }}
                onPreviewChange={setTranscribePreview}
              />

              <hr className="inspector__rule" />

              <AiPanel
                trackIndex={selectedTrack}
                selection={selection}
                settingsRevision={settingsRevision}
                onApplied={(state) => {
                  setEditor(state);
                  setSelection(state.affected);
                }}
                onPreviewChange={setAiPreview}
                onOpenSettings={onOpenSettings}
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
          <Transport
            playing={playing}
            recording={recording}
            inCountIn={inCountIn}
            loopRegion={loopRegion}
            metronome={metronome}
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
            onRecord={toggleRecord}
            onToggleLoop={toggleLoop}
            onToggleMetronome={toggleMetronome}
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
            velocity={keyboardVelocity}
            channel={track?.channel ?? 0}
            onNoteOn={noteOn}
            onNoteOff={noteOff}
            externalNotes={liveNotes}
          />
        </div>
      </footer>

      {showConsole && <ConsolePanel onClose={() => setShowConsole(false)} />}
    </div>
  );
}
