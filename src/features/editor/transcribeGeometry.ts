/**
 * The mapping between a transcription and the pixels it is drawn on.
 *
 * Pure functions over an explicit scale, for the same reason `pianoRollGeometry` is: the
 * arithmetic that turns seconds and semitones into a rectangle is where an editor like
 * this goes subtly wrong, and it can only be checked if it does not need a canvas.
 *
 * The scale carries a *window* rather than the whole take, because correcting a note is
 * close work: at six seconds across a display one semitone is a few pixels tall and a
 * tenth of a second is invisible, which is not something a steadier hand fixes. Zooming
 * is therefore not a convenience here — it is what makes the editing possible — and the
 * window is the one piece of state every other function reads.
 */

import type { Analysis, Note } from "../../lib/types";

/** Height of the waveform lane, in CSS pixels. */
export const WAVE_HEIGHT = 96;
/** Vertical padding inside the pitch lane. */
export const PITCH_PAD = 12;
/** Grab width, in pixels, of a note's resize edge. */
export const EDGE_PX = 6;
/** Height of a note box, in pixels. */
export const NOTE_HEIGHT = 14;

/** As far in as time zooms: 60 ms across the width, which is one pitch period of a bass note. */
export const MIN_SPAN_SECONDS = 0.06;
/** As far in as pitch zooms. Two semitones fill the lane; below that there is nothing left to see. */
export const MIN_SPAN_SEMITONES = 2;
export const MAX_SPAN_SEMITONES = 96;

/**
 * What is on screen: a stretch of time and a stretch of pitch.
 *
 * Kept as span-and-origin rather than start-and-end because every zoom is "the same
 * amount of span, somewhere else" and every pan is "the same span, further along" —
 * expressed that way the two operations cannot corrupt each other's invariant.
 */
export interface Window {
  /** Left edge, in seconds. */
  startSeconds: number;
  /** How much time the width spans. Never zero — everything divides by it. */
  spanSeconds: number;
  /** Lowest MIDI note drawn, at the bottom of the lane. Fractional. */
  low: number;
  /** Highest MIDI note drawn. Always above `low`. */
  high: number;
}

