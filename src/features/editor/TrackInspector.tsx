import type {
  EditorState,
  PlatformCapabilities,
  ProjectManifest,
  Track,
  TranscriptionPreview,
} from "../../lib/types";
import { HistoryStrip } from "./HistoryStrip";
import { InterchangeBar } from "./InterchangeBar";
import { ShortcutList } from "./ShortcutList";
import { TranscribeSettings, type TranscribeOptions } from "./TranscribeSettings";
import "./TrackInspector.css";

interface TrackInspectorProps {
  manifest: ProjectManifest;
  editor: EditorState;
  track: Track;
  trackIndex: number;
  selectionCount: number;
  capabilities: PlatformCapabilities | null;
  transcribeOptions: TranscribeOptions;
  listening: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onTranscribeOptionsChange: (options: TranscribeOptions) => void;
  onTranscribed: (preview: TranscriptionPreview) => void;
  onImported: (state: EditorState) => void;
}

/**
 * The right-hand panel.
 *
 * Titled "History" rather than "Track Inspector": the chain of transformations is what
 * leads this panel now, and it is not track-scoped — the undo stack spans the project.
 */
export function TrackInspector({
  manifest,
  editor,
  track,
  trackIndex,
  selectionCount,
  capabilities,
  transcribeOptions,
  listening,
  onUndo,
  onRedo,
  onTranscribeOptionsChange,
  onTranscribed,
  onImported,
}: TrackInspectorProps) {
  return (
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

      <ShortcutList />

      <hr className="inspector__rule" />

      <TranscribeSettings
        trackIndex={trackIndex}
        ppq={manifest.ppq}
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
        onImported={onImported}
      />
    </div>
  );
}
