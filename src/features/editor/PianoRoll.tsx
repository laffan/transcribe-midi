import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { EditRequest, Note, TimeSignature, Track } from "../../lib/types";
import {
  GRID_OPTIONS,
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
  type Viewport,
  xToTick,
  yToPitch,
} from "./pianoRollGeometry";
import "./PianoRoll.css";

/** Height of the velocity lane beneath the roll. */
const VELOCITY_LANE_HEIGHT = 72;

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
}

type Gesture =
  | { type: "none" }
  | { type: "marquee"; marquee: Marquee; additive: boolean }
  | { type: "move"; startTick: number; startPitch: number; indices: number[]; moved: boolean }
  | { type: "resize"; startTick: number; indices: number[]; moved: boolean }
  | { type: "velocity"; indices: number[] }
  | { type: "scrub" };

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
  const [clipboard, setClipboard] = useState<Note[]>([]);

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
      if (rect) setSize({ width: Math.max(200, rect.width), height: Math.max(160, rect.height) });
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  const rollHeight = Math.max(80, size.height - VELOCITY_LANE_HEIGHT);

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
      // Velocity drives opacity so dynamics are legible at a glance.
      const alpha = 0.4 + (note.velocity / 127) * 0.6;

      ctx.globalAlpha = alpha;
      ctx.fillStyle = selected ? accent : track.color;
      const x = Math.max(KEY_WIDTH, rect.x);
      const width = rect.width - (x - rect.x);
      ctx.fillRect(x, rect.y + 1, Math.max(1, width), rect.height - 2);

      ctx.globalAlpha = 1;
      if (selected) {
        ctx.strokeStyle = keyWhite;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, rect.y + 1.5, Math.max(1, width) - 1, rect.height - 3);
      }
    });
    ctx.globalAlpha = 1;

    // ---- velocity lane ----
    const laneTop = rollHeight;
    ctx.fillStyle = bg1;
    ctx.fillRect(0, laneTop, size.width, VELOCITY_LANE_HEIGHT);
    ctx.strokeStyle = border;
    ctx.beginPath();
    ctx.moveTo(0, laneTop + 0.5);
    ctx.lineTo(size.width, laneTop + 0.5);
    ctx.stroke();

    ctx.fillStyle = text2;
    ctx.font = "10px ui-monospace, monospace";
    ctx.fillText("VELOCITY", 6, laneTop + 14);

    const laneBottom = laneTop + VELOCITY_LANE_HEIGHT - 6;
    const laneHeight = VELOCITY_LANE_HEIGHT - 22;

    track.notes.forEach((note, index) => {
      const x = tickToX(note.start_ticks, view);
      if (x < KEY_WIDTH || x > size.width) return;
      const height = (note.velocity / 127) * laneHeight;
      ctx.fillStyle = selectionSet.has(index) ? accent : track.color;
      ctx.globalAlpha = selectionSet.has(index) ? 1 : 0.7;
      ctx.fillRect(x, laneBottom - height, 3, height);
    });
    ctx.globalAlpha = 1;

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
    size, rollHeight, view, track, selectionSet, ppq, timeSignature,
    scrollTicks, pxPerTick, rowHeight, topPitch, snap, gesture, playheadTicks,
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

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
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

  function applyVelocityFromY(y: number, indices: number[]) {
    const laneBottom = rollHeight + VELOCITY_LANE_HEIGHT - 6;
    const laneHeight = VELOCITY_LANE_HEIGHT - 22;
    const ratio = Math.min(1, Math.max(0, (laneBottom - y) / laneHeight));
    onEdit({
      kind: "set_velocity",
      track: trackIndex,
      indices,
      velocity: Math.round(1 + ratio * 126),
    });
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

  // -- wheel: scroll and zoom ----------------------------------------------

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Registered non-passively so `preventDefault` works — otherwise the whole page
    // scrolls (and on a trackpad, pinch-zooms the webview) instead of the roll.
    function onWheel(event: WheelEvent) {
      event.preventDefault();
      const rect = canvas!.getBoundingClientRect();
      const x = event.clientX - rect.left;

      if (event.ctrlKey || event.metaKey) {
        // Zoom about the pointer, so the tick under the cursor stays put.
        const anchorTick = xToTick(x, view);
        const factor = Math.exp(-event.deltaY * 0.003);
        const next = Math.min(4, Math.max(0.005, pxPerTick * factor));
        setPxPerTick(next);
        setScrollTicks(Math.max(0, anchorTick - (x - KEY_WIDTH) / next));
        return;
      }

      if (event.shiftKey) {
        setScrollTicks((prev) => Math.max(0, prev + event.deltaY / pxPerTick));
        return;
      }

      setScrollTicks((prev) => Math.max(0, prev + event.deltaX / pxPerTick));
      setTopPitch((prev) =>
        Math.min(MAX_PITCH, Math.max(12, prev - Math.round(event.deltaY / rowHeight))),
      );
    }

    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [view, pxPerTick, rowHeight]);

  // -- keyboard ------------------------------------------------------------

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      // Never steal keys from a text field.
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }

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
  }, [selection, clipboard, track, trackIndex, onEdit, onSelectionChange, snap, ppq, timeSignature, playheadTicks]);

  // -- render --------------------------------------------------------------

  const cursor =
    gesture.type === "move" ? "grabbing" : gesture.type === "resize" ? "ew-resize" : "default";

  return (
    <div className="roll">
      <div className="roll__toolbar">
        <label className="roll__control">
          <span className="roll__control-label">Grid</span>
          <select
            className="input roll__select"
            value={gridDivisor}
            onChange={(e) => setGridDivisor(Number(e.target.value))}
          >
            {GRID_OPTIONS.map((option) => (
              <option key={option.label} value={option.divisor}>
                {option.label}
              </option>
            ))}
          </select>
        </label>

        <label className="roll__control">
          <span className="roll__control-label">Zoom</span>
          <input
            type="range"
            min={0.005}
            max={1}
            step={0.005}
            value={pxPerTick}
            onChange={(e) => setPxPerTick(Number(e.target.value))}
            className="roll__range"
            aria-label="Horizontal zoom"
          />
        </label>

        <label className="roll__control">
          <span className="roll__control-label">Rows</span>
          <input
            type="range"
            min={6}
            max={28}
            step={1}
            value={rowHeight}
            onChange={(e) => setRowHeight(Number(e.target.value))}
            className="roll__range"
            aria-label="Vertical zoom"
          />
        </label>

        <div className="spacer" />

        <span className="roll__hint muted">
          {selection.length > 0 ? `${selection.length} selected` : "click to add · drag to select"}
        </span>
      </div>

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
