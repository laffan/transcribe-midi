/**
 * The mapping between a transcription and the pixels it is drawn on.
 *
 * Pure functions over an explicit scale, for the same reason `pianoRollGeometry` is: the
 * arithmetic that turns seconds and semitones into a rectangle is where an editor like
 * this goes subtly wrong, and it can only be checked if it does not need a canvas.
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

export interface Scale {
  width: number;
  height: number;
  /** Length of the take, in seconds. Never zero — everything divides by it. */
  duration: number;
  /** Lowest and highest MIDI note the pitch lane spans. */
  low: number;
  high: number;
  ticksPerSecond: number;
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
  return (seconds / scale.duration) * scale.width;
}

export function secondsOf(scale: Scale, x: number): number {
  return (x / Math.max(1, scale.width)) * scale.duration;
}

/** Where the pitch lane starts and how tall it is, given the waveform above it. */
function lane(scale: Scale): { top: number; height: number } {
  return { top: WAVE_HEIGHT, height: Math.max(80, scale.height - WAVE_HEIGHT) };
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

export function noteRect(scale: Scale, note: Note): Rect {
  const start = note.start_ticks / scale.ticksPerSecond;
  const end = (note.start_ticks + note.duration_ticks) / scale.ticksPerSecond;
  const y = yOf(scale, note.pitch);
  return {
    x: xOf(scale, start),
    width: Math.max(3, xOf(scale, end) - xOf(scale, start)),
    y: y - NOTE_HEIGHT / 2,
    height: NOTE_HEIGHT,
  };
}

/**
 * The span of pitches worth showing: everything drawn, plus air either side, and never
 * so tight that a note dragged a semitone leaves the view.
 */
export function pitchRangeOf(notes: Note[], analysis: Analysis): { low: number; high: number } {
  const measured = analysis.frames.filter((f) => f.midi > 0).map((f) => f.midi);
  const all = [...notes.map((n) => n.pitch), ...measured];
  if (all.length === 0) return { low: 48, high: 72 };

  const low = Math.floor(Math.min(...all)) - 2;
  const high = Math.ceil(Math.max(...all)) + 2;
  return { low, high: Math.max(high, low + 12) };
}

/** Nearest detected attack within a small window, so a dragged edge lands on it. */
export function snapSeconds(analysis: Analysis, seconds: number): number {
  let best = seconds;
  let bestDistance = 0.05; // 50 ms — close enough to be what the user meant
  for (const frame of analysis.onsets) {
    const at = frame * analysis.hop_seconds;
    const distance = Math.abs(at - seconds);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = at;
    }
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

    if (x <= rect.x + EDGE_PX) return { type: "left", index };
    if (x >= rect.x + rect.width - EDGE_PX) return { type: "right", index };
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
