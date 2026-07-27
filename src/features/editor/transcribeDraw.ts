/**
 * Drawing a take: the waveform, the pitch that was measured, the onsets, and the notes.
 *
 * Everything here was already computed by the transcriber. The point of showing it is
 * that a note drawn off its own pitch line is one the analysis was unsure about — which
 * is exactly the note worth checking — and that cannot be seen against a grid.
 */

import type { Analysis, Note, WaveformPeaks } from "../../lib/types";
import { noteRect, WAVE_HEIGHT, xOf, yOf, type Scale } from "./transcribeGeometry";

export interface Frame {
  scale: Scale;
  peaks: WaveformPeaks;
  notes: Note[];
  selected: number | null;
  analysis: Analysis;
  /** Where playback has reached, in seconds, or null when stopped. */
  playhead: number | null;
}

function token(style: CSSStyleDeclaration, name: string, fallback: string): string {
  return style.getPropertyValue(name).trim() || fallback;
}

export function drawTranscription(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const { scale, peaks, notes, selected, analysis, playhead } = frame;
  const { width, height } = scale;

  const style = getComputedStyle(document.documentElement);
  const bgInset = token(style, "--bg-inset", "#0a0b0e");
  const border = token(style, "--border", "#2b303b");
  const text2 = token(style, "--text-2", "#6f7689");
  const accent = token(style, "--accent", "#5b8dd9");
  const added = token(style, "--diff-added", "#6cb08a");
  const changed = token(style, "--diff-changed", "#d9a441");

  ctx.clearRect(0, 0, width, height);

  // ---- waveform ----
  ctx.fillStyle = bgInset;
  ctx.fillRect(0, 0, width, WAVE_HEIGHT);

  const mid = WAVE_HEIGHT / 2;
  ctx.strokeStyle = text2;
  ctx.globalAlpha = 0.75;
  ctx.beginPath();
  peaks.forEach(([min, max], index) => {
    const x = index + 0.5;
    ctx.moveTo(x, mid - max * (mid - 4));
    ctx.lineTo(x, mid - min * (mid - 4));
  });
  ctx.stroke();
  ctx.globalAlpha = 1;

  ctx.strokeStyle = border;
  ctx.beginPath();
  ctx.moveTo(0, WAVE_HEIGHT + 0.5);
  ctx.lineTo(width, WAVE_HEIGHT + 0.5);
  ctx.stroke();

  // ---- onsets ----
  // Drawn through both lanes: they are where a boundary snaps, so they need to be
  // visible against the waveform *and* against the notes.
  ctx.strokeStyle = changed;
  ctx.globalAlpha = 0.45;
  ctx.setLineDash([2, 3]);
  ctx.beginPath();
  for (const onset of analysis.onsets) {
    const x = Math.round(xOf(scale, onset * analysis.hop_seconds)) + 0.5;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, height);
  }
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.globalAlpha = 1;

  // ---- octave rules ----
  ctx.strokeStyle = border;
  ctx.globalAlpha = 0.5;
  ctx.font = "9px ui-monospace, monospace";
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
  ctx.globalAlpha = 1;

  // ---- the measured pitch line ----
  // The heart of the view. Confidence drives opacity, so a passage the tracker was
  // unsure about looks unsure rather than looking like a fact.
  let open = false;
  ctx.lineWidth = 2;
  analysis.frames.forEach((analysed, index) => {
    const voiced = analysed.midi > 0 && analysed.level > analysis.silence_floor;
    if (!voiced) {
      if (open) {
        ctx.stroke();
        open = false;
      }
      return;
    }
    const x = xOf(scale, index * analysis.hop_seconds);
    const y = yOf(scale, analysed.midi);
    if (!open) {
      ctx.beginPath();
      ctx.strokeStyle = accent;
      ctx.globalAlpha = 0.35 + Math.min(1, analysed.confidence) * 0.5;
      ctx.moveTo(x, y);
      open = true;
    } else {
      ctx.lineTo(x, y);
    }
  });
  if (open) ctx.stroke();
  ctx.globalAlpha = 1;
  ctx.lineWidth = 1;

  // ---- notes ----
  notes.forEach((note, index) => {
    const rect = noteRect(scale, note);
    const isSelected = selected === index;

    ctx.globalAlpha = isSelected ? 1 : 0.8;
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
