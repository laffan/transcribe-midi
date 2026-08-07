import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { EditRequest, NoteDiff, TimeSignature, Track } from "../../lib/types";
import {
  gridTicks,
  hitTest,
  KEY_WIDTH,
  loopEdgeAt,
  MAX_PITCH,
  MIN_PITCH,
  type Marquee,
  normalizeLoopTicks,
  normalizeMarquee,
  notesInMarquee,
  RULER_HEIGHT,
  snapTick,
  snapTickDown,
  velocityLaneHeight,
  type Viewport,
  xToTick,
  yToPitch,
} from "./pianoRollGeometry";
import { isNavigating, pinchView, type TouchPoint, useRollWheel } from "./rollGestures";
import { paintRoll } from "./pianoRollPaint";
import { readRollTheme } from "./pianoRollTheme";
import { RollToolbar } from "./RollToolbar";
import { useRollShortcuts } from "./useRollShortcuts";
import "./PianoRoll.css";

/** Below this drag distance a pointer-up counts as a click, not a drag. */
const DRAG_THRESHOLD_PX = 3;

/**
 * How close a press has to be to a note's end, or a loop's tab, to take hold of it.
 *
 * Two numbers because a fingertip covers about a centimetre and a mouse pointer aims at
 * a pixel. The grip is drawn at one size either way; this is only how forgiving it is.
 */
const GRAB_PX = { fine: 6, coarse: 20 };

const grabPx = (event: React.PointerEvent): number =>
  event.pointerType === "mouse" ? GRAB_PX.fine : GRAB_PX.coarse;

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
  /** Set the looped span, in ticks, or clear it. Dragged out in the ruler. */
  onLoopChange: (region: [number, number] | null) => void;
  /**
   * An AI proposal being previewed. While this is set the roll is read-only and draws
   * the change on top of the current notes rather than instead of them — the point of a
   * preview is seeing what would move, not seeing the result in isolation.
   */
  preview?: NoteDiff | null;
  /**
   * True while another mode owns the keyboard — typing on the on-screen keys, or the
   * listen overlay. The roll keeps its mouse behaviour and gives up its shortcuts, which
   * is the only way `⌫`, `J` and the arrows can mean one thing at a time.
   */
  shortcutsSuspended?: boolean;
}

