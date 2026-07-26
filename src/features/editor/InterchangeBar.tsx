import { useState } from "react";

import { api, errorMessage, isTauri } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, ExportPayload, PlatformCapabilities } from "../../lib/types";
import "./InterchangeBar.css";

interface InterchangeBarProps {
  trackIndex: number;
  trackName: string;
  capabilities: PlatformCapabilities | null;
  onImported: (state: EditorState) => void;
}

/**
 * Phase 5: import, export, share and drag-out.
 *
 * Only affordances the platform actually supports are shown — `platform_capabilities`
 * reports what is real, so there is never a Share button on macOS that fails when
 * pressed. All bytes are produced in Rust; this only decides where they go.
 */
export function InterchangeBar({
  trackIndex,
  trackName,
  capabilities,
  onImported,
}: InterchangeBarProps) {
  const [busy, setBusy] = useState<string | null>(null);

  async function pickSavePath(defaultName: string): Promise<string | null> {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({
      defaultPath: defaultName,
      filters: [{ name: "MIDI file", extensions: ["mid"] }],
    });
  }

  async function exportTo(payload: () => Promise<ExportPayload>, label: string) {
    setBusy(label);
    try {
      const { filename, bytes } = await payload();
      const path = await pickSavePath(filename);
      if (!path) return; // cancelled

      const written = await api.writeExport(path, bytes);
      logger.info(`Exported to ${written}`);
    } catch (error) {
      logger.error(`${label} failed`, errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  /** Stage the file on disk, then hand its path to the platform. */
  async function staged(payload: () => Promise<ExportPayload>): Promise<string> {
    const { filename, bytes } = await payload();
    return api.stageExport(filename, bytes);
  }

  async function doImport() {
    setBusy("Import");
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        filters: [{ name: "MIDI file", extensions: ["mid", "midi"] }],
      });
      const path = typeof picked === "string" ? picked : null;
      if (!path) return;

      // Preview first so a PPQ rescale is announced rather than happening silently.
      const preview = await api.previewImport(path);
      if (preview.will_rescale && preview.source_ppq) {
        logger.warn(
          `Importing at ${preview.source_ppq} PPQ`,
          "timing will be rescaled to match this project",
        );
      }

      const result = await api.importSmf(path);
      onImported(result.editor);
      logger.info(
        `Imported ${result.tracks_added} ${result.tracks_added === 1 ? "track" : "tracks"}, ` +
          `${result.notes_added} notes`,
      );
    } catch (error) {
      logger.error("Import failed", errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  async function doShare() {
    setBusy("Share");
    try {
      await api.shareFile(await staged(() => api.exportTrackSmf(trackIndex)));
    } catch (error) {
      logger.error("Share failed", errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  async function doCopy() {
    setBusy("Copy");
    try {
      await api.copyFileToPasteboard(await staged(() => api.exportTrackSmf(trackIndex)));
      logger.info(`Copied "${trackName}" as a .mid file`);
    } catch (error) {
      logger.error("Copy failed", errorMessage(error));
    } finally {
      setBusy(null);
    }
  }

  async function startDrag(event: React.PointerEvent) {
    if (!capabilities?.drag_out) return;
    // The native drag must begin while the pointer event is still live — AppKit needs a
    // real event to attach the session to.
    event.preventDefault();
    try {
      await api.beginFileDrag(await staged(() => api.exportTrackSmf(trackIndex)));
    } catch (error) {
      logger.error("Drag failed", errorMessage(error));
    }
  }

  if (!isTauri()) {
    return (
      <div className="interchange">
        <span className="field__label">Import / Export</span>
        <p className="field__hint">
          Unavailable in the browser preview — SMF is written by Rust.
        </p>
      </div>
    );
  }

  return (
    <div className="interchange">
      <span className="field__label">Import / Export</span>

      <div className="interchange__row">
        <button className="btn btn--ghost" onClick={doImport} disabled={busy !== null}>
          Import…
        </button>
        <button
          className="btn btn--ghost"
          onClick={() => exportTo(() => api.exportProjectSmf(), "Export project")}
          disabled={busy !== null}
        >
          Export project
        </button>
      </div>

      <div className="interchange__row">
        <button
          className="btn btn--ghost"
          onClick={() => exportTo(() => api.exportTrackSmf(trackIndex), "Export track")}
          disabled={busy !== null}
        >
          Export track
        </button>

        {capabilities?.pasteboard && (
          <button className="btn btn--ghost" onClick={doCopy} disabled={busy !== null}>
            Copy
          </button>
        )}

        {capabilities?.share_sheet && (
          <button className="btn btn--ghost" onClick={doShare} disabled={busy !== null}>
            Share…
          </button>
        )}
      </div>

      {capabilities?.drag_out && (
        <div
          className="interchange__drag"
          onPointerDown={startDrag}
          role="button"
          tabIndex={0}
          title={`Drag "${trackName}" into Logic Pro or the Finder`}
        >
          ⇱ Drag track out as .mid
        </div>
      )}

      <p className="field__hint">
        {capabilities?.pasteboard
          ? "Copy puts the .mid file on the clipboard — it pastes into Finder or Files, not as notes into Logic's piano roll."
          : "Exports are SMF type 1, which Logic Pro and GarageBand import directly."}
      </p>
    </div>
  );
}
