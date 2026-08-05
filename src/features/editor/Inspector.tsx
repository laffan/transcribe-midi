import type {
  EditorState,
  PlatformCapabilities,
  Track,
  TranscribeTuning,
  TranscriptionPreview,
} from "../../lib/types";
import { HistoryStrip } from "./HistoryStrip";
import { InterchangeBar } from "./InterchangeBar";
import { TranscribeSettings, type TranscribeOptions } from "./TranscribeSettings";

interface InspectorProps {
  editor: EditorState;
  track: Track | null;
  trackIndex: number;
  ppq: number;
  listening: boolean;
  transcribeOptions: TranscribeOptions;
  tuning: TranscribeTuning;
  capabilities: PlatformCapabilities | null;
  onUndo: () => void;
  onRedo: () => void;
  onTranscribeOptionsChange: (options: TranscribeOptions) => void;
  onTranscribed: (preview: TranscriptionPreview) => void;
}

/**
 * The side panel.
 *
 * Titled "History" rather than "Track Inspector": the chain of transformations is what
 * leads this panel, and it is not track-scoped — the undo stack spans the project. What
 * sits under it is the settings that change how the next transcription comes out, and
 * where finished notes go.
 */
export function Inspector({
  editor,
  track,
  trackIndex,
  ppq,
  listening,
  transcribeOptions,
  tuning,
  capabilities,
  onUndo,
  onRedo,
  onTranscribeOptionsChange,
  onTranscribed,
}: InspectorProps) {
  return (
    <aside className="editor__inspector">
      <div className="editor__panel-head">
        <h2 className="editor__panel-title">History</h2>
      </div>

      {track ? (
        <div className="inspector">
          <HistoryStrip
            history={editor.history}
            redoHistory={editor.redo_history}
            onUndo={onUndo}
            onRedo={onRedo}
          />

          <hr className="inspector__rule" />

          {/*
            What is left after saying each thing once.

            This panel used to carry the track name, the note count, the selection count
            and the instrument, each as a label above a value. Three of the four were
            already on screen: the name and the count are the selected row of the track
            list a few inches up, and the selection count is in the roll's own toolbar,
            right beside the notes it counts. Repeating them here did not make them
            clearer, it made the panel long enough that the things only it can tell you
            were below the fold.

            The instrument was a constant — "Built-in sampler" — under a note about a
            phase that has not happened. It comes back when there is a choice to make.

            Channel and PPQ are what nothing else shows: one decides where MIDI goes, the
            other is the resolution every exported file inherits.
          */}
          <div className="inspector__facts mono">
            <span>ch {track.channel + 1}</span>
            <span>{ppq} PPQ</span>
          </div>

          <hr className="inspector__rule" />

          <details className="inspector__shortcuts">
            <summary className="field__label">Shortcuts</summary>
            <dl className="shortcuts">
              <dt className="mono">Space</dt><dd>Play / stop</dd>
              <dt className="mono">⌘Z / ⇧⌘Z</dt><dd>Undo / redo</dd>
              <dt className="mono">⌘A</dt><dd>Select all</dd>
              <dt className="mono">⌘C / ⌘X / ⌘V</dt><dd>Copy / cut / paste</dd>
              <dt className="mono">⌘Q</dt><dd>Quantize selection</dd>
              <dt className="mono">J</dt><dd>Join selection into one note</dd>
              <dt className="mono">⌫</dt><dd>Delete selection</dd>
              <dt className="mono">↑ ↓ ← →</dt><dd>Nudge (⇧ for octave / bar)</dd>
              <dt className="mono">⌥click</dt><dd>Delete note</dd>
              <dt className="mono">R</dt><dd>Record</dd>
              <dt className="mono">L</dt><dd>Listen (audio → MIDI)</dd>
              <dt className="mono">⌘K</dt><dd>Focus the prompt</dd>
            </dl>
            <p className="field__hint">
              The letter keys play the on-screen keyboard only in Typing mode, which
              pauses everything above. Turn it on from the keyboard panel; Esc leaves.
            </p>
            <dl className="shortcuts">
              <dt className="mono">A–L, W/E/T/Y/U</dt><dd>Play keys (typing)</dd>
              <dt className="mono">Z / X</dt><dd>Octave down / up (typing)</dd>
            </dl>
          </details>

          <hr className="inspector__rule" />

          <TranscribeSettings
            trackIndex={trackIndex}
            ppq={ppq}
            options={transcribeOptions}
            tuning={tuning}
            disabled={listening}
            onChange={onTranscribeOptionsChange}
            onResult={onTranscribed}
          />

          <hr className="inspector__rule" />

          <InterchangeBar
            trackIndex={trackIndex}
            trackName={track.name}
            capabilities={capabilities}
          />
        </div>
      ) : (
        <p className="muted" style={{ padding: "var(--space-4)" }}>
          No track selected.
        </p>
      )}
    </aside>
  );
}