type Gesture =
  | { type: "none" }
  | { type: "marquee"; marquee: Marquee; additive: boolean }
  | { type: "move"; startTick: number; startPitch: number; indices: number[]; moved: boolean }
  | { type: "resize"; startTick: number; indices: number[]; moved: boolean }
  | { type: "velocity"; indices: number[] }
  // A press in the ruler, before it is known whether it is a tap or a drag. A tap moves
  // the playhead; a drag marks the loop, or moves an end of the one already there.
  | { type: "ruler"; anchorTick: number; edge: "start" | "end" | null; moved: boolean }
  // Two fingers on the canvas: panning and zooming rather than editing. Holds the
  // touches and the viewport as they were when the second finger landed, because
  // pinchView measures from the start of the gesture rather than frame to frame.
  | { type: "pinch"; from: readonly [TouchPoint, TouchPoint]; view: Viewport };

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
  onLoopChange,
  preview,
  shortcutsSuspended = false,
}: PianoRollProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 400 });

  const [gridDivisor, setGridDivisor] = useState(16);
  const [pxPerTick, setPxPerTick] = useState(0.12);
  const [rowHeight, setRowHeight] = useState(12);
  const [scrollTicks, setScrollTicks] = useState(0);
  const [topPitch, setTopPitch] = useState(84);
  const [gesture, setGesture] = useState<Gesture>({ type: "none" });

  const view: Viewport = useMemo(
    () => ({ pxPerTick, rowHeight, scrollTicks, topPitch }),
    [pxPerTick, rowHeight, scrollTicks, topPitch],
  );

  const snap = gridTicks(ppq, gridDivisor);
  const selectionSet = useMemo(() => new Set(selection), [selection]);

  // -- sizing --------------------------------------------------------------

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      // The floor is what the roll needs to draw a ruler and a few semitones. Higher
      // than that and the canvas is taller than the box holding it, so on a short
      // screen the bottom — which is where the velocity lane is — is simply clipped
      // away rather than shrunk.
      if (rect) setSize({ width: Math.max(200, rect.width), height: Math.max(96, rect.height) });
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  const velocityLane = velocityLaneHeight(size.height);
  const rollHeight = size.height - velocityLane;

  // -- drawing -------------------------------------------------------------

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // The backing store is in device pixels and the scene is in CSS pixels; the
    // transform is what lets everything below think in one of them.
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
      velocityLane,
      view,
      track,
      selection: selectionSet,
      ppq,
      timeSignature,
      snap,
      playheadTicks,
      loopRegion,
      marquee: gesture.type === "marquee" ? gesture.marquee : null,
      preview: preview ?? null,
    });
  }, [
    size, rollHeight, velocityLane, view, track, selectionSet, ppq, timeSignature,
    snap, gesture, playheadTicks, loopRegion, preview,
  ]);

  useEffect(() => {
    const frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [draw]);

  // -- pointer -------------------------------------------------------------

  const localPoint = (event: React.PointerEvent): { x: number; y: number } => {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  const dragOrigin = useRef({ x: 0, y: 0 });

  /**
   * Every pointer currently down on the canvas, so a second finger can be noticed. A
   * ref rather than state: it is read inside the same handler that writes it, and a
   * render between the two would lose the gesture.
   */
  const pointers = useRef(new Map<number, TouchPoint>());

  /** The two touches of a pinch, in the order they arrived. */
  function pinchPair(): readonly [TouchPoint, TouchPoint] | null {
    const live = [...pointers.current.values()];
    return live.length >= 2 ? [live[0]!, live[1]!] : null;
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    // A proposal is on screen. Editing underneath it would invalidate it — Rust checks
    // and refuses on accept — so the roll is read-only until the user decides.
    if (preview) return;

    const { x, y } = localPoint(event);
    pointers.current.set(event.pointerId, { x, y });

    // A second finger takes over from whatever the first one was doing. Any edit it
    // already made stands and is one undo away, which is the right outcome — the
    // alternative is a pinch that silently reverts a drag you meant to keep.
    const pair = pinchPair();
    if (isNavigating(pointers.current.size) && pair) {
      setGesture({ type: "pinch", from: pair, view });
      return;
    }

    dragOrigin.current = { x, y };
    canvasRef.current?.setPointerCapture(event.pointerId);

    // Ruler: the playhead and the loop share it, and which one a press means is decided
    // by whether it travels. A tap seeks; a drag marks a loop; a press on one of the
    // loop's tabs moves that end. No modifier, because a touchscreen has none.
    if (y < RULER_HEIGHT && x > KEY_WIDTH) {
      setGesture({
        type: "ruler",
        anchorTick: Math.max(0, snapTick(xToTick(x, view), snap)),
        edge: loopEdgeAt(loopRegion, x, view, grabPx(event)),
        moved: false,
      });
      return;
    }

    // Keyboard gutter: audition the pitch.
    if (x < KEY_WIDTH && y > RULER_HEIGHT && y < rollHeight) {
      onPreviewNote(yToPitch(y, view));
      return;
    }

    // Velocity lane, when there is one.
    if (velocityLane > 0 && y >= rollHeight) {
      const target = selection.length > 0 ? selection : [];
      if (target.length > 0) {
        setGesture({ type: "velocity", indices: target });
        applyVelocityFromY(y, target);
      }
      return;
    }

    const hit = hitTest(track.notes, x, y, view, grabPx(event));

    if (hit) {
      const additive = event.shiftKey || event.metaKey || event.ctrlKey;
      let next = selection;

      if (additive) {
        next = selectionSet.has(hit.index)
          ? selection.filter((i) => i !== hit.index)
          : [...selection, hit.index];
      } else if (!selectionSet.has(hit.index)) {
        next = [hit.index];
      }
      onSelectionChange(next);

      // Alt-click deletes, which is the fastest way to clear a stray note.
      if (event.altKey) {
        onEdit({ kind: "delete", track: trackIndex, indices: next });
        onSelectionChange([]);
        setGesture({ type: "none" });
        return;
      }

      const note = track.notes[hit.index]!;
      onPreviewNote(note.pitch);

      setGesture(
        hit.onResizeHandle
          ? { type: "resize", startTick: xToTick(x, view), indices: next, moved: false }
          : {
              type: "move",
              startTick: xToTick(x, view),
              startPitch: yToPitch(y, view),
              indices: next,
              moved: false,
            },
      );
      return;
    }

    // Empty space: marquee.
    setGesture({
      type: "marquee",
      marquee: { x0: x, y0: y, x1: x, y1: y },
      additive: event.shiftKey,
    });
    if (!event.shiftKey) onSelectionChange([]);
  }

  function applyVelocityFromY(y: number, indices: number[]) {
    const laneBottom = rollHeight + velocityLane - 6;
    const laneHeight = velocityLane - 22;
    const ratio = Math.min(1, Math.max(0, (laneBottom - y) / laneHeight));
    onEdit({
      kind: "set_velocity",
      track: trackIndex,
      indices,
      velocity: Math.round(1 + ratio * 126),
    });
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    const { x, y } = localPoint(event);
    if (pointers.current.has(event.pointerId)) {
      pointers.current.set(event.pointerId, { x, y });
    }

    if (gesture.type === "pinch") {
      const pair = pinchPair();
      // One finger lifted mid-gesture; the remaining one must not start editing from
      // wherever it happens to be, so the roll waits for it to be lifted too.
      if (!pair) return;
      const next = pinchView({ from: gesture.from, to: pair, view: gesture.view });
      setPxPerTick(next.pxPerTick);
      setScrollTicks(next.scrollTicks);
      setTopPitch(next.topPitch);
      return;
    }

    if (gesture.type === "none") return;

    if (gesture.type === "ruler") {
      const tick = Math.max(0, snapTick(xToTick(x, view), snap));
      const travelled = Math.abs(x - dragOrigin.current.x) > DRAG_THRESHOLD_PX;
      if (!gesture.edge && !travelled) return;

      const minimum = snap > 0 ? snap : Math.round(ppq / 4);
      const other =
        gesture.edge === "start"
          ? (loopRegion?.[1] ?? gesture.anchorTick)
          : gesture.edge === "end"
            ? (loopRegion?.[0] ?? gesture.anchorTick)
            : gesture.anchorTick;

      onLoopChange(normalizeLoopTicks(tick, other, minimum));
      setGesture({ ...gesture, moved: true });
      return;
    }

    if (gesture.type === "marquee") {
      setGesture({ ...gesture, marquee: { ...gesture.marquee, x1: x, y1: y } });
      return;
    }

    if (gesture.type === "velocity") {
      applyVelocityFromY(y, gesture.indices);
      return;
    }

    const travelled = Math.hypot(x - dragOrigin.current.x, y - dragOrigin.current.y);
    if (travelled < DRAG_THRESHOLD_PX) return;

    if (gesture.type === "move") {
      const rawDelta = xToTick(x, view) - gesture.startTick;
      // Snap the delta, not the absolute position, so a group keeps its internal
      // rhythm when dragged rather than collapsing onto grid lines.
      const deltaTicks = snap > 0 ? Math.round(rawDelta / snap) * snap : Math.round(rawDelta);
      const deltaPitch = yToPitch(y, view) - gesture.startPitch;

      if (deltaTicks !== 0 || deltaPitch !== 0) {
        onEdit({ kind: "move", track: trackIndex, indices: gesture.indices, delta_ticks: deltaTicks, delta_pitch: deltaPitch });
        setGesture({
          ...gesture,
          startTick: gesture.startTick + deltaTicks,
          startPitch: gesture.startPitch + deltaPitch,
          moved: true,
        });
      }
      return;
    }

    if (gesture.type === "resize") {
      const rawDelta = xToTick(x, view) - gesture.startTick;
      const deltaTicks = snap > 0 ? Math.round(rawDelta / snap) * snap : Math.round(rawDelta);
      if (deltaTicks !== 0) {
        onEdit({ kind: "resize", track: trackIndex, indices: gesture.indices, delta_ticks: deltaTicks });
        setGesture({ ...gesture, startTick: gesture.startTick + deltaTicks, moved: true });
      }
    }
  }

  function onPointerUp(event: React.PointerEvent<HTMLCanvasElement>) {
    canvasRef.current?.releasePointerCapture(event.pointerId);
    pointers.current.delete(event.pointerId);

    // Lifting out of a pinch resolves to nothing rather than falling through to the
    // marquee branch below, which would read the gesture as a tap and draw a note
    // wherever the fingers happened to be.
    if (gesture.type === "pinch") {
      setGesture({ type: "none" });
      return;
    }

    // A press in the ruler that never travelled is a seek, decided here rather than on
    // the way down so that the same press can turn out to be a loop instead.
    if (gesture.type === "ruler") {
      if (!gesture.moved) onScrub(gesture.anchorTick);
      setGesture({ type: "none" });
      return;
    }

    if (gesture.type === "marquee") {
      const picked = notesInMarquee(track.notes, gesture.marquee, view);
      const { left, right, top, bottom } = normalizeMarquee(gesture.marquee);
      const isClick = right - left < DRAG_THRESHOLD_PX && bottom - top < DRAG_THRESHOLD_PX;

      if (isClick) {
        // A click on empty space with no drag: insert a note there.
        const tick = snapTickDown(xToTick(left, view), snap);
        const pitch = yToPitch(top, view);
        if (pitch >= MIN_PITCH && pitch <= MAX_PITCH && left > KEY_WIDTH) {
          const duration = snap > 0 ? snap : Math.round(ppq / 4);
          onEdit({
            kind: "insert",
            track: trackIndex,
            note: { pitch, start_ticks: tick, duration_ticks: duration, velocity: 100, channel: track.channel },
          });
          onPreviewNote(pitch);
        }
      } else {
        onSelectionChange(
          gesture.additive ? [...new Set([...selection, ...picked])] : picked,
        );
      }
    }

    setGesture({ type: "none" });
  }

  useRollWheel({ canvasRef, view, setPxPerTick, setScrollTicks, setTopPitch });

  useRollShortcuts({
    track,
    trackIndex,
    selection,
    onSelectionChange,
    onEdit,
    snap,
    ppq,
    timeSignature,
    playheadTicks,
    // A proposal on screen, or another mode holding the keyboard — the listen overlay
    // and the on-screen keys both type. Either way the roll keeps its mouse behaviour
    // and gives up its shortcuts, which is what lets ⌫, J and the arrows mean one thing
    // at a time.
    readOnly: Boolean(preview) || shortcutsSuspended,
  });

  // -- render --------------------------------------------------------------

  const cursor =
    gesture.type === "move" ? "grabbing" : gesture.type === "resize" ? "ew-resize" : "default";

  return (
    <div className="roll">
      <RollToolbar
        gridDivisor={gridDivisor}
        onGridDivisorChange={setGridDivisor}
        pxPerTick={pxPerTick}
        onPxPerTickChange={setPxPerTick}
        rowHeight={rowHeight}
        onRowHeightChange={setRowHeight}
        selectionCount={selection.length}
      />

      <div className="roll__canvas-wrap" ref={containerRef}>
        <canvas
          ref={canvasRef}
          className="roll__canvas"
          style={{ width: size.width, height: size.height, cursor }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
      </div>
    </div>
  );
}
