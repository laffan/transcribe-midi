/**
 * Two-finger navigation for the piano roll.
 *
 * The roll is scrolled and zoomed with the wheel — plain to move, ⇧ to pan, ⌘ to zoom —
 * and a touchscreen has none of those. Without this the roll on a phone shows whatever
 * bars and pitches it happened to open on and there is no way to reach any others: the
 * canvas sets `touch-action: none` so the browser will not pan it, and every single
 * pointer is already spoken for by drawing, selecting, dragging and scrubbing.
 *
 * So the second finger is the navigation. Two fingers together pan; moving them apart
 * or together zooms time about the point between them, which is the same anchoring rule
 * the wheel handler uses so the two feel like one behaviour.
 *
 * The arithmetic that decides where the roll ends up is separated from the components
 * that call it and needs neither React nor a canvas to be checked — that is the part
 * worth being sure about. The wheel handler at the bottom of the file is a hook only
 * because it has to register its listener non-passively; the sums it does are the same
 * ones, and they live here so the two input devices cannot drift apart.
 */

import { type RefObject, useEffect } from "react";

import { KEY_WIDTH, MAX_PITCH, type Viewport, xToTick } from "./pianoRollGeometry";

export interface TouchPoint {
  x: number;
  y: number;
}

/** The zoom range the wheel handler and the toolbar slider also work within. */
const MIN_PX_PER_TICK = 0.005;
const MAX_PX_PER_TICK = 4;

/** The topmost pitch the roll will scroll to. Matches the wheel handler's floor. */
const MIN_TOP_PITCH = 12;

/**
 * Two touches this close together are not a measurable distance apart — the ratio
 * between two of them is noise, and dividing by one sends the zoom to infinity.
 */
const MIN_SPREAD_PX = 24;

export function centroid(a: TouchPoint, b: TouchPoint): TouchPoint {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

export function spread(a: TouchPoint, b: TouchPoint): number {
  return Math.max(MIN_SPREAD_PX, Math.hypot(a.x - b.x, a.y - b.y));
}

export interface PinchInput {
  /** Where the two touches were when the gesture began. */
  from: readonly [TouchPoint, TouchPoint];
  /** Where they are now. */
  to: readonly [TouchPoint, TouchPoint];
  /** The viewport when the gesture began. */
  view: Viewport;
}

/**
 * The viewport after a two-finger pan-and-pinch.
 *
 * Measured from the start of the gesture rather than from the previous frame: an
 * incremental version accumulates its own rounding, so a pinch out and back does not
 * return to where it started and the roll creeps under a finger that is holding still.
 *
 * Zoom is horizontal only. Vertical zoom — the height of a semitone — stays on its
 * slider, because a gesture that changed both at once would make every pan a small
 * accidental zoom in whichever axis the fingers were less careful about.
 */
export function pinchView({ from, to, view }: PinchInput): Viewport {
  const factor = spread(to[0], to[1]) / spread(from[0], from[1]);
  const pxPerTick = Math.min(
    MAX_PX_PER_TICK,
    Math.max(MIN_PX_PER_TICK, view.pxPerTick * factor),
  );

  const before = centroid(from[0], from[1]);
  const after = centroid(to[0], to[1]);

  // The tick under the midpoint of the fingers stays under it, so the roll moves with
  // the hand rather than around a corner of the canvas.
  const anchorTick = xToTick(before.x, view);
  const scrollTicks = Math.max(0, anchorTick - (after.x - KEY_WIDTH) / pxPerTick);

  // Dragging down reveals what is above, which on a roll where pitch descends means
  // higher notes — the same direction as dragging a piece of paper.
  const rows = Math.round((after.y - before.y) / view.rowHeight);
  const topPitch = Math.min(MAX_PITCH, Math.max(MIN_TOP_PITCH, view.topPitch + rows));

  return { pxPerTick, rowHeight: view.rowHeight, scrollTicks, topPitch };
}

/**
 * Whether a set of live pointers should be driving the roll rather than editing it.
 *
 * Two fingers navigate; one finger draws, selects, drags and scrubs exactly as a mouse
 * does. Deliberately counts pointers rather than asking whether the device has a
 * touchscreen: a two-finger gesture is impossible with a mouse, so no branch on pointer
 * type is needed and the desktop path is left byte-for-byte as it was.
 */
export function isNavigating(pointerCount: number): boolean {
  return pointerCount >= 2;
}

/**
 * The wheel and trackpad half of the same job: plain to scroll, ⇧ to pan along time,
 * ⌘ or ⌃ to zoom about the cursor.
 *
 * Here beside the touch gestures because the two are one behaviour with two input
 * devices, and because they have to agree — a pinch and a ⌘-scroll anchor on the same
 * rule, so zooming feels the same whichever hardware is doing it. Kept as a hook rather
 * than a handler prop because it must register non-passively: React's `onWheel` is
 * passive, so `preventDefault` is ignored there and the page scrolls (and, on a
 * trackpad, the whole webview pinch-zooms) instead of the roll.
 */
export interface RollWheelOptions {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  view: Viewport;
  setPxPerTick: (value: number) => void;
  setScrollTicks: (update: (previous: number) => number) => void;
  setTopPitch: (update: (previous: number) => number) => void;
}

export function useRollWheel({
  canvasRef,
  view,
  setPxPerTick,
  setScrollTicks,
  setTopPitch,
}: RollWheelOptions): void {
  const { pxPerTick, rowHeight } = view;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    function onWheel(event: WheelEvent) {
      event.preventDefault();
      const rect = canvas!.getBoundingClientRect();
      const x = event.clientX - rect.left;

      if (event.ctrlKey || event.metaKey) {
        // Zoom about the pointer, so the tick under the cursor stays put.
        const anchorTick = xToTick(x, view);
        const factor = Math.exp(-event.deltaY * 0.003);
        const next = Math.min(MAX_PX_PER_TICK, Math.max(MIN_PX_PER_TICK, pxPerTick * factor));
        setPxPerTick(next);
        setScrollTicks(() => Math.max(0, anchorTick - (x - KEY_WIDTH) / next));
        return;
      }

      if (event.shiftKey) {
        setScrollTicks((prev) => Math.max(0, prev + event.deltaY / pxPerTick));
        return;
      }

      setScrollTicks((prev) => Math.max(0, prev + event.deltaX / pxPerTick));
      setTopPitch((prev) =>
        Math.min(MAX_PITCH, Math.max(MIN_TOP_PITCH, prev - Math.round(event.deltaY / rowHeight))),
      );
    }

    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [canvasRef, view, pxPerTick, rowHeight, setPxPerTick, setScrollTicks, setTopPitch]);
}
