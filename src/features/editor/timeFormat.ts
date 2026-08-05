/**
 * Reading a tick position out loud.
 *
 * Pure and dependency-free so the toolbar's clock, the transport and anything else that
 * needs to say where the playhead is all say it the same way.
 */

import type { Note, TimeSignature } from "../../lib/types";

export interface BarsBeats {
  /** One-based, the way every DAW counts. */
  bar: number;
  beat: number;
  tick: number;
}

/** How many ticks one beat of this time signature lasts. */
export function beatTicks(ppq: number, timeSignature: TimeSignature): number {
  return (ppq * 4) / timeSignature.denominator;
}

/** How many ticks one bar of this time signature lasts. */
export function barTicks(ppq: number, timeSignature: TimeSignature): number {
  return beatTicks(ppq, timeSignature) * timeSignature.numerator;
}

export function barsBeats(ticks: number, ppq: number, timeSignature: TimeSignature): BarsBeats {
  const beat = beatTicks(ppq, timeSignature);
  const bar = beat * timeSignature.numerator;
  const at = Math.max(0, ticks);

  return {
    bar: Math.floor(at / bar) + 1,
    beat: Math.floor((at % bar) / beat) + 1,
    tick: Math.floor(at % beat),
  };
}

/** Bars|beats|ticks, one-based, the way every DAW displays position. */
export function formatPosition(ticks: number, ppq: number, timeSignature: TimeSignature): string {
  const { bar, beat, tick } = barsBeats(ticks, ppq, timeSignature);
  return `${bar}.${beat}.${String(tick).padStart(3, "0")}`;
}

const PITCH_CLASSES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];

/** Scientific pitch notation, middle C as C4 — the same numbering the piano roll draws. */
export function noteName(pitch: number): string {
  const rounded = Math.round(pitch);
  return `${PITCH_CLASSES[((rounded % 12) + 12) % 12]}${Math.floor(rounded / 12) - 1}`;
}

/**
 * The note sounding at `tick`, or null in a gap.
 *
 * The last one wins where notes overlap: on a monophonic line there is nothing to choose
 * between, and on a chord the top of the stack is the one the eye is on.
 */
export function noteAt(notes: Note[], tick: number): Note | null {
  let found: Note | null = null;
  for (const note of notes) {
    if (note.start_ticks > tick) break; // sorted by start, so nothing later can match
    if (tick < note.start_ticks + note.duration_ticks) found = note;
  }
  return found;
}
