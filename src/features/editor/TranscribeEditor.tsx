import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { Note, TranscriptionPreview, WaveformPeaks } from "../../lib/types";
import "./TranscribeEditor.css";

interface TranscribeEditorProps {
  preview: TranscriptionPreview;
  ppq: number;
  onChange: (preview: TranscriptionPreview) => void;
  onApply: () => void;
  onClose: () => void;
}

/** Height of the waveform lane, in CSS pixels. */
const WAVE_HEIGHT = 96;
/** Vertical padding inside the pitch lane. */
const PITCH_PAD = 12;
/** Grab width, in pixels, of a note's resize edge. */
const EDGE_PX = 6;

const GRIDS: { label: string; divisor: number }[] = [
  { label: "Off", divisor: 0 },
  { label: "1/4", divisor: 1 },
  { label: "1/8", divisor: 2 },
  { label: "1/16", divisor: 4 },
  { label: "1/8 triplet", divisor: 3 },
];

type Drag =
  | { type: "none" }
  | { type: "move"; index: number; grabSeconds: number; startPitch: number; startY: number }
  | { type: "left"; index: number }
  | { type: "right"; index: number };

/**
 * The transcription editor: the take drawn as a waveform, the measured pitch traced over
 * it, and the detected notes on top as boxes you can drag.
 *
 * The point is to correct a note **against the evidence** rather than by ear against a
 * grid. Everything drawn here was already computed by the transcriber and, until Phase 9,
 * thrown away: the pitch line is the per-frame YIN estimate, the tick marks are the
 * spectral-flux onsets a boundary snaps to, and a note drawn off its own pitch line is
 * one the analysis was unsure about — which is exactly the note worth checking.
 *
 * Adjustments go back to Rust as notes, which validates them, rather than being applied
 * here.
 */
