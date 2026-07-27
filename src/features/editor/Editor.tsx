import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, errorMessage, onLiveNote } from "../../lib/api";
import { logger } from "../../lib/console";
import type {
  BuildInfo,
  EditorState,
  EditRequest,
  PlatformCapabilities,
  ProjectManifest,
  TranscriptionPreview,
} from "../../lib/types";
import { ConsolePanel } from "./ConsolePanel";
import { importSmfFile } from "./importSmf";
import { Inspector } from "./Inspector";
import { ListenCapture } from "./ListenCapture";
import { ListenOverlay } from "./ListenOverlay";
import { OnScreenKeyboard } from "./OnScreenKeyboard";
import { previewDiff } from "./pending";
import { PianoRoll } from "./PianoRoll";
import { PromptBar } from "./PromptBar";
import { ReviewBar } from "./ReviewBar";
import { StartHere } from "./StartHere";
import { noteAt } from "./timeFormat";
import { Toolbar } from "./Toolbar";
import { TrackList } from "./TrackList";
import { TranscribeEditor } from "./TranscribeEditor";
import { Transport } from "./Transport";
import { useTranscription } from "./useTranscription";
import { useTransport } from "./useTransport";
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
  const [loadError, setLoadError] = useState<string | null>(null);
  const [capabilities, setCapabilities] = useState<PlatformCapabilities | null>(null);
  const [importing, setImporting] = useState(false);
  /** Pitches currently sounding from live input, for keyboard feedback. */
  const [liveNotes, setLiveNotes] = useState<Set<number>>(new Set());

  // What the toolbar's toggles show and hide. Both start on: the app should introduce
  // itself complete, and hiding a panel is a decision the user makes about their screen.
  const [showKeyboard, setShowKeyboard] = useState(true);
  const [showHistory, setShowHistory] = useState(true);
  const [showConsole, setShowConsole] = useState(false);

  /**
   * Which build this is.
   *
   * In the toolbar rather than only in Settings because once this runs as a plugin, this
   * window is the entire UI — and "am I looking at the fix I just built?" is the question
   * a cached Audio Unit scan makes impossible to answer any other way.
   */
  const [build, setBuild] = useState<BuildInfo | null>(null);

  const transport = useTransport({
    manifest,
    settingsRevision,
    onRecorded: setEditor,
  });

  const applied = useCallback((state: EditorState) => {
    setEditor(state);
    setSelection(state.affected);
    setSelectedTrack(state.affected_track);
  }, []);

  const listen = useTranscription({
    selectedTrack,
    ppq: manifest?.ppq ?? 480,
    onApplied: applied,
  });

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
    api.buildInfo().then(setBuild).catch(() => setBuild(null));
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

  const doImport = useCallback(async () => {
    setImporting(true);
    const state = await importSmfFile();
    setImporting(false);
    if (!state) return;
    setEditor(state);
    setSelection([]);
  }, []);

  // Keep the armed track in step with the selected one, so recording lands where the
  // user is looking rather than on whichever track was armed last.
  useEffect(() => {
    if (transport.recording) return;
    api.setArmedTrack(selectedTrack).catch(() => {});
  }, [selectedTrack, transport.recording]);

  // -- global shortcuts ----------------------------------------------------

  // Held in refs so the key handler does not need re-binding on every state change.
  const playingRef = useRef(transport.playing);
  playingRef.current = transport.playing;
  const toggleRecordRef = useRef(transport.toggleRecord);
  toggleRecordRef.current = transport.toggleRecord;
  const toggleListenRef = useRef(listen.toggleListen);
  toggleListenRef.current = listen.toggleListen;
  // The listen overlay is a mode: while it is up, Space belongs to it and Record and
  // Listen would both act on something the user cannot see.
  const overlayRef = useRef(listen.stage !== null);
  overlayRef.current = listen.stage !== null;

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }
      if (overlayRef.current) return;

      const mod = event.metaKey || event.ctrlKey;

      if (event.code === "Space") {
        event.preventDefault();
        void (playingRef.current ? transport.pause() : transport.play());
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
        return;
      }

      // Listen sits next to Record on the keyboard as it does in the transport, because
      // it is the peer of Record in this app rather than something behind a panel.
      if (!mod && event.key.toLowerCase() === "l") {
        event.preventDefault();
        toggleListenRef.current();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [transport.play, transport.pause, doUndo, doRedo, doSave]);

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

  const rollPreview = previewDiff(listen.pending);

  /** What the clock shows in note mode: the note under the playhead, if there is one. */
  const notePitch = useMemo(
    () => (track ? (noteAt(track.notes, transport.positionTicks)?.pitch ?? null) : null),
    [track, transport.positionTicks],
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

  const layout = [
    "editor",
    showConsole ? "editor--console" : "",
    showKeyboard ? "" : "editor--no-keyboard",
    showHistory ? "" : "editor--no-history",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={layout}>
      <Toolbar
        projectName={manifest.name}
        dirty={editor.dirty}
        build={build}
        onBack={onClose}
        showKeyboard={showKeyboard}
        onToggleKeyboard={() => setShowKeyboard((v) => !v)}
        showHistory={showHistory}
        onToggleHistory={() => setShowHistory((v) => !v)}
        playing={transport.playing}
        onPlay={() => void transport.play()}
        onPause={() => void transport.pause()}
        onStop={() => void transport.stop()}
        positionTicks={transport.positionTicks}
        ppq={manifest.ppq}
        timeSignature={manifest.time_signature}
        tempoBpm={transport.tempoBpm}
        inCountIn={transport.inCountIn}
        notePitch={notePitch}
        onTempoChange={transport.changeTempo}
        onOpenSettings={onOpenSettings}
        showConsole={showConsole}
        onToggleConsole={() => setShowConsole((v) => !v)}
        onImport={() => void doImport()}
        importing={importing}
      />

      <main className="editor__main">
        <section className="editor__tracks">
          <TrackList
            tracks={editor.tracks}
            selected={selectedTrack}
            onSelect={(index) => {
              setSelectedTrack(index);
              setSelection([]);
            }}
            onAdd={() => void addTrack()}
            onDelete={(trackId) => void deleteTrack(trackId)}
          />

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
                playheadTicks={transport.positionTicks}
                onScrub={(tick) => void transport.seek(tick)}
                loopRegion={transport.loopRegion}
                preview={rollPreview}
              />
            ) : (
              <p className="muted">No track selected.</p>
            )}

            {editor.tracks.every((t) => t.notes.length === 0) && !listen.pending && (
              <StartHere
                onListen={listen.toggleListen}
                onDescribe={() =>
                  document.querySelector<HTMLTextAreaElement>(".promptbar__input")?.focus()
                }
              />
            )}
          </div>
        </section>

        {showHistory && (
          <Inspector
            editor={editor}
            track={track}
            trackIndex={selectedTrack}
            selectionCount={selection.length}
            ppq={manifest.ppq}
            listening={listen.listening}
            transcribeOptions={listen.options}
            capabilities={capabilities}
            onUndo={doUndo}
            onRedo={doRedo}
            onTranscribeOptionsChange={listen.setOptions}
            onTranscribed={listen.takeResult}
          />
        )}
      </main>

      <footer className="editor__bottom">
        {listen.pending ? (
          <ReviewBar
            pending={listen.pending}
            onApply={() => void listen.applyPending()}
            onDiscard={() => void listen.discardPending()}
            onFineTune={listen.openReview}
          />
        ) : (
          <PromptBar
            trackIndex={selectedTrack}
            trackName={track?.name ?? "this track"}
            selection={selection}
            settingsRevision={settingsRevision}
            disabled={listen.listening}
            onProposal={(proposal) => listen.setPending({ kind: "ai", proposal })}
            onOpenSettings={onOpenSettings}
          />
        )}

        <div className="editor__transport">
          <Transport
            recording={transport.recording}
            loopRegion={transport.loopRegion}
            metronome={transport.metronome}
            dirty={editor.dirty}
            canUndo={editor.can_undo}
            canRedo={editor.can_redo}
            undoLabel={editor.undo_label}
            redoLabel={editor.redo_label}
            onRecord={() => void transport.toggleRecord()}
            listening={listen.listening}
            onListen={listen.toggleListen}
            onToggleLoop={() => void transport.toggleLoop()}
            onToggleMetronome={() => void transport.toggleMetronome()}
            onUndo={doUndo}
            onRedo={doRedo}
            onSave={doSave}
            onPanic={() => void api.panic().catch(() => {})}
          />
        </div>

        {showKeyboard && (
          <div className="editor__keyboard">
            <OnScreenKeyboard
              velocity={transport.keyboardVelocity}
              channel={track?.channel ?? 0}
              onNoteOn={noteOn}
              onNoteOff={noteOff}
              externalNotes={liveNotes}
            />
          </div>
        )}
      </footer>

      {showConsole && <ConsolePanel onClose={() => setShowConsole(false)} />}

      {listen.stage === "capture" && (
        <ListenOverlay
          title="Sing, hum, whistle or play — one note at a time"
          onClose={listen.cancelListen}
          closeLabel="Cancel this take"
        >
          <ListenCapture
            onStop={() => void listen.stopAndTranscribe()}
            onCancel={listen.cancelListen}
          />
        </ListenOverlay>
      )}

      {listen.stage === "review" && listen.pending?.kind === "transcription" && (
        <ListenOverlay
          title="What came back"
          stat={statOf(listen.pending.preview)}
          onClose={listen.closeReview}
          closeLabel="Close — the take stays in the review bar"
        >
          <TranscribeEditor
            preview={listen.pending.preview}
            ppq={manifest.ppq}
            onChange={listen.replacePreview}
            onApply={() => void listen.applyPending()}
            onDiscard={() => void listen.discardPending()}
          />
        </ListenOverlay>
      )}
    </div>
  );
}

/** The numbers worth having in the overlay's header, plus any warning about them. */
function statOf(preview: TranscriptionPreview): string {
  const head =
    `${preview.notes.length} notes · ${preview.duration_seconds.toFixed(1)}s · ` +
    `${preview.tempo_bpm.toFixed(0)} bpm${preview.tempo_estimated ? " (estimated)" : ""}`;
  return preview.warning ? `${head} — ${preview.warning}` : head;
}
