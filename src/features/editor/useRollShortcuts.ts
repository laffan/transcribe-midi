import { useEffect, useState } from "react";

import type { EditRequest, Note, TimeSignature, Track } from "../../lib/types";
import { snapTick } from "./pianoRollGeometry";

/**
 * The piano roll's keyboard shortcuts, and the note clipboard they act on.
 *
 * Split out of PianoRoll because it is a self-contained conversation with the window:
 * it reads keys and emits edits, and touches none of the canvas, the viewport or the
 * gesture state that the rest of the component is about. The clipboard lives here
 * rather than in the component because copy, cut and paste are the only three things
 * that ever look at it.
 *
 * Everything it does goes through `onEdit`, so a shortcut and a drag produce the same
 * command and land as the same undo step — there is no second path into a track.
 */
export interface RollShortcutOptions {
  track: Track;
  trackIndex: number;
  selection: number[];
  onSelectionChange: (indices: number[]) => void;
  onEdit: (request: EditRequest) => void;
  /** Grid size in ticks, for paste position and quantize. */
  snap: number;
  ppq: number;
  timeSignature: TimeSignature;
  playheadTicks: number;
  /** True while a proposal is on screen: the roll is read-only until it is answered. */
  readOnly: boolean;
}

export function useRollShortcuts({
  track,
  trackIndex,
  selection,
  onSelectionChange,
  onEdit,
  snap,
  ppq,
  timeSignature,
  playheadTicks,
  readOnly,
}: RollShortcutOptions): void {
  const [clipboard, setClipboard] = useState<Note[]>([]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      // Never steal keys from a text field.
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }
      // Editing underneath a proposal would invalidate it — Rust checks and refuses on
      // accept — so nothing here fires until the user has decided.
      if (readOnly) return;

      const mod = event.metaKey || event.ctrlKey;

      if ((event.key === "Delete" || event.key === "Backspace") && selection.length > 0) {
        event.preventDefault();
        onEdit({ kind: "delete", track: trackIndex, indices: selection });
        onSelectionChange([]);
        return;
      }

      if (mod && event.key.toLowerCase() === "a") {
        event.preventDefault();
        onSelectionChange(track.notes.map((_, index) => index));
        return;
      }

      if (mod && event.key.toLowerCase() === "c" && selection.length > 0) {
        event.preventDefault();
        setClipboard(selection.map((index) => track.notes[index]!).filter(Boolean));
        return;
      }

      if (mod && event.key.toLowerCase() === "x" && selection.length > 0) {
        event.preventDefault();
        setClipboard(selection.map((index) => track.notes[index]!).filter(Boolean));
        onEdit({ kind: "delete", track: trackIndex, indices: selection });
        onSelectionChange([]);
        return;
      }

      if (mod && event.key.toLowerCase() === "v" && clipboard.length > 0) {
        event.preventDefault();
        onEdit({ kind: "paste", track: trackIndex, notes: clipboard, at_ticks: snapTick(playheadTicks, snap) });
        return;
      }

      if (mod && event.key.toLowerCase() === "q" && selection.length > 0) {
        event.preventDefault();
        onEdit({ kind: "quantize", track: trackIndex, indices: selection, grid_ticks: snap });
        return;
      }

      // Arrow nudging. Shift moves by an octave / a whole bar rather than one step.
      if (selection.length > 0 && event.key.startsWith("Arrow")) {
        event.preventDefault();
        const beat = (ppq * 4) / timeSignature.denominator;

        if (event.key === "ArrowUp" || event.key === "ArrowDown") {
          const delta = (event.key === "ArrowUp" ? 1 : -1) * (event.shiftKey ? 12 : 1);
          onEdit({ kind: "move", track: trackIndex, indices: selection, delta_ticks: 0, delta_pitch: delta });
        } else {
          const step = snap > 0 ? snap : Math.round(ppq / 4);
          const delta = (event.key === "ArrowRight" ? 1 : -1) * (event.shiftKey ? beat : step);
          onEdit({ kind: "move", track: trackIndex, indices: selection, delta_ticks: delta, delta_pitch: 0 });
        }
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    selection,
    clipboard,
    track,
    trackIndex,
    onEdit,
    onSelectionChange,
    snap,
    ppq,
    timeSignature,
    playheadTicks,
    readOnly,
  ]);
}