export interface Scale extends Window {
  width: number;
  height: number;
  ticksPerSecond: number;
  /**
   * Where the note lane begins. [`WAVE_HEIGHT`] under a waveform; zero when there is no
   * recording behind the notes, which is every proposal the model makes.
   */
  laneTop: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type Drag =
  | { type: "none" }
  | { type: "move"; index: number; grabSeconds: number; startPitch: number; startY: number }
  | { type: "left"; index: number }
  | { type: "right"; index: number };

export function xOf(scale: Scale, seconds: number): number {
  return ((seconds - scale.startSeconds) / scale.spanSeconds) * scale.width;
}

export function secondsOf(scale: Scale, x: number): number {
  return scale.startSeconds + (x / Math.max(1, scale.width)) * scale.spanSeconds;
}

/** Where the note lane starts and how tall it is, given whatever sits above it. */
function lane(scale: Scale): { top: number; height: number } {
  return { top: scale.laneTop, height: Math.max(80, scale.height - scale.laneTop) };
}

export function yOf(scale: Scale, midi: number): number {
  const { top, height } = lane(scale);
  const t = (midi - scale.low) / (scale.high - scale.low);
  return top + height - PITCH_PAD - t * (height - PITCH_PAD * 2);
}

export function midiOf(scale: Scale, y: number): number {
  const { top, height } = lane(scale);
  const t = (top + height - PITCH_PAD - y) / (height - PITCH_PAD * 2);
  return scale.low + t * (scale.high - scale.low);
}

/** How tall one semitone is on screen. The measure of whether a pitch drag is doable. */
export function semitonePx(scale: Scale): number {
  const { height } = lane(scale);
  return (height - PITCH_PAD * 2) / Math.max(1, scale.high - scale.low);
}

export function noteRect(scale: Scale, note: Note): Rect {
  const start = note.start_ticks / scale.ticksPerSecond;
  const end = (note.start_ticks + note.duration_ticks) / scale.ticksPerSecond;
  const y = yOf(scale, note.pitch);
  // Boxes grow with the pitch zoom, up to a point: at four semitones on screen a 14px
  // box would be a hairline in a lane of empty space, and the box is what you grab.
  const height = Math.max(NOTE_HEIGHT, Math.min(40, semitonePx(scale) * 0.7));
  return {
    x: xOf(scale, start),
    width: Math.max(3, xOf(scale, end) - xOf(scale, start)),
    y: y - height / 2,
    height,
  };
}

/**
 * The window a take opens on: all of it, with the pitches that are actually in it.
 *
 * Never tighter than an octave, so the first note dragged upward has somewhere to go
 * without the lane rescaling under the finger holding it.
 */
export function fitWindow(notes: Note[], analysis: Analysis, duration: number): Window {
  const measured = analysis.frames.filter((f) => f.midi > 0).map((f) => f.midi);
  const all = [...notes.map((n) => n.pitch), ...measured];
  if (all.length === 0) {
    return { startSeconds: 0, spanSeconds: Math.max(0.001, duration), low: 48, high: 72 };
  }

  const low = Math.floor(Math.min(...all)) - 2;
  const high = Math.ceil(Math.max(...all)) + 2;
  return {
    startSeconds: 0,
    spanSeconds: Math.max(0.001, duration),
    low,
    high: Math.max(high, low + 12),
  };
}

/** Keep a window inside the take, and inside the limits of what is worth drawing. */
export function clampWindow(window: Window, duration: number): Window {
  const spanSeconds = Math.min(
    Math.max(duration, MIN_SPAN_SECONDS),
    Math.max(MIN_SPAN_SECONDS, window.spanSeconds),
  );
  // Half a window of run-off at each end: a note at the very start or the very end of a
  // take has to be reachable with room to work either side of it.
  const startSeconds = Math.min(
    Math.max(0, duration - spanSeconds / 2),
    Math.max(-spanSeconds / 2, window.startSeconds),
  );

  const spanSemitones = Math.min(
    MAX_SPAN_SEMITONES,
    Math.max(MIN_SPAN_SEMITONES, window.high - window.low),
  );
  const centre = (window.high + window.low) / 2;
  const low = Math.min(127 - spanSemitones, Math.max(0, centre - spanSemitones / 2));

  return { startSeconds, spanSeconds, low, high: low + spanSemitones };
}

/** Zoom time by `factor` about a moment that must stay where it is on screen. */
export function zoomTime(
  window: Window,
  factor: number,
  anchorSeconds: number,
  duration: number,
): Window {
  const spanSeconds = window.spanSeconds / factor;
  const at = (anchorSeconds - window.startSeconds) / window.spanSeconds;
  return clampWindow(
    { ...window, spanSeconds, startSeconds: anchorSeconds - at * spanSeconds },
    duration,
  );
}

/** Zoom pitch by `factor` about a pitch that must stay where it is on screen. */
export function zoomPitch(
  window: Window,
  factor: number,
  anchorMidi: number,
  duration: number,
): Window {
  const span = (window.high - window.low) / factor;
  const at = (anchorMidi - window.low) / (window.high - window.low);
  const low = anchorMidi - at * span;
  return clampWindow({ ...window, low, high: low + span }, duration);
}

export function panWindow(
  window: Window,
  deltaSeconds: number,
  deltaSemitones: number,
  duration: number,
): Window {
  return clampWindow(
    {
      ...window,
      startSeconds: window.startSeconds + deltaSeconds,
      low: window.low + deltaSemitones,
      high: window.high + deltaSemitones,
    },
    duration,
  );
}

export interface TouchPoint {
  x: number;
  y: number;
}

/** Two touches this close together are noise, and dividing by one sends zoom to infinity. */
const MIN_SPREAD_PX = 24;

function spread(a: TouchPoint, b: TouchPoint): { x: number; y: number; distance: number } {
  return {
    x: Math.max(MIN_SPREAD_PX, Math.abs(a.x - b.x)),
    y: Math.max(MIN_SPREAD_PX, Math.abs(a.y - b.y)),
    distance: Math.max(MIN_SPREAD_PX, Math.hypot(a.x - b.x, a.y - b.y)),
  };
}

function centroid(a: TouchPoint, b: TouchPoint): TouchPoint {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

/** Which way a pinch is pointing, decided once when it starts and then held. */
export type PinchAxis = "time" | "pitch";

/**
 * Whether the fingers are separated more across than down.
 *
 * Fixed at the start of the gesture, and this is the whole reason the two axes can share
 * one gesture: a pinch that re-decided every frame would flip axis in the middle of a
 * squeeze, and a pinch that zoomed both would make every sideways spread a little
 * accidental pitch zoom. The piano roll settles the same question by zooming time only
 * and leaving pitch on a slider; there is no room for a slider here, and pitch is the
 * axis this view exists to work in.
 */
export function pinchAxisOf(a: TouchPoint, b: TouchPoint): PinchAxis {
  return Math.abs(a.x - b.x) >= Math.abs(a.y - b.y) ? "time" : "pitch";
}

export interface PinchInput {
  /** Where the two touches were when the gesture began, in canvas pixels. */
  from: readonly [TouchPoint, TouchPoint];
  /** Where they are now. */
  to: readonly [TouchPoint, TouchPoint];
  /** The window when the gesture began, and the scale it was drawn with. */
  view: Window;
  scale: Scale;
  axis: PinchAxis;
  duration: number;
}

/**
 * The window after a two-finger pan-and-pinch.
 *
 * Measured from the start of the gesture rather than from the previous frame: an
 * incremental version accumulates its own rounding, so a pinch out and back does not
 * return to where it started and the take creeps under a finger that is holding still.
 */
export function pinchWindow({ from, to, view, scale, axis, duration }: PinchInput): Window {
  const before = spread(from[0], from[1]);
  const after = spread(to[0], to[1]);
  const factor = axis === "time" ? after.x / before.x : after.y / before.y;

  const startCentre = centroid(from[0], from[1]);
  const endCentre = centroid(to[0], to[1]);

  // The moment and the pitch under the middle of the fingers stay under it, so the take
  // moves with the hand rather than around a corner of the canvas.
  const anchorSeconds = secondsOf(scale, startCentre.x);
  const anchorMidi = midiOf(scale, startCentre.y);

  const zoomed =
    axis === "time"
      ? zoomTime(view, factor, anchorSeconds, duration)
      : zoomPitch(view, factor, anchorMidi, duration);

  const zoomedScale: Scale = { ...scale, ...zoomed };
  return clampWindow(
    {
      ...zoomed,
      startSeconds:
        zoomed.startSeconds - (secondsOf(zoomedScale, endCentre.x) - anchorSeconds),
      low: zoomed.low - (midiOf(zoomedScale, endCentre.y) - anchorMidi),
      high: zoomed.high - (midiOf(zoomedScale, endCentre.y) - anchorMidi),
    },
    duration,
  );
}

/**
 * Whether a set of live pointers should be navigating rather than editing.
 *
 * Two fingers navigate; one finger drags a note exactly as a mouse does. Counts pointers
 * rather than asking whether the device has a touchscreen — a two-finger gesture is
 * impossible with a mouse, so the desktop path needs no branch of its own.
 */
export function isNavigating(pointerCount: number): boolean {
  return pointerCount >= 2;
}

/** How an edit lands on the timeline: on nothing, on the attacks, or on a musical grid. */
export interface Snap {
  /** Detected attacks, which is where a note boundary usually belongs. */
  attacks: boolean;
  /** Grid spacing in seconds, or 0 for none. */
  gridSeconds: number;
}

export const NO_SNAP: Snap = { attacks: false, gridSeconds: 0 };

/**
 * The moment an edge dragged to `seconds` should take.
 *
 * Snapping used to be unconditional: every dragged boundary jumped to the nearest
 * detected attack within 50 ms whatever the settings said. That is right nine times out
 * of ten and impossible to defeat the tenth — the one where the attack was detected in
 * the wrong place, which is exactly when someone is dragging the edge by hand. So it is
 * a setting now, and "Snap to: Off" means nothing snaps.
 */
export function snapSeconds(analysis: Analysis, seconds: number, snap: Snap): number {
  let best = seconds;
  let bestDistance = 0.05; // 50 ms — close enough to be what the user meant

  if (snap.attacks) {
    for (const frame of analysis.onsets) {
      const at = frame * analysis.hop_seconds;
      const distance = Math.abs(at - seconds);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = at;
      }
    }
  }

  // The grid only gets a say where no attack claimed it: a note starts where it was
  // played, and the grid is a guess about where it was meant to be.
  if (snap.gridSeconds > 0 && best === seconds) {
    best = Math.round(seconds / snap.gridSeconds) * snap.gridSeconds;
  }

  return Math.max(0, best);
}

/**
 * What a press at (x, y) grabs. Topmost note wins, and its outer few pixels resize
 * rather than move.
 */
export function hitTest(scale: Scale, notes: Note[], x: number, y: number): Drag {
  for (let index = notes.length - 1; index >= 0; index -= 1) {
    const rect = noteRect(scale, notes[index]!);
    if (x < rect.x - EDGE_PX || x > rect.x + rect.width + EDGE_PX) continue;
    if (y < rect.y - 4 || y > rect.y + rect.height + 4) continue;

    // A note narrower than three grab widths is all edge and no middle, and the middle
    // is the gesture you cannot get any other way.
    const edge = Math.min(EDGE_PX, rect.width / 3);
    if (x <= rect.x + edge) return { type: "left", index };
    if (x >= rect.x + rect.width - edge) return { type: "right", index };
    return {
      type: "move",
      index,
      grabSeconds: secondsOf(scale, x) - notes[index]!.start_ticks / scale.ticksPerSecond,
      startPitch: notes[index]!.pitch,
      startY: y,
    };
  }
  return { type: "none" };
}

/**
 * How far off equal temperament the take actually was, under one note, in cents.
 *
 * Read from the frames rather than carried on the note, so it survives every edit: the
 * notes come back from Rust re-derived, and an index into the original detection would
 * be pointing at a different note by the second drag. `null` when the tracker had
 * nothing confident to say there, which is not the same as "in tune".
 */
export function measuredCents(
  analysis: Analysis,
  note: Note,
  ticksPerSecond: number,
): number | null {
  const start = note.start_ticks / ticksPerSecond;
  const end = (note.start_ticks + note.duration_ticks) / ticksPerSecond;

  let total = 0;
  let count = 0;
  for (const [index, frame] of analysis.frames.entries()) {
    const at = index * analysis.hop_seconds;
    if (at < start || at >= end) continue;
    if (frame.midi <= 0 || frame.level <= analysis.silence_floor) continue;
    total += frame.midi;
    count += 1;
  }

  if (count === 0) return null;
  return Math.round((total / count - note.pitch) * 100);
}
