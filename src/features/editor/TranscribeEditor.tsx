import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type {
  AuditionSource,
  Note,
  TranscribeTuning,
  TranscriptionPreview,
  WaveformPeaks,
} from "../../lib/types";
import { drawTranscription } from "./transcribeDraw";
import {
  hitTest,
  midiOf,
  pitchRangeOf,
  secondsOf,
  snapSeconds,
  WAVE_HEIGHT,
  type Drag,
  type Scale,
} from "./transcribeGeometry";
import { sameTuning, TuningDials } from "./TuningDials";
import { useAudition } from "./useAudition";
import "./TranscribeEditor.css";

interface TranscribeEditorProps {
  preview: TranscriptionPreview;
  ppq: number;
  onApply: () => void;
  onDiscard: () => void;
  /** Sound a pitch briefly. What makes dragging a note something you can do by ear. */
  onPreviewNote: (pitch: number) => void;
  /**
   * Read the take again. Owned by the parent because it swaps this view for the progress
   * stage while it runs — seconds of analysis behind a frozen picture of the old result
   * is the thing this replaced.
   */
  onReprocess: (useProjectTempo: boolean, quantizeTicks: number, tuning: TranscribeTuning) => void;
}

const GRIDS: { label: string; divisor: number }[] = [
  { label: "Off", divisor: 0 },
  { label: "1/4", divisor: 1 },
  { label: "1/8", divisor: 2 },
  { label: "1/16", divisor: 4 },
  { label: "1/8 triplet", divisor: 3 },
];

const SOURCES: { value: AuditionSource; label: string; title: string }[] = [
  { value: "midi", label: "Notes", title: "Play the transcription on the sampler" },
  { value: "take", label: "Recording", title: "Play the take you performed" },
  { value: "both", label: "Both", title: "Play them together and compare" },
];

/**
 * The review stage: the take drawn as a waveform, the measured pitch traced over it, and
 * the detected notes on top as boxes you can drag.
 *
 * The point is to correct a note **against the evidence** rather than by ear against a
 * grid — and to hear the result, which is why the notes are what plays by default.
 *
 * Adjustments go back to Rust as notes, which validates them, rather than being applied
 * here. Drawing and the geometry live in siblings; what is left is state and pointers.
 */
