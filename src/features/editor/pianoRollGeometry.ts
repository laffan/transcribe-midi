/**
 * Coordinate maths for the piano roll, kept out of the component so it can be reasoned
 * about (and unit-tested) without a canvas or a DOM.
 */

import type { Note } from "../../lib/types";

export const KEY_WIDTH = 60;
export const RULER_HEIGHT = 22;
export const MIN_PITCH = 0;
export const MAX_PITCH = 127;
export const PITCH_COUNT = MAX_PITCH - MIN_PITCH + 1;

/** Pixels within which a pointer counts as grabbing a note's right edge to resize. */
export const RESIZE_HANDLE_PX = 6;

/** Height of the velocity lane beneath the roll when there is room for it. */
export const VELOCITY_LANE_HEIGHT = 72;

/** The least it can be and still be a bar chart you can drag the top of. */
const VELOCITY_LANE_MIN = 40;

/** Below this the grid is not enough semitones to place a note against. */
const MIN_GRID_HEIGHT = 96;

/**
 * The most of the roll's height the lane may take — a quarter of it.
 *
 * A fixed 72px is a quarter of a desktop roll and a third of a phone's, so the same
 * number that reads as a footnote on one screen dominates the other. Velocity is an
 * adjustment to notes that already exist; the grid is where they come from, and the
 * grid is what should get the pixels when there are fewer of them.
 */
const VELOCITY_LANE_SHARE = 0.25;

/**
 * How much of the roll's height the velocity lane may take.
 *
 * It is the first thing to give when the roll is short. A phone lying down leaves the
 * roll about 110pt in total; 72 of them spent on velocity would leave no grid to put
 * notes on. So it shrinks — to a smaller lane that is still a bar chart you can drag —
 * and below the point where it would be too thin to aim at it goes entirely. A lane you
 * cannot drag is worse than no lane, because it still costs the height.
 *
 * Everything that draws or hit-tests the lane reads this, so the two cannot end up
 * disagreeing about where the roll stops and the lane starts.
 */
export function velocityLaneHeight(available: number): number {
  const cap = available * VELOCITY_LANE_SHARE;
  for (const lane of [VELOCITY_LANE_HEIGHT, VELOCITY_LANE_MIN]) {
    if (lane <= cap && available - lane >= MIN_GRID_HEIGHT) return lane;
  }
  return 0;
}

export interface Viewport {
  /** Horizontal zoom. */
  pxPerTick: number;
  /** Vertical zoom — height of one semitone lane. */
  rowHeight: number;
  /** Leftmost visible tick. */
  scrollTicks: number;
  /** Topmost visible pitch (pitches descend down the screen). */
  topPitch: number;
}

/** Semitones within an octave that are black keys. */
const BLACK_KEYS = new Set([1, 3, 6, 8, 10]);

export function isBlackKey(pitch: number): boolean {
  return BLACK_KEYS.has(((pitch % 12) + 12) % 12);
}

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

export function pitchName(pitch: number): string {
  const name = NAMES[((pitch % 12) + 12) % 12]!;
  // MIDI 60 is C4 in the scientific convention Logic and most DAWs display.
  const octave = Math.floor(pitch / 12) - 1;
  return `${name}${octave}`;
}

export function tickToX(tick: number, view: Viewport): number {
  return KEY_WIDTH + (tick - view.scrollTicks) * view.pxPerTick;
}

export function xToTick(x: number, view: Viewport): number {
  return (x - KEY_WIDTH) / view.pxPerTick + view.scrollTicks;
}

export function pitchToY(pitch: number, view: Viewport): number {
  return RULER_HEIGHT + (view.topPitch - pitch) * view.rowHeight;
}

export function yToPitch(y: number, view: Viewport): number {
  return view.topPitch - Math.floor((y - RULER_HEIGHT) / view.rowHeight);
}

export function snapTick(tick: number, gridTicks: number): number {
  if (gridTicks <= 0) return Math.max(0, Math.round(tick));
  return Math.max(0, Math.round(tick / gridTicks) * gridTicks);
}

