/**
 * Bringing a MIDI file in.
 *
 * One routine rather than one per button: import is offered from the toolbar, where it
 * belongs with the other top-level ways material arrives, and the preview-then-import
 * sequence — which is what makes a PPQ rescale an announcement rather than a surprise —
 * must not exist in two versions that can drift apart.
 */

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState } from "../../lib/types";

/**
 * Pick a `.mid` file and import it. Resolves to the new editor state, or null when the
 * user cancelled or the import failed — failures are reported to the console here.
 */
export async function importSmfFile(): Promise<EditorState | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({
      multiple: false,
      filters: [{ name: "MIDI file", extensions: ["mid", "midi"] }],
    });
    const path = typeof picked === "string" ? picked : null;
    if (!path) return null;

    // Preview first so a PPQ rescale is announced rather than happening silently.
    const preview = await api.previewImport(path);
    if (preview.will_rescale && preview.source_ppq) {
      logger.warn(
        `Importing at ${preview.source_ppq} PPQ`,
        "timing will be rescaled to match this project",
      );
    }

    const result = await api.importSmf(path);
    logger.info(
      `Imported ${result.tracks_added} ${result.tracks_added === 1 ? "track" : "tracks"}, ` +
        `${result.notes_added} notes`,
    );
    return result.editor;
  } catch (error) {
    logger.error("Import failed", errorMessage(error));
    return null;
  }
}
