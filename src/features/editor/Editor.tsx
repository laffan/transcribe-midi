import { useEffect, useState } from "react";

import { api } from "../../lib/api";
import type { BuildInfo, PlatformCapabilities } from "../../lib/types";
import { ConsolePanel } from "./ConsolePanel";
import { EditorTitlebar } from "./EditorTitlebar";
import { ListeningBar } from "./ListeningBar";
import { OnScreenKeyboard } from "./OnScreenKeyboard";
import { previewDiff } from "./pending";
import { PianoRoll } from "./PianoRoll";
import { PromptBar } from "./PromptBar";
import { ReviewBar } from "./ReviewBar";
import { StartHere } from "./StartHere";
import { TrackInspector } from "./TrackInspector";
import { TrackList } from "./TrackList";
import { TranscribeEditor } from "./TranscribeEditor";
import { Transport } from "./Transport";
import { useEditorSession } from "./useEditorSession";
import { useEditorShortcuts } from "./useEditorShortcuts";
import { useInputSettings } from "./useInputSettings";
import { useLiveNotes } from "./useLiveNotes";
import { usePendingEdit } from "./usePendingEdit";
import { useTransport } from "./useTransport";
import "./Editor.css";

interface EditorProps {
  projectId: string;
  /** Changes when Settings closes, so input settings are re-read. */
  settingsRevision: number;
  onClose: () => void;
  onOpenSettings: () => void;
}

/**
 * The editor screen: layout, and the wiring between the hooks that hold its state.
 *
 * Nothing here talks to the backend directly. Each concern — the open project, the
 * transport, live input, a pending proposal, the shortcuts — is a hook in its own file, and
 * this component's job is to say how they connect and where they appear on screen.
 */