/** Snap toward zero, for placing a note where the user clicked rather than after it. */
export function snapTickDown(tick: number, gridTicks: number): number {
  if (gridTicks <= 0) return Math.max(0, Math.floor(tick));
  return Math.max(0, Math.floor(tick / gridTicks) * gridTicks);
}

export interface NoteRect {
  index: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export function noteRect(note: Note, index: number, view: Viewport): NoteRect {
  const x = tickToX(note.start_ticks, view);
  const width = Math.max(2, note.duration_ticks * view.pxPerTick);
  return {
    index,
    x,
    y: pitchToY(note.pitch, view),
    width,
    height: view.rowHeight,
  };
}

/**
 * Topmost note under a point, or null.
 *
 * Searched back-to-front so that when notes overlap, the one drawn last (visually on
 * top) is the one you grab — which is what a user expects.
 */
export function hitTest(
  notes: Note[],
  x: number,
  y: number,
  view: Viewport,
  grab: number = RESIZE_HANDLE_PX,
): { index: number; onResizeHandle: boolean } | null {
  for (let index = notes.length - 1; index >= 0; index -= 1) {
    const rect = noteRect(notes[index]!, index, view);
    if (x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height) {
      // Only offer the resize grip if the note is wide enough that the grip does not
      // swallow the whole body. `grab` is wider than the grip is drawn, and wider again
      // for a finger, because the band you have to hit is not the band you can see.
      const band = Math.min(grab, rect.width / 3);
      return {
        index,
        onResizeHandle: rect.width > RESIZE_HANDLE_PX * 2 && x >= rect.x + rect.width - band,
      };
    }
  }
  return null;
}

/** Which end of the loop a press at `x` has hold of, if either. */
export function loopEdgeAt(
  loop: [number, number] | null,
  x: number,
  view: Viewport,
  grab: number,
): "start" | "end" | null {
  if (!loop) return null;
  const toStart = Math.abs(x - tickToX(loop[0], view));
  const toEnd = Math.abs(x - tickToX(loop[1], view));
  if (Math.min(toStart, toEnd) > grab) return null;
  return toStart <= toEnd ? "start" : "end";
}

/**
 * A loop from two ticks, ordered and never shorter than one grid step.
 *
 * A zero-length loop is a click that happened to travel one pixel, and playing it would
 * be a stutter rather than a bar; a grid step is the smallest thing anyone means.
 */
export function normalizeLoopTicks(
  a: number,
  b: number,
  minimum: number,
): [number, number] {
  const from = Math.max(0, Math.min(a, b));
  const to = Math.max(a, b);
  return [from, Math.max(to, from + Math.max(1, minimum))];
}

export interface Marquee {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export function normalizeMarquee(m: Marquee): { left: number; top: number; right: number; bottom: number } {
  return {
    left: Math.min(m.x0, m.x1),
    right: Math.max(m.x0, m.x1),
    top: Math.min(m.y0, m.y1),
    bottom: Math.max(m.y0, m.y1),
  };
}

/** Indices of every note intersecting the marquee. */
export function notesInMarquee(notes: Note[], marquee: Marquee, view: Viewport): number[] {
  const { left, top, right, bottom } = normalizeMarquee(marquee);
  const out: number[] = [];

  notes.forEach((note, index) => {
    const rect = noteRect(note, index, view);
    const intersects =
      rect.x < right && rect.x + rect.width > left && rect.y < bottom && rect.y + rect.height > top;
    if (intersects) out.push(index);
  });

  return out;
}

/** Grid divisions offered in the toolbar, as a fraction of a whole note. */
export const GRID_OPTIONS = [
  { label: "1/1", divisor: 1 },
  { label: "1/2", divisor: 2 },
  { label: "1/4", divisor: 4 },
  { label: "1/8", divisor: 8 },
  { label: "1/16", divisor: 16 },
  { label: "1/32", divisor: 32 },
  { label: "1/8T", divisor: 12 },
  { label: "1/16T", divisor: 24 },
  { label: "Off", divisor: 0 },
] as const;

/** Ticks per grid division. A divisor of 0 means snapping is off. */
export function gridTicks(ppq: number, divisor: number): number {
  if (divisor <= 0) return 0;
  return Math.max(1, Math.round((ppq * 4) / divisor));
}
