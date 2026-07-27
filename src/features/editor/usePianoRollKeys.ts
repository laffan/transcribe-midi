import { useEffect, useState } from "react";

import type { EditRequest, Note, NoteDiff, TimeSignature, Track } from "../../lib/types";
import { snapTick } from "./pianoRollGeometry";

interface KeyDeps {
  track: Track;
  trackIndex: number;
  ppq: number;
  timeSignature: TimeSignature;
  snap: number;
  selection: number[];
  onSelectionChange: (indices: number[]) => void;
  onEdit: (request: EditRequest) => void;
  playheadTicks: number;
  preview: NoteDiff | null | undefined;
}

/**
 * The note-editing shortcuts, and the clipboard they operate on.
 *
 * The clipboard is local to the roll rather than the system one: Logic's note format is
 * proprietary and interchange goes through SMF files, so there is nothing useful to put on
 * the system clipboard and nothing useful to read back from it.
 *
 * Every one of these emits a command, which is what makes ⌘Z undo a nudge the same way it
 * undoes an AI edit.
 */
export function usePianoRollKeys({
  track,
  trackIndex,
  ppq,
  timeSignature,
  snap,
  selection,
  onSelectionChange,
  onEdit,
  playheadTicks,
  preview,
}: KeyDeps) {
  const [clipboard, setClipboard] = useState<Note[]>([]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      // Never steal keys from a text field.
      if (
        target &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      ) {
        return;
      }
      if (preview) return;

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
  }, [selection, clipboard, track, trackIndex, onEdit, onSelectionChange, snap, ppq, timeSignature, playheadTicks, preview]);
}