export function TranscribeEditor({
  preview,
  ppq,
  onChange,
  onApply,
  onClose,
}: TranscribeEditorProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 900, height: 420 });
  const [peaks, setPeaks] = useState<WaveformPeaks>([]);
  const [notes, setNotes] = useState<Note[]>(() => preview.notes.map((n) => n.note));
  const [selected, setSelected] = useState<number | null>(null);
  const [drag, setDrag] = useState<Drag>({ type: "none" });
  const [busy, setBusy] = useState(false);

  const duration = Math.max(0.001, preview.duration_seconds);
  const { analysis } = preview;

  // Notes arrive in ticks; everything here is in seconds against the waveform, so the
  // conversion happens once and both directions use it.
  const ticksPerSecond = (preview.tempo_bpm / 60) * ppq;

  // -- sizing --------------------------------------------------------------

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) {
        setSize({ width: Math.max(320, rect.width), height: Math.max(240, rect.height) });
      }
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    setNotes(preview.notes.map((n) => n.note));
    setSelected(null);
  }, [preview]);

  // -- waveform ------------------------------------------------------------

  useEffect(() => {
    // One bucket per pixel. Asking Rust rather than shipping the samples: two minutes at
    // 48 kHz is over twenty megabytes for a few hundred columns.
    api
      .captureWaveform(0, duration, Math.round(size.width))
      .then(setPeaks)
      .catch(() => setPeaks([]));
  }, [duration, size.width]);

  // -- geometry ------------------------------------------------------------

  const pitchRange = useMemo(() => {
    const pitches = notes.map((n) => n.pitch);
    const measured = analysis.frames.filter((f) => f.midi > 0).map((f) => f.midi);
    const all = [...pitches, ...measured];
    if (all.length === 0) return { low: 48, high: 72 };

    // A little air either side, and never so tight that a note dragged a semitone leaves
    // the view.
    const low = Math.floor(Math.min(...all)) - 2;
    const high = Math.ceil(Math.max(...all)) + 2;
    return { low, high: Math.max(high, low + 12) };
  }, [notes, analysis.frames]);

  const pitchTop = WAVE_HEIGHT;
  const pitchHeight = Math.max(80, size.height - WAVE_HEIGHT);

  const xOf = useCallback(
    (seconds: number) => (seconds / duration) * size.width,
    [duration, size.width],
  );
  const secondsOf = useCallback(
    (x: number) => (x / Math.max(1, size.width)) * duration,
    [duration, size.width],
  );
  const yOf = useCallback(
    (midi: number) => {
      const span = pitchRange.high - pitchRange.low;
      const t = (midi - pitchRange.low) / span;
      return pitchTop + pitchHeight - PITCH_PAD - t * (pitchHeight - PITCH_PAD * 2);
    },
    [pitchRange, pitchTop, pitchHeight],
  );
  const midiOf = useCallback(
    (y: number) => {
      const span = pitchRange.high - pitchRange.low;
      const t = (pitchTop + pitchHeight - PITCH_PAD - y) / (pitchHeight - PITCH_PAD * 2);
      return pitchRange.low + t * span;
    },
    [pitchRange, pitchTop, pitchHeight],
  );

  const noteRect = useCallback(
    (note: Note) => {
      const start = note.start_ticks / ticksPerSecond;
      const end = (note.start_ticks + note.duration_ticks) / ticksPerSecond;
      const y = yOf(note.pitch);
      return { x: xOf(start), width: Math.max(3, xOf(end) - xOf(start)), y: y - 7, height: 14 };
    },
    [ticksPerSecond, xOf, yOf],
  );

  // -- drawing -------------------------------------------------------------

  useEffect(() => {
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
    const border = token("--border", "#2b303b");
    const text2 = token("--text-2", "#6f7689");
    const accent = token("--accent", "#5b8dd9");
    const added = token("--diff-added", "#6cb08a");
    const changed = token("--diff-changed", "#d9a441");

    ctx.clearRect(0, 0, size.width, size.height);

    // ---- waveform ----
    ctx.fillStyle = bgInset;
    ctx.fillRect(0, 0, size.width, WAVE_HEIGHT);

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
    ctx.lineTo(size.width, WAVE_HEIGHT + 0.5);
    ctx.stroke();

    // ---- onsets ----
    // Drawn through both lanes: they are where a boundary snaps, so they need to be
    // visible against the waveform *and* against the notes.
    ctx.strokeStyle = changed;
    ctx.globalAlpha = 0.45;
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    for (const frame of analysis.onsets) {
      const x = Math.round(xOf(frame * analysis.hop_seconds)) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, size.height);
    }
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.globalAlpha = 1;

    // ---- semitone rules ----
    ctx.strokeStyle = border;
    ctx.globalAlpha = 0.5;
    ctx.font = "9px ui-monospace, monospace";
    for (let midi = Math.ceil(pitchRange.low); midi <= pitchRange.high; midi += 1) {
      if (midi % 12 !== 0) continue;
      const y = Math.round(yOf(midi)) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(size.width, y);
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
    analysis.frames.forEach((frame, index) => {
      const voiced = frame.midi > 0 && frame.level > analysis.silence_floor;
      if (!voiced) {
        if (open) {
          ctx.stroke();
          open = false;
        }
        return;
      }
      const x = xOf(index * analysis.hop_seconds);
      const y = yOf(frame.midi);
      if (!open) {
        ctx.beginPath();
        ctx.strokeStyle = accent;
        ctx.globalAlpha = 0.35 + Math.min(1, frame.confidence) * 0.5;
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
      const rect = noteRect(note);
      const isSelected = selected === index;

      ctx.globalAlpha = isSelected ? 1 : 0.8;
      ctx.fillStyle = added;
      ctx.fillRect(rect.x, rect.y, rect.width, rect.height);

      if (isSelected) {
        ctx.strokeStyle = token("--key-white", "#e8eaf0");
        ctx.strokeRect(rect.x + 0.5, rect.y + 0.5, rect.width - 1, rect.height - 1);
      }
      ctx.globalAlpha = 1;
    });
  }, [size, peaks, notes, selected, analysis, pitchRange, xOf, yOf, noteRect]);

  // -- pointer -------------------------------------------------------------

  function localPoint(event: React.PointerEvent) {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function hitTest(x: number, y: number): Drag {
    for (let index = notes.length - 1; index >= 0; index -= 1) {
      const rect = noteRect(notes[index]!);
      if (x < rect.x - EDGE_PX || x > rect.x + rect.width + EDGE_PX) continue;
      if (y < rect.y - 4 || y > rect.y + rect.height + 4) continue;

      if (x <= rect.x + EDGE_PX) return { type: "left", index };
      if (x >= rect.x + rect.width - EDGE_PX) return { type: "right", index };
      return {
        type: "move",
        index,
        grabSeconds: secondsOf(x) - notes[index]!.start_ticks / ticksPerSecond,
        startPitch: notes[index]!.pitch,
        startY: y,
      };
    }
    return { type: "none" };
  }

  /** Nearest onset within a small window, so a dragged edge lands on the attack. */
  function snapSeconds(seconds: number): number {
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

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const { x, y } = localPoint(event);
    const hit = hitTest(x, y);
    canvasRef.current?.setPointerCapture(event.pointerId);
    setDrag(hit);
    setSelected(hit.type === "none" ? null : hit.index);
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    if (drag.type === "none") return;
    const { x, y } = localPoint(event);

    setNotes((current) => {
      const next = [...current];
      const note = { ...next[drag.index]! };

      if (drag.type === "move") {
        const start = snapSeconds(secondsOf(x) - drag.grabSeconds);
        note.start_ticks = Math.max(0, Math.round(start * ticksPerSecond));
        // Semitone steps: the pitch line shows where the source actually sat, and a note
        // is a note. Fractions belong on the line, not in the MIDI.
        const delta = Math.round(midiOf(y) - midiOf(drag.startY));
        note.pitch = Math.min(127, Math.max(0, drag.startPitch + delta));
      } else if (drag.type === "left") {
        const end = note.start_ticks + note.duration_ticks;
        const start = Math.round(snapSeconds(secondsOf(x)) * ticksPerSecond);
        note.start_ticks = Math.max(0, Math.min(start, end - 1));
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      } else {
        const end = Math.round(snapSeconds(secondsOf(x)) * ticksPerSecond);
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      }

      next[drag.index] = note;
      return next;
    });
  }

  function onPointerUp() {
    if (drag.type === "none") return;
    setDrag({ type: "none" });
    void commit();
  }

  /** Send the adjusted notes to Rust, which validates them. */
  const commit = useCallback(async () => {
    try {
      await api.captureSetNotes(notes);
    } catch (e) {
      logger.error("Could not adjust the notes", errorMessage(e));
    }
  }, [notes]);

  async function rederive(useProjectTempo: boolean, quantizeTicks: number) {
    setBusy(true);
    try {
      // The take is still here, so this re-reads it rather than asking for another
      // performance — which is the whole reason the audio is retained.
      onChange(await api.captureRetranscribe(useProjectTempo, quantizeTicks));
    } catch (e) {
      logger.error("Could not re-read the take", errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function deleteSelected() {
    if (selected === null) return;
    setNotes((current) => current.filter((_, index) => index !== selected));
    setSelected(null);
    void commit();
  }

  const gridDivisor =
    preview.quantize_ticks === 0
      ? 0
      : (GRIDS.find((g) => Math.round(ppq / g.divisor) === preview.quantize_ticks)?.divisor ?? 0);

  return (
    <div className="tedit" role="dialog" aria-label="Transcription editor">
      <header className="tedit__head">
        <h2 className="tedit__title">Fine-tune transcription</h2>
        <span className="tedit__stat mono">
          {notes.length} notes · {duration.toFixed(1)}s ·{" "}
          {preview.tempo_bpm.toFixed(0)} bpm
          {preview.tempo_estimated ? " (estimated)" : ""}
        </span>
        <div className="spacer" />
        <button className="btn btn--ghost btn--icon" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </header>

      <div className="tedit__canvas" ref={containerRef}>
        <canvas
          ref={canvasRef}
          style={{ width: size.width, height: size.height }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />
      </div>

      <div className="tedit__controls">
        <label className="tedit__control">
          <span className="field__label">Snap to</span>
          <select
            className="input"
            value={gridDivisor}
            disabled={busy}
            onChange={(e) => {
              const divisor = Number(e.target.value);
              void rederive(preview.use_project_tempo, divisor === 0 ? 0 : Math.round(ppq / divisor));
            }}
          >
            {GRIDS.map((grid) => (
              <option key={grid.label} value={grid.divisor}>
                {grid.label}
              </option>
            ))}
          </select>
        </label>

        <label className="tedit__check">
          <input
            type="checkbox"
            checked={preview.use_project_tempo}
            disabled={busy}
            onChange={(e) => void rederive(e.target.checked, preview.quantize_ticks)}
          />
          <span>Use the project tempo</span>
        </label>

        <span className="field__hint">
          Drag a note to move it, its edges to change length. Edges snap to the detected
          attacks. The line is the pitch that was actually measured.
        </span>

        <div className="spacer" />

        <button className="btn" onClick={deleteSelected} disabled={selected === null}>
          Delete note
        </button>
        <button className="btn btn--primary" onClick={onApply} disabled={notes.length === 0}>
          Add to track
        </button>
      </div>
    </div>
  );
}
