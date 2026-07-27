import type React from "react";
import { useMemo, useRef, useState } from "react";

import type { EditRequest, NoteDiff, Track } from "../../lib/types";
import {
  hitTest,
  KEY_WIDTH,
  MAX_PITCH,
  MIN_PITCH,
  type Marquee,
  normalizeMarquee,
  notesInMarquee,
  RULER_HEIGHT,
  snapTick,
  snapTickDown,
  velocityLane,
  type Viewport,
  xToTick,
  yToPitch,
} from "./pianoRollGeometry";

/** Below this drag distance a pointer-up counts as a click, not a drag. */
const DRAG_THRESHOLD_PX = 3;

type Gesture =
  | { type: "none" }
  | { type: "marquee"; marquee: Marquee; additive: boolean }
  | { type: "move"; startTick: number; startPitch: number; indices: number[]; moved: boolean }
  | { type: "resize"; startTick: number; indices: number[]; moved: boolean }
  | { type: "velocity"; indices: number[] }
  | { type: "scrub" };

interface GestureDeps {
  canvasRef: React.RefObject<HTMLCanvasElement | null>;
  track: Track;
  trackIndex: number;
  ppq: number;
  view: Viewport;
  snap: number;
  rollHeight: number;
  selection: number[];
  selectionSet: Set<number>;
  onSelectionChange: (indices: number[]) => void;
  onEdit: (request: EditRequest) => void;
  onPreviewNote: (pitch: number) => void;
  onScrub: (tick: number) => void;
  preview: NoteDiff | null | undefined;
}

/**
 * Every pointer gesture the roll understands, and which one is in progress.
 *
 * Which gesture a press starts is decided entirely by where it landed — ruler scrubs,
 * gutter auditions, velocity lane drags a level, a note moves or resizes, empty space
 * marquees or inserts. Keeping that dispatch in one function is what makes the regions
 * readable as a set rather than as scattered conditions.
 */
export function usePianoRollGestures({
  canvasRef,
  track,
  trackIndex,
  ppq,
  view,
  snap,
  rollHeight,
  selection,
  selectionSet,
  onSelectionChange,
  onEdit,
  onPreviewNote,
  onScrub,
  preview,
}: GestureDeps) {
  const [gesture, setGesture] = useState<Gesture>({ type: "none" });
  const dragOrigin = useRef({ x: 0, y: 0 });

  const localPoint = (event: React.PointerEvent): { x: number; y: number } => {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  };

  function applyVelocityFromY(y: number, indices: number[]) {
    const lane = velocityLane(rollHeight);
    const ratio = Math.min(1, Math.max(0, (lane.bottom - y) / lane.height));
    onEdit({
      kind: "set_velocity",
      track: trackIndex,
      indices,
      velocity: Math.round(1 + ratio * 126),
    });
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    // A proposal is on screen. Editing underneath it would invalidate it — Rust checks
    // and refuses on accept — so the roll is read-only until the user decides.
    if (preview) return;

    const { x, y } = localPoint(event);
    dragOrigin.current = { x, y };
    canvasRef.current?.setPointerCapture(event.pointerId);

    // Ruler: scrub.
    if (y < RULER_HEIGHT && x > KEY_WIDTH) {
      const tick = snapTick(xToTick(x, view), snap);
      onScrub(Math.max(0, tick));
      setGesture({ type: "scrub" });
      return;
    }

    // Keyboard gutter: audition the pitch.
    if (x < KEY_WIDTH && y > RULER_HEIGHT && y < rollHeight) {
      onPreviewNote(yToPitch(y, view));
      return;
    }

    // Velocity lane.
    if (y >= rollHeight) {
      const target = selection.length > 0 ? selection : [];
      if (target.length > 0) {
        setGesture({ type: "velocity", indices: target });
        applyVelocityFromY(y, target);
      }
      return;
    }

    const hit = hitTest(track.notes, x, y, view);

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

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    if (gesture.type === "none") return;
    const { x, y } = localPoint(event);

    if (gesture.type === "scrub") {
      onScrub(Math.max(0, snapTick(xToTick(x, view), snap)));
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

  const marquee = gesture.type === "marquee" ? gesture.marquee : null;

  const cursor = useMemo(
    () =>
      gesture.type === "move" ? "grabbing" : gesture.type === "resize" ? "ew-resize" : "default",
    [gesture.type],
  );

  return { marquee, cursor, onPointerDown, onPointerMove, onPointerUp };
}
