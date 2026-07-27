import type { EditorState, PlatformCapabilities, Track, TranscriptionPreview } from "../../lib/types";
import { HistoryStrip } from "./HistoryStrip";
import { InterchangeBar } from "./InterchangeBar";
import { TranscribeSettings, type TranscribeOptions } from "./TranscribeSettings";

interface InspectorProps {
  editor: EditorState;
  track: Track | null;
  trackIndex: number;
  selectionCount: number;
  ppq: number;
  listening: boolean;
  transcribeOptions: TranscribeOptions;
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
  selectionCount,
  ppq,
  listening,
  transcribeOptions,
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

          <div className="field">
            <span className="field__label">Track</span>
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
            <div className="inspector__value mono">{selectionCount}</div>
          </div>

          <div className="field">
            <span className="field__label">Instrument</span>
            <div className="inspector__value">Built-in sampler</div>
            <span className="field__hint">AUv3 instruments arrive in Phase 9.</span>
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
              <dt className="mono">⌫</dt><dd>Delete selection</dd>
              <dt className="mono">↑ ↓ ← →</dt><dd>Nudge (⇧ for octave / bar)</dd>
              <dt className="mono">⌥click</dt><dd>Delete note</dd>
              <dt className="mono">A–L, W/E/T/Y/U</dt><dd>Play keys</dd>
              <dt className="mono">Z / X</dt><dd>Octave down / up</dd>
              <dt className="mono">L</dt><dd>Listen (audio → MIDI)</dd>
              <dt className="mono">⌘K</dt><dd>Focus the prompt</dd>
            </dl>
          </details>

          <hr className="inspector__rule" />

          <TranscribeSettings
            trackIndex={trackIndex}
            ppq={ppq}
            options={transcribeOptions}
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
