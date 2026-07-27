import { useEffect, useMemo } from "react";

import type { EditRequest, NoteDiff, TimeSignature, Track } from "../../lib/types";
import { paintRoll } from "./pianoRollPaint";
import { readRollTheme } from "./pianoRollTheme";
import { RollToolbar } from "./RollToolbar";
import { usePianoRollGestures } from "./usePianoRollGestures";
import { usePianoRollKeys } from "./usePianoRollKeys";
import { usePianoRollView } from "./usePianoRollView";
import "./PianoRoll.css";

interface PianoRollProps {
  track: Track;
  trackIndex: number;
  ppq: number;
  timeSignature: TimeSignature;
  selection: number[];
  onSelectionChange: (indices: number[]) => void;
  onEdit: (request: EditRequest) => void;
  onPreviewNote: (pitch: number) => void;
  playheadTicks: number;
  onScrub: (tick: number) => void;
  loopRegion: [number, number] | null;
  /**
   * An AI proposal being previewed. While this is set the roll is read-only and draws
   * the change on top of the current notes rather than instead of them — the point of a
   * preview is seeing what would move, not seeing the result in isolation.
   */
  preview?: NoteDiff | null;
}

/**
 * The piano roll: the canvas, and the three things that act on it.
 *
 * The maths is in `pianoRollGeometry`, the painting in `pianoRollPaint`, and the input in
 * the three hooks — view (zoom and scroll), gestures (pointer), keys. What is left here is
 * the frame loop and the element tree, which is the part that genuinely needs a component.
 */
export function PianoRoll({
  track,
  trackIndex,
  ppq,
  timeSignature,
  selection,
  onSelectionChange,
  onEdit,
  onPreviewNote,
  playheadTicks,
  onScrub,
  loopRegion,
  preview,
}: PianoRollProps) {
  const view = usePianoRollView(ppq);
  const { canvasRef, containerRef, size, rollHeight, snap } = view;

  const selectionSet = useMemo(() => new Set(selection), [selection]);

  const gestures = usePianoRollGestures({
    canvasRef,
    track,
    trackIndex,
    ppq,
    view: view.view,
    snap,
    rollHeight,
    selection,
    selectionSet,
    onSelectionChange,
    onEdit,
    onPreviewNote,
    onScrub,
    preview,
  });

  usePianoRollKeys({
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
  });

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      // Back the canvas at device resolution, then work in CSS pixels — otherwise every
      // hairline the roll draws lands between physical pixels and blurs.
      const dpr = window.devicePixelRatio || 1;
      if (canvas.width !== size.width * dpr || canvas.height !== size.height * dpr) {
        canvas.width = size.width * dpr;
        canvas.height = size.height * dpr;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      paintRoll(ctx, {
        theme: readRollTheme(),
        width: size.width,
        height: size.height,
        rollHeight,
        view: view.view,
        track,
        selection: selectionSet,
        ppq,
        timeSignature,
        snap,
        playheadTicks,
        loopRegion,
        marquee: gestures.marquee,
        preview: preview ?? null,
      });
    });

    return () => cancelAnimationFrame(frame);
  }, [
    canvasRef, size, rollHeight, view.view, track, selectionSet, ppq, timeSignature,
    snap, playheadTicks, loopRegion, gestures.marquee, preview,
  ]);

  return (
    <div className="roll">
      <RollToolbar
        gridDivisor={view.gridDivisor}
        onGridChange={view.setGridDivisor}
        pxPerTick={view.pxPerTick}
        onZoomChange={view.setPxPerTick}
        rowHeight={view.rowHeight}
        onRowHeightChange={view.setRowHeight}
        selectionCount={selection.length}
      />

      <div className="roll__canvas-wrap" ref={containerRef}>
        <canvas
          ref={canvasRef}
          className="roll__canvas"
          style={{ width: size.width, height: size.height, cursor: gestures.cursor }}
          onPointerDown={gestures.onPointerDown}
          onPointerMove={gestures.onPointerMove}
          onPointerUp={gestures.onPointerUp}
          onPointerCancel={gestures.onPointerUp}
        />
      </div>
    </div>
  );
}
