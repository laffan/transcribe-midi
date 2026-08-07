/**
 * Drawing a take: the waveform, the pitch that was measured, the onsets, and the notes.
 *
 * Everything here was already computed by the transcriber. The point of showing it is
 * that a note drawn off its own pitch line is one the analysis was unsure about — which
 * is exactly the note worth checking — and that cannot be seen against a grid.
 *
 * Everything is drawn through the scale's window, so what the arithmetic decides is on
 * screen and what is painted cannot disagree. The only thing that changes with the zoom
 * is how much *detail* is worth drawing: pitch rules go from an octave apart to a
 * semitone apart, and the time ruler picks a step that leaves labels legible.
 */

import type { Analysis, Note, WaveformPeaks } from "../../lib/types";
import { noteName } from "./timeFormat";
import { noteRect, semitonePx, WAVE_HEIGHT, xOf, yOf, type Scale } from "./transcribeGeometry";

export interface Frame {
  scale: Scale;
  /** One min/max pair per pixel of the *window*, not of the take. */
  peaks: WaveformPeaks;
  notes: Note[];
  selected: number | null;
  analysis: Analysis;
  /** Where playback has reached, in seconds, or null when stopped. */
  playhead: number | null;
  /** Whether the window is being played round and round. */
  loop: boolean;
}

function token(style: CSSStyleDeclaration, name: string, fallback: string): string {
  return style.getPropertyValue(name).trim() || fallback;
}

/** A ruler step that leaves room between labels at the current zoom. */
function timeStep(spanSeconds: number): number {
  for (const step of [0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30]) {
    if (spanSeconds / step <= 12) return step;
  }
  return 60;
}

export function drawTranscription(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const { scale, peaks, notes, selected, analysis, playhead, loop } = frame;
  const { width, height } = scale;
  const windowEnd = scale.startSeconds + scale.spanSeconds;

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

  // ---- time ruler ----
  // Only worth its ink once the window is shorter than the take: at full zoom-out the
  // waveform itself says where you are.
  const step = timeStep(scale.spanSeconds);
  ctx.font = "9px ui-monospace, monospace";
  ctx.textAlign = "left";
  for (let at = Math.ceil(scale.startSeconds / step) * step; at < windowEnd; at += step) {
    const x = Math.round(xOf(scale, at)) + 0.5;
    ctx.strokeStyle = border;
    ctx.globalAlpha = 0.7;
    ctx.beginPath();
    ctx.moveTo(x, WAVE_HEIGHT - 10);
    ctx.lineTo(x, WAVE_HEIGHT);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.fillStyle = text2;
    ctx.fillText(step < 1 ? `${at.toFixed(2)}s` : `${at.toFixed(1)}s`, x + 3, WAVE_HEIGHT - 3);
  }

  ctx.strokeStyle = border;
  ctx.globalAlpha = 1;
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
    const at = onset * analysis.hop_seconds;
    if (at < scale.startSeconds || at > windowEnd) continue;
    const x = Math.round(xOf(scale, at)) + 0.5;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, height);
  }
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.globalAlpha = 1;

  // ---- pitch rules ----
  // Every semitone once there is room to tell them apart, which is what turns a zoomed
  // lane into something you can judge a note against; octaves only, before that.
  const rowPx = semitonePx(scale);
  const everySemitone = rowPx >= 11;
  ctx.font = "9px ui-monospace, monospace";
  for (let midi = Math.ceil(scale.low); midi <= scale.high; midi += 1) {
    const isOctave = midi % 12 === 0;
    if (!everySemitone && !isOctave) continue;

    const y = Math.round(yOf(scale, midi)) + 0.5;
    ctx.strokeStyle = border;
    ctx.globalAlpha = isOctave ? 0.5 : 0.22;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();

    if (isOctave || rowPx >= 16) {
      ctx.globalAlpha = isOctave ? 0.9 : 0.5;
      ctx.fillStyle = text2;
      ctx.fillText(noteName(midi), 3, y - 2);
    }
  }
  ctx.globalAlpha = 1;

  // ---- the measured pitch line ----
  // The heart of the view. Confidence drives opacity, so a passage the tracker was
  // unsure about looks unsure rather than looking like a fact.
  let open = false;
  ctx.lineWidth = 2;
  analysis.frames.forEach((analysed, index) => {
    const at = index * analysis.hop_seconds;
    const voiced =
      analysed.midi > 0 &&
      analysed.level > analysis.silence_floor &&
      at >= scale.startSeconds - analysis.hop_seconds &&
      at <= windowEnd + analysis.hop_seconds;
    if (!voiced) {
      if (open) {
        ctx.stroke();
        open = false;
      }
      return;
    }
    const x = xOf(scale, at);
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
    if (rect.x + rect.width < 0 || rect.x > width) return;
    const isSelected = selected === index;

    ctx.globalAlpha = isSelected ? 1 : 0.8;
    ctx.fillStyle = added;
    ctx.fillRect(rect.x, rect.y, rect.width, rect.height);

    if (isSelected) {
      ctx.strokeStyle = token(style, "--key-white", "#e8eaf0");
      ctx.strokeRect(rect.x + 0.5, rect.y + 0.5, rect.width - 1, rect.height - 1);
      // Handles, because the two gestures at a note's ends are not discoverable on a
      // touchscreen and the selected note is the one being worked on.
      ctx.fillStyle = token(style, "--key-white", "#e8eaf0");
      ctx.fillRect(rect.x, rect.y, 2, rect.height);
      ctx.fillRect(rect.x + rect.width - 2, rect.y, 2, rect.height);
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

  // ---- the loop ----
  // What repeats is what is on screen, so the mark is on the frame rather than inside
  // it: a band across the window would be a band across everything.
  if (loop) {
    ctx.fillStyle = accent;
    ctx.fillRect(0, 0, width, 3);
    ctx.fillRect(0, 0, 3, WAVE_HEIGHT);
    ctx.fillRect(width - 3, 0, 3, WAVE_HEIGHT);
  }
}