export function Editor({ projectId, settingsRevision, onClose, onOpenSettings }: EditorProps) {
  const session = useEditorSession(projectId);
  const { manifest, editor, track } = session;

  const transport = useTransport({
    manifest,
    setEditor: session.setEditor,
    selectedTrack: session.selectedTrack,
  });
  const input = useInputSettings(settingsRevision);
  const live = useLiveNotes(track?.channel ?? 0);
  const proposal = usePendingEdit({
    ppq: manifest?.ppq,
    selectedTrack: session.selectedTrack,
    setEditor: session.setEditor,
    setSelection: session.setSelection,
    setSelectedTrack: session.setSelectedTrack,
  });

  const [showConsole, setShowConsole] = useState(false);
  const [capabilities, setCapabilities] = useState<PlatformCapabilities | null>(null);
  const [build, setBuild] = useState<BuildInfo | null>(null);

  useEffect(() => {
    api.platformCapabilities().then(setCapabilities).catch(() => setCapabilities(null));
    api.buildInfo().then(setBuild).catch(() => setBuild(null));
  }, []);

  useEditorShortcuts({
    playing: transport.playing,
    play: transport.play,
    stop: transport.stop,
    undo: session.undo,
    redo: session.redo,
    save: session.save,
    toggleRecord: transport.toggleRecord,
    toggleListen: proposal.toggleListen,
  });

  if (session.loadError) {
    return (
      <div className="editor__failure">
        <h2>Could not open this project</h2>
        <p className="muted">{session.loadError}</p>
        <button className="btn" onClick={onClose}>
          Back to Projects
        </button>
      </div>
    );
  }

  if (!manifest || !editor) {
    return <div className="editor__failure muted">Loading…</div>;
  }

  return (
    <div className={`editor ${showConsole ? "editor--console" : ""}`}>
      <EditorTitlebar
        name={manifest.name}
        ppq={manifest.ppq}
        dirty={editor.dirty}
        build={build}
        showConsole={showConsole}
        onToggleConsole={() => setShowConsole((v) => !v)}
        onClose={onClose}
        onOpenSettings={onOpenSettings}
      />

      <main className="editor__main">
        <section className="editor__tracks">
          <TrackList
            tracks={editor.tracks}
            selectedTrack={session.selectedTrack}
            onSelect={session.selectTrack}
            onAdd={() => void session.addTrack()}
            onDelete={(trackId) => void session.deleteTrack(trackId)}
          />

          <div className="editor__roll">
            {track ? (
              <PianoRoll
                track={track}
                trackIndex={session.selectedTrack}
                ppq={manifest.ppq}
                timeSignature={manifest.time_signature}
                selection={session.selection}
                onSelectionChange={session.setSelection}
                onEdit={session.applyEdit}
                onPreviewNote={live.previewNote}
                playheadTicks={transport.positionTicks}
                onScrub={transport.seek}
                loopRegion={transport.loopRegion}
                preview={previewDiff(proposal.pending)}
              />
            ) : (
              <p className="muted">No track selected.</p>
            )}

            {editor.tracks.every((t) => t.notes.length === 0) && !proposal.pending && (
              <StartHere
                onListen={() => void proposal.toggleListen()}
                onDescribe={() =>
                  document.querySelector<HTMLTextAreaElement>(".promptbar__input")?.focus()
                }
              />
            )}
          </div>
        </section>

        <aside className="editor__inspector">
          <div className="editor__panel-head">
            <h2 className="editor__panel-title">History</h2>
          </div>

          {track ? (
            <TrackInspector
              manifest={manifest}
              editor={editor}
              track={track}
              trackIndex={session.selectedTrack}
              selectionCount={session.selection.length}
              capabilities={capabilities}
              transcribeOptions={proposal.transcribeOptions}
              listening={proposal.listening}
              onUndo={session.undo}
              onRedo={session.redo}
              onTranscribeOptionsChange={proposal.setTranscribeOptions}
              onTranscribed={(preview) => {
                proposal.setPending({ kind: "transcription", preview });
                proposal.setFineTuning(preview.notes.length > 0);
              }}
              onImported={(state) => {
                session.setEditor(state);
                session.setSelection([]);
              }}
            />
          ) : (
            <p className="muted" style={{ padding: "var(--space-4)" }}>
              No track selected.
            </p>
          )}
        </aside>
      </main>

      <footer className="editor__bottom">
        {proposal.listening ? (
          <ListeningBar
            onStop={() => void proposal.toggleListen()}
            onCancel={proposal.cancelListening}
          />
        ) : proposal.pending ? (
          <ReviewBar
            pending={proposal.pending}
            onApply={() => void proposal.apply()}
            onDiscard={() => void proposal.discard()}
            onFineTune={() => proposal.setFineTuning(true)}
          />
        ) : (
          <PromptBar
            trackIndex={session.selectedTrack}
            trackName={track?.name ?? "this track"}
            selection={session.selection}
            settingsRevision={settingsRevision}
            disabled={proposal.listening}
            onProposal={(aiProposal) => proposal.setPending({ kind: "ai", proposal: aiProposal })}
            onOpenSettings={onOpenSettings}
          />
        )}

        <div className="editor__transport">
          <Transport
            playing={transport.playing}
            recording={transport.recording}
            inCountIn={transport.inCountIn}
            loopRegion={transport.loopRegion}
            metronome={input.metronome}
            positionTicks={transport.positionTicks}
            tempoBpm={session.tempo}
            ppq={manifest.ppq}
            timeSignature={manifest.time_signature}
            dirty={editor.dirty}
            canUndo={editor.can_undo}
            canRedo={editor.can_redo}
            undoLabel={editor.undo_label}
            redoLabel={editor.redo_label}
            onPlay={transport.play}
            onStop={transport.stop}
            onRecord={transport.toggleRecord}
            listening={proposal.listening}
            listenLevel={proposal.listenLevel}
            onListen={() => void proposal.toggleListen()}
            onToggleLoop={transport.toggleLoop}
            onToggleMetronome={input.toggleMetronome}
            onReturnToZero={() => void transport.seek(0)}
            onTempoChange={session.changeTempo}
            onUndo={session.undo}
            onRedo={session.redo}
            onSave={session.save}
            onPanic={() => void api.panic().catch(() => {})}
          />
        </div>
        <div className="editor__keyboard">
          <OnScreenKeyboard
            velocity={input.keyboardVelocity}
            channel={track?.channel ?? 0}
            onNoteOn={live.noteOn}
            onNoteOff={live.noteOff}
            externalNotes={live.liveNotes}
          />
        </div>
      </footer>

      {showConsole && <ConsolePanel onClose={() => setShowConsole(false)} />}

      {proposal.fineTuning && proposal.pending?.kind === "transcription" && (
        <TranscribeEditor
          preview={proposal.pending.preview}
          ppq={manifest.ppq}
          onChange={(preview) => proposal.setPending({ kind: "transcription", preview })}
          onApply={() => void proposal.apply()}
          onClose={() => proposal.setFineTuning(false)}
        />
      )}
    </div>
  );
}
