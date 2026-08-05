import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { EditRequest, Note, NoteDiff, TimeSignature, Track } from "../../lib/types";
import {
  gridTicks,
  hitTest,
  isBlackKey,
  KEY_WIDTH,
  MAX_PITCH,
  MIN_PITCH,
  type Marquee,
  normalizeMarquee,
  noteRect,
  notesInMarquee,
  pitchName,
  pitchToY,
  RULER_HEIGHT,
  snapTick,
  snapTickDown,
  tickToX,
  velocityLaneHeight,
  type Viewport,
  xToTick,
  yToPitch,
} from "./pianoRollGeometry";
import { isNavigating, pinchView, type TouchPoint, useRollWheel } from "./rollGestures";
import { RollToolbar } from "./RollToolbar";
import { useRollShortcuts } from "./useRollShortcuts";
import "./PianoRoll.css";

/** Below this drag distance a pointer-up counts as a click, not a drag. */
const DRAG_THRESHOLD_PX = 3;

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

type Gesture =
  | { type: "none" }
  | { type: "marquee"; marquee: Marquee; additive: boolean }
  | { type: "move"; startTick: number; startPitch: number; indices: number[]; moved: boolean }
  | { type: "resize"; startTick: number; indices: number[]; moved: boolean }
  | { type: "velocity"; indices: number[] }
  | { type: "scrub" }
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
  preview,
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

    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== size.width * dpr || canvas.height !== size.height * dpr) {
      canvas.width = size.width * dpr;
      canvas.height = size.height * dpr;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const style = getComputedStyle(document.documentElement);
    const token = (name: string, fallback: string) =>
      style.getPropertyValue(name).trim() || fallback;

    const bgInset = token("--bg-inset", "#0a0b0e");
    const bg1 = token("--bg-1", "#15171c");
    const gridLine = token("--grid-line", "#21252e");
    const gridBar = token("--grid-line-bar", "#333a48");
    const border = token("--border", "#2b303b");
    const text2 = token("--text-2", "#6f7689");
    const accent = token("--accent", "#5b8dd9");
    const playheadColor = token("--playhead", "#d9a441");
    const keyWhite = token("--key-white", "#e8eaf0");
    const keyBlack = token("--key-black", "#1c1f26");

    ctx.clearRect(0, 0, size.width, size.height);

    // ---- lane backgrounds (black-key rows sit darker, as on a real roll) ----
    ctx.fillStyle = bgInset;
    ctx.fillRect(KEY_WIDTH, RULER_HEIGHT, size.width - KEY_WIDTH, rollHeight - RULER_HEIGHT);

    const firstPitch = Math.min(MAX_PITCH, topPitch);
    const lastPitch = Math.max(MIN_PITCH, topPitch - Math.ceil((rollHeight - RULER_HEIGHT) / rowHeight));

    for (let pitch = firstPitch; pitch >= lastPitch; pitch -= 1) {
      const y = pitchToY(pitch, view);
      if (y > rollHeight) continue;
      if (isBlackKey(pitch)) {
        ctx.fillStyle = "rgba(0,0,0,0.22)";
        ctx.fillRect(KEY_WIDTH, y, size.width - KEY_WIDTH, rowHeight);
      }
      // Octave boundaries read as the strongest horizontal rule.
      if (pitch % 12 === 0) {
        ctx.strokeStyle = gridBar;
        ctx.beginPath();
        ctx.moveTo(KEY_WIDTH, y + rowHeight + 0.5);
        ctx.lineTo(size.width, y + rowHeight + 0.5);
        ctx.stroke();
      }
    }

    // ---- vertical grid ----
    const beatTicks = (ppq * 4) / timeSignature.denominator;
    const barTicks = beatTicks * timeSignature.numerator;
    const startTick = Math.max(0, scrollTicks);
    const endTick = xToTick(size.width, view);

    // Only draw subdivisions when they are far enough apart to be legible.
    const subdivision = snap > 0 && snap * pxPerTick >= 6 ? snap : beatTicks;

    ctx.lineWidth = 1;
    for (let tick = Math.floor(startTick / subdivision) * subdivision; tick <= endTick; tick += subdivision) {
      const x = Math.round(tickToX(tick, view)) + 0.5;
      if (x < KEY_WIDTH) continue;
      const isBar = tick % barTicks === 0;
      const isBeat = tick % beatTicks === 0;
      ctx.strokeStyle = isBar ? gridBar : gridLine;
      ctx.globalAlpha = isBar || isBeat ? 1 : 0.55;
      ctx.beginPath();
      ctx.moveTo(x, RULER_HEIGHT);
      ctx.lineTo(x, rollHeight);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // ---- notes ----
    track.notes.forEach((note, index) => {
      const rect = noteRect(note, index, view);
      if (rect.x + rect.width < KEY_WIDTH || rect.x > size.width) return;
      if (rect.y + rect.height < RULER_HEIGHT || rect.y > rollHeight) return;

      const selected = selectionSet.has(index);
      // Velocity drives opacity so dynamics are legible at a glance. Under a preview
      // everything recedes so the proposed change is what the eye lands on.
      const alpha = (0.4 + (note.velocity / 127) * 0.6) * (preview ? 0.3 : 1);

      ctx.globalAlpha = alpha;
      ctx.fillStyle = selected && !preview ? accent : track.color;
      const x = Math.max(KEY_WIDTH, rect.x);
      const width = rect.width - (x - rect.x);
      ctx.fillRect(x, rect.y + 1, Math.max(1, width), rect.height - 2);

      ctx.globalAlpha = 1;
      if (selected && !preview) {
        ctx.strokeStyle = keyWhite;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, rect.y + 1.5, Math.max(1, width) - 1, rect.height - 3);
      }
    });
    ctx.globalAlpha = 1;

    // ---- proposed change ----
    //
    // Three colours, one meaning each: green is new, red is going, amber is moving.
    // Removed and "before" notes are outlined rather than filled, so a filled block
    // always means a note that will exist once the change is accepted.
    if (preview) {
      const added = token("--diff-added", "#6cb08a");
      const removed = token("--diff-removed", "#d16b6b");
      const changed = token("--diff-changed", "#d9a441");

      const block = (note: Note, color: string, filled: boolean) => {
        const rect = noteRect(note, 0, view);
        if (rect.x + rect.width < KEY_WIDTH || rect.x > size.width) return;
        if (rect.y + rect.height < RULER_HEIGHT || rect.y > rollHeight) return;

        const x = Math.max(KEY_WIDTH, rect.x);
        const width = Math.max(1, rect.width - (x - rect.x));

        if (filled) {
          ctx.globalAlpha = 0.85;
          ctx.fillStyle = color;
          ctx.fillRect(x, rect.y + 1, width, rect.height - 2);
          ctx.globalAlpha = 1;
        } else {
          ctx.globalAlpha = 0.9;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 2]);
          ctx.strokeRect(x + 0.5, rect.y + 1.5, width - 1, rect.height - 3);
          ctx.setLineDash([]);
          ctx.globalAlpha = 1;
        }
      };

      preview.removed.forEach((note) => block(note, removed, false));
      preview.changed.forEach(({ before }) => block(before, changed, false));
      preview.changed.forEach(({ after }) => block(after, changed, true));
      preview.added.forEach((note) => block(note, added, true));
    }

    // ---- velocity lane ----
    if (velocityLane > 0) {
      const laneTop = rollHeight;
      ctx.fillStyle = bg1;
      ctx.fillRect(0, laneTop, size.width, velocityLane);
      ctx.strokeStyle = border;
      ctx.beginPath();
      ctx.moveTo(0, laneTop + 0.5);
      ctx.lineTo(size.width, laneTop + 0.5);
      ctx.stroke();

      ctx.fillStyle = text2;
      ctx.font = "10px ui-monospace, monospace";
      ctx.fillText("VELOCITY", 6, laneTop + 14);

      const laneBottom = laneTop + velocityLane - 6;
      const laneHeight = velocityLane - 22;

      track.notes.forEach((note, index) => {
        const x = tickToX(note.start_ticks, view);
        if (x < KEY_WIDTH || x > size.width) return;
        const height = (note.velocity / 127) * laneHeight;
        ctx.fillStyle = selectionSet.has(index) ? accent : track.color;
        ctx.globalAlpha = selectionSet.has(index) ? 1 : 0.7;
        ctx.fillRect(x, laneBottom - height, 3, height);
      });
      ctx.globalAlpha = 1;
    }

    // ---- ruler ----
    ctx.fillStyle = bg1;
    ctx.fillRect(0, 0, size.width, RULER_HEIGHT);
    ctx.strokeStyle = border;
    ctx.beginPath();
    ctx.moveTo(0, RULER_HEIGHT + 0.5);
    ctx.lineTo(size.width, RULER_HEIGHT + 0.5);
    ctx.stroke();

    ctx.fillStyle = text2;
    ctx.font = "10px ui-monospace, monospace";
    for (let tick = Math.floor(startTick / barTicks) * barTicks; tick <= endTick; tick += barTicks) {
      const x = tickToX(tick, view);
      if (x < KEY_WIDTH) continue;
      ctx.fillText(String(Math.floor(tick / barTicks) + 1), x + 3, 14);
      ctx.strokeStyle = gridBar;
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, 0);
      ctx.lineTo(Math.round(x) + 0.5, RULER_HEIGHT);
      ctx.stroke();
    }

    // ---- keyboard gutter ----
    ctx.fillStyle = bg1;
    ctx.fillRect(0, RULER_HEIGHT, KEY_WIDTH, size.height - RULER_HEIGHT);

    for (let pitch = firstPitch; pitch >= lastPitch; pitch -= 1) {
      const y = pitchToY(pitch, view);
      if (y > rollHeight || y + rowHeight < RULER_HEIGHT) continue;
      const black = isBlackKey(pitch);
      ctx.fillStyle = black ? keyBlack : keyWhite;
      ctx.fillRect(0, y, black ? KEY_WIDTH * 0.62 : KEY_WIDTH - 1, rowHeight - 1);

      if (pitch % 12 === 0 && rowHeight >= 9) {
        ctx.fillStyle = text2;
        ctx.font = "9px ui-monospace, monospace";
        ctx.fillText(pitchName(pitch), KEY_WIDTH - 22, y + rowHeight - 2);
      }
    }

    ctx.strokeStyle = border;
    ctx.beginPath();
    ctx.moveTo(KEY_WIDTH + 0.5, 0);
    ctx.lineTo(KEY_WIDTH + 0.5, size.height);
    ctx.stroke();

    // ---- marquee ----
    if (gesture.type === "marquee") {
      const { left, top, right, bottom } = normalizeMarquee(gesture.marquee);
      ctx.fillStyle = "rgba(91,141,217,0.16)";
      ctx.fillRect(left, top, right - left, bottom - top);
      ctx.strokeStyle = accent;
      ctx.setLineDash([3, 3]);
      ctx.strokeRect(left + 0.5, top + 0.5, right - left, bottom - top);
      ctx.setLineDash([]);
    }

    // ---- loop region ----
    if (loopRegion) {
      const [loopStart, loopEnd] = loopRegion;
      const startX = Math.max(KEY_WIDTH, tickToX(loopStart, view));
      const endX = Math.min(size.width, tickToX(loopEnd, view));

      if (endX > startX) {
        // A wash over the looped span, so it reads at a glance without obscuring notes.
        ctx.fillStyle = "rgba(217,164,65,0.07)";
        ctx.fillRect(startX, RULER_HEIGHT, endX - startX, rollHeight - RULER_HEIGHT);

        ctx.fillStyle = playheadColor;
        ctx.fillRect(startX, 0, 2, RULER_HEIGHT);
        ctx.fillRect(endX - 2, 0, 2, RULER_HEIGHT);
        ctx.globalAlpha = 0.35;
        ctx.fillRect(startX, 0, endX - startX, RULER_HEIGHT);
        ctx.globalAlpha = 1;
      }
    }

    // ---- playhead ----
    const playheadX = tickToX(playheadTicks, view);
    if (playheadX >= KEY_WIDTH && playheadX <= size.width) {
      ctx.strokeStyle = playheadColor;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(playheadX, 0);
      ctx.lineTo(playheadX, size.height);
      ctx.stroke();
    }
  }, [
    size, rollHeight, velocityLane, view, track, selectionSet, ppq, timeSignature,
    scrollTicks, pxPerTick, rowHeight, topPitch, snap, gesture, playheadTicks, loopRegion,
    preview,
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

    // Velocity lane, when there is one.
    if (velocityLane > 0 && y >= rollHeight) {
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
    pointers.current.delete(event.pointerId);

    // Lifting out of a pinch resolves to nothing rather than falling through to the
    // marquee branch below, which would read the gesture as a tap and draw a note
    // wherever the fingers happened to be.
    if (gesture.type === "pinch") {
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
    readOnly: Boolean(preview),
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
