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
): { index: number; onResizeHandle: boolean } | null {
  for (let index = notes.length - 1; index >= 0; index -= 1) {
    const rect = noteRect(notes[index]!, index, view);
    if (x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height) {
      return {
        index,
        // Only offer the resize grip if the note is wide enough that the grip does not
        // swallow the whole body.
        onResizeHandle: rect.width > RESIZE_HANDLE_PX * 2 && x >= rect.x + rect.width - RESIZE_HANDLE_PX,
      };
    }
  }
  return null;
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