export function TranscribeEditor({
  preview,
  ppq,
  onApply,
  onDiscard,
  onPreviewNote,
  onReprocess,
}: TranscribeEditorProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 900, height: 420 });
  const [peaks, setPeaks] = useState<WaveformPeaks>([]);
  const [notes, setNotes] = useState<Note[]>(() => preview.notes.map((n) => n.note));
  const [selected, setSelected] = useState<number | null>(null);
  const [drag, setDrag] = useState<Drag>({ type: "none" });
  /** Where the dials are, which is not where the result on screen came from. */
  const [draft, setDraft] = useState<TranscribeTuning>(preview.tuning);
  /** The last pitch sounded by a drag, so a semitone step blips once and not per frame. */
  const auditioned = useRef<number | null>(null);

  const { source, changeSource, playhead, playFrom, stop, toggle } = useAudition();

  const duration = Math.max(0.001, preview.duration_seconds);
  const { analysis } = preview;

  // Notes arrive in ticks; everything here is in seconds against the waveform, so the
  // conversion happens once and both directions use it.
  const ticksPerSecond = (preview.tempo_bpm / 60) * ppq;

  const scale: Scale = useMemo(() => {
    const range = pitchRangeOf(notes, analysis);
    return {
      ...size,
      duration,
      ticksPerSecond,
      low: range.low,
      high: range.high,
      laneTop: WAVE_HEIGHT,
    };
  }, [size, duration, ticksPerSecond, notes, analysis]);

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
    // A fresh analysis answers with the tuning it actually used, including any clamping.
    setDraft(preview.tuning);
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

  // Re-deriving replaces the notes under the playhead, so nothing may still be sounding.
  useEffect(() => {
    stop();
  }, [preview, stop]);

  // -- playback ------------------------------------------------------------

  // Held in a ref so the listener is bound once and still sees the current selection —
  // the same shape the editor's shortcuts use, for the same reason.
  const shortcuts = useRef<(event: KeyboardEvent) => void>(() => {});
  shortcuts.current = (event: KeyboardEvent) => {
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "SELECT")) return;

    if (event.code === "Space") {
      event.preventDefault();
      toggle();
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      deleteSelected();
      return;
    }
    if (!event.metaKey && !event.ctrlKey && event.key.toLowerCase() === "j") {
      event.preventDefault();
      joinWithNext();
    }
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => shortcuts.current(event);
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

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

    drawTranscription(ctx, { scale, peaks, notes, selected, analysis, playhead });
  }, [scale, size, peaks, notes, selected, analysis, playhead]);

  // -- pointer -------------------------------------------------------------

  function localPoint(event: React.PointerEvent) {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const { x, y } = localPoint(event);

    // The waveform lane is for listening, the pitch lane is for editing. Clicking the
    // waveform plays from there, which is the fastest way to check a particular moment.
    if (y < WAVE_HEIGHT) {
      void playFrom(secondsOf(scale, x));
      return;
    }

    const hit = hitTest(scale, notes, x, y);
    canvasRef.current?.setPointerCapture(event.pointerId);
    setDrag(hit);
    setSelected(hit.type === "none" ? null : hit.index);

    // Sound what was grabbed. Correcting a transcription is an ear job, and the note
    // under the cursor is the one being judged.
    if (hit.type !== "none") {
      const pitch = notes[hit.index]!.pitch;
      auditioned.current = pitch;
      onPreviewNote(pitch);
    }
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    if (drag.type === "none") return;
    const { x, y } = localPoint(event);

    setNotes((current) => {
      const next = [...current];
      const note = { ...next[drag.index]! };

      if (drag.type === "move") {
        const start = snapSeconds(analysis, secondsOf(scale, x) - drag.grabSeconds);
        note.start_ticks = Math.max(0, Math.round(start * ticksPerSecond));
        // Semitone steps: the pitch line shows where the source actually sat, and a note
        // is a note. Fractions belong on the line, not in the MIDI.
        const delta = Math.round(midiOf(scale, y) - midiOf(scale, drag.startY));
        note.pitch = Math.min(127, Math.max(0, drag.startPitch + delta));
      } else if (drag.type === "left") {
        const end = note.start_ticks + note.duration_ticks;
        const start = Math.round(snapSeconds(analysis, secondsOf(scale, x)) * ticksPerSecond);
        note.start_ticks = Math.max(0, Math.min(start, end - 1));
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      } else {
        const end = Math.round(snapSeconds(analysis, secondsOf(scale, x)) * ticksPerSecond);
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      }

      // Dragging by ear: a blip on each semitone crossed, not on each pointer frame.
      if (drag.type === "move" && note.pitch !== auditioned.current) {
        auditioned.current = note.pitch;
        onPreviewNote(note.pitch);
      }

      next[drag.index] = note;
      return next;
    });
  }

  function onPointerUp() {
    if (drag.type === "none") return;
    setDrag({ type: "none" });
    auditioned.current = null;
    void commit(notes);
  }

  /**
   * Send the adjusted notes to Rust and take back what it kept.
   *
   * Rust validates them and flattens any overlap a drag created, so what comes back is
   * not always what went out. Adopting the answer is what keeps the next drag working
   * against notes that actually exist.
   */
  const commit = useCallback(async (next: Note[]) => {
    try {
      setNotes(await api.captureSetNotes(next));
    } catch (e) {
      logger.error("Could not adjust the notes", errorMessage(e));
    }
  }, []);

  function deleteSelected() {
    if (selected === null) return;
    const next = notes.filter((_, index) => index !== selected);
    setSelected(null);
    void commit(next);
  }

  /**
   * Merge the selected note into the one after it.
   *
   * The editor selects one note at a time, and the case this is for is always the same
   * pair: a held note the analysis broke in two, at a vibrato wobble or a slur it read as
   * an attack. Joining forwards covers it without a marquee, and repeating the key walks
   * along a note that came back in four pieces.
   */
  function joinWithNext() {
    if (selected === null || selected + 1 >= notes.length) return;
    const first = notes[selected]!;
    const second = notes[selected + 1]!;
    const end = second.start_ticks + second.duration_ticks;

    const next = notes.filter((_, index) => index !== selected + 1);
    next[selected] = {
      ...first,
      duration_ticks: Math.max(1, end - first.start_ticks),
    };
    void commit(next);
  }

  const gridDivisor =
    preview.quantize_ticks === 0
      ? 0
      : (GRIDS.find((g) => Math.round(ppq / g.divisor) === preview.quantize_ticks)?.divisor ?? 0);

  return (
    <>
      <div className="listen__body tedit__canvas" ref={containerRef}>
        <canvas
          ref={canvasRef}
          style={{ width: size.width, height: size.height }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
        />

        {/* Over the result rather than beside the dials: what it acts on is what you are
            looking at, and the answer to "why has nothing changed?" should be in view. */}
        {!sameTuning(draft, preview.tuning) && (
          <button
            className="btn btn--primary btn--lg tedit__reprocess"
            onClick={() => onReprocess(preview.use_project_tempo, preview.quantize_ticks, draft)}
          >
            ↻ Re-process with these settings
          </button>
        )}
      </div>

      <footer className="listen__actions tedit__controls">
        <div className="tedit__row">
          <button className="btn btn--lg" onClick={toggle}>
            {playhead === null ? "▶ Play" : "⏹ Stop"}
          </button>

          <div className="segmented" role="group" aria-label="What to play">
            {SOURCES.map((option) => (
              <button
                key={option.value}
                className={`segmented__option ${source === option.value ? "segmented__option--on" : ""}`}
                onClick={() => changeSource(option.value)}
                aria-pressed={source === option.value}
                title={option.title}
              >
                {option.label}
              </button>
            ))}
          </div>

          <span className="field__hint">
            Space plays. Click the waveform to start from there. Drag a note to hear and
            move it, its edges to change length — edges snap to the detected attacks. The
            line is the pitch that was actually measured.
          </span>
        </div>

        <div className="tedit__row">
          <label className="tedit__control">
            <span className="field__label">Snap to</span>
            <select
              className="input"
              value={gridDivisor}
              
              onChange={(e) => {
                const divisor = Number(e.target.value);
                onReprocess(
                  preview.use_project_tempo,
                  divisor === 0 ? 0 : Math.round(ppq / divisor),
                  draft,
                );
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
              
              onChange={(e) => onReprocess(e.target.checked, preview.quantize_ticks, draft)}
            />
            <span>Use the project tempo</span>
          </label>

          <button
            className="btn"
            onClick={joinWithNext}
            disabled={selected === null || selected + 1 >= notes.length}
            title="Join this note to the one after it (J)"
          >
            Join
          </button>
          <button
            className="btn"
            onClick={deleteSelected}
            disabled={selected === null}
            title="Delete the selected note (⌫)"
          >
            Delete note
          </button>

          <div className="spacer" />

          <button className="btn" onClick={onDiscard}>
            Discard take
          </button>
          <button
            className="btn btn--primary btn--lg"
            onClick={onApply}
            disabled={notes.length === 0}
          >
            Add to track
          </button>
        </div>

        <TuningDials
          draft={draft}
          applied={preview.tuning}
          disabled={false}
          onChange={setDraft}
        />
      </footer>
    </>
  );
}
