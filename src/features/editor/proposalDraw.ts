/**
 * Drawing a proposal: what is on offer, over what is already there.
 *
 * A sibling of `transcribeDraw` rather than a flag inside it. The two views share their
 * geometry and nothing else — there is no waveform here, no measured pitch line and no
 * onsets, and a single function that drew both would be mostly branches.
 *
 * The colours are the app's diff colours, which is the point: green is what would arrive,
 * and the dim grey underneath is what the track holds now, so "added a third above" is
 * something you can see rather than infer.
 */

import type { Note } from "../../lib/types";
import { noteRect, xOf, yOf, type Scale } from "./transcribeGeometry";

export interface Frame {
  scale: Scale;
  /** The notes being offered. */
  notes: Note[];
  /** The track as it stands, drawn behind them. */
  base: Note[];
  selected: number | null;
  playhead: number | null;
  /** One bar, in seconds, for the ruling. */
  barSeconds: number;
}

function token(style: CSSStyleDeclaration, name: string, fallback: string): string {
  return style.getPropertyValue(name).trim() || fallback;
}

export function drawProposal(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const { scale, notes, base, selected, playhead, barSeconds } = frame;
  const { width, height } = scale;

  const style = getComputedStyle(document.documentElement);
  const bgInset = token(style, "--bg-inset", "#0a0b0e");
  const gridBar = token(style, "--grid-line-bar", "#333a48");
  const grid = token(style, "--grid-line", "#21252e");
  const text2 = token(style, "--text-2", "#6f7689");
  const added = token(style, "--diff-added", "#6cb08a");

  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = bgInset;
  ctx.fillRect(0, 0, width, height);

  // ---- bars ----
  // Enough to place a note in time; not a full ruler, because nothing here is edited
  // against a grid — the roll behind the overlay is where that happens.
  if (barSeconds > 0.05) {
    ctx.font = "9px ui-monospace, monospace";
    for (let bar = 0; bar * barSeconds < scale.startSeconds + scale.spanSeconds; bar += 1) {
      const x = Math.round(xOf(scale, bar * barSeconds)) + 0.5;
      ctx.strokeStyle = gridBar;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
      ctx.fillStyle = text2;
      ctx.fillText(String(bar + 1), x + 3, 11);
    }
  }

  // ---- octave rules ----
  ctx.strokeStyle = grid;
  for (let midi = Math.ceil(scale.low); midi <= scale.high; midi += 1) {
    if (midi % 12 !== 0) continue;
    const y = Math.round(yOf(scale, midi)) + 0.5;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();
    ctx.fillStyle = text2;
    ctx.fillText(`C${Math.floor(midi / 12) - 1}`, 3, y - 2);
  }

  // ---- what is already on the track ----
  ctx.fillStyle = text2;
  ctx.globalAlpha = 0.35;
  for (const note of base) {
    const rect = noteRect(scale, note);
    ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
  }
  ctx.globalAlpha = 1;

  // ---- what is on offer ----
  notes.forEach((note, index) => {
    const rect = noteRect(scale, note);
    const isSelected = selected === index;

    ctx.globalAlpha = isSelected ? 1 : 0.85;
    ctx.fillStyle = added;
    ctx.fillRect(rect.x, rect.y, rect.width, rect.height);

    if (isSelected) {
      ctx.strokeStyle = token(style, "--key-white", "#e8eaf0");
      ctx.strokeRect(rect.x + 0.5, rect.y + 0.5, rect.width - 1, rect.height - 1);
    }
    ctx.globalAlpha = 1;
  });

  // ---- playhead ----
  if (playhead !== null) {
    const x = Math.round(xOf(scale, playhead)) + 0.5;
    ctx.strokeStyle = token(style, "--playhead", "#d9a441");
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x, 0);
    ctx.lineTo(x, height);
    ctx.stroke();
    ctx.lineWidth = 1;
  }
}
