import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { Note, TranscribeTuning, TranscriptionPreview } from "../../lib/types";
import { noteName } from "./timeFormat";
import { TranscribeControls, GRIDS } from "./TranscribeControls";
import { drawTranscription } from "./transcribeDraw";
import { useTranscribeWheel } from "./transcribeGestures";
import {
  fitWindow,
  hitTest,
  isNavigating,
  loopEdgeAt,
  measuredCents,
  midiOf,
  normalizeLoop,
  pinchAxisOf,
  pinchWindow,
  secondsOf,
  semitonePx,
  snapSeconds,
  WAVE_HEIGHT,
  zoomTime,
  type Drag,
  type PinchAxis,
  type Scale,
  type Snap,
  type TouchPoint,
  type Window,
} from "./transcribeGeometry";
import { sameTuning, TuningDials } from "./TuningDials";
import { useTakeLoop } from "./useTakeLoop";
import { useTakeWaveform } from "./useTakeWaveform";
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

/** How far an arrow key moves a note along the take. */
const NUDGE_SECONDS = 0.01;

/**
 * How wide the grab band at a note's end, or at a loop's edge, is — for a pointer that
 * can be aimed, and for one that cannot. The grips are drawn at 7px either way; what
 * changes is how close you have to get, and a fingertip is about a centimetre across.
 */
const GRAB_PX = { fine: 8, coarse: 22 };

/** A press that never travels this far is a tap: it plays from there rather than looping. */
const LOOP_DRAG_PX = 6;

/**
 * The review stage: the take drawn as a waveform, the measured pitch traced over it, and
 * the detected notes on top as boxes you can drag.
 *
 * The point is to correct a note **against the evidence** rather than by ear against a
 * grid — and to hear the result, which is why the notes are what plays by default.
 *
 * Adjustments go back to Rust as notes, which validates them, rather than being applied
 * here. Drawing, geometry, the wheel and the controls live in siblings; what is left is
 * state and pointers.
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
  const [notes, setNotes] = useState<Note[]>(() => preview.notes.map((n) => n.note));
  const [selected, setSelected] = useState<number | null>(null);
  const [drag, setDrag] = useState<Drag>({ type: "none" });
  /** Where the dials are, which is not where the result on screen came from. */
  const [draft, setDraft] = useState<TranscribeTuning>(preview.tuning);
  /** The last pitch sounded by a drag, so a semitone step blips once and not per frame. */
  const auditioned = useRef<number | null>(null);

  const duration = Math.max(0.001, preview.duration_seconds);
  const { analysis } = preview;

  /** What is on screen. Everything else is drawn and hit-tested through it. */
  const [view, setView] = useState<Window>(() =>
    fitWindow(
      preview.notes.map((n) => n.note),
      analysis,
      duration,
    ),
  );
  const {
    source,
    changeSource,
    playhead,
    playing,
    loop,
    setLoop,
    region,
    setRegion,
    loopSpan,
    start,
    togglePlay,
  } = useTakeLoop({ preview, duration, view });

  const wave = useTakeWaveform(preview, duration, view, size.width);

  // Notes arrive in ticks; everything here is in seconds against the waveform, so the
  // conversion happens once and both directions use it.
  const ticksPerSecond = (preview.tempo_bpm / 60) * ppq;

  const scale: Scale = useMemo(
    () => ({ ...size, ...view, ticksPerSecond, laneTop: WAVE_HEIGHT }),
    [size, view, ticksPerSecond],
  );

  /**
   * What a dragged boundary lands on. One control decides it: "Snap to: Off" means
   * nothing snaps, including to the detected attacks — which is the only way to place a
   * boundary the analysis put in the wrong place.
   */
  const snap: Snap = useMemo(
    () => ({
      attacks: preview.quantize_ticks > 0,
      gridSeconds: preview.quantize_ticks > 0 ? preview.quantize_ticks / ticksPerSecond : 0,
    }),
    [preview.quantize_ticks, ticksPerSecond],
  );

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
    const fresh = preview.notes.map((n) => n.note);
    setNotes(fresh);
    setSelected(null);
    // A fresh analysis answers with the tuning it actually used, including any clamping.
    setDraft(preview.tuning);
    setView(fitWindow(fresh, preview.analysis, Math.max(0.001, preview.duration_seconds)));
    setRegion(null);
  }, [preview]);

  // -- navigation ----------------------------------------------------------

  const zoom = useCallback(
    (factor: number) =>
      setView((current) =>
        zoomTime(current, factor, current.startSeconds + current.spanSeconds / 2, duration),
      ),
    [duration],
  );

  const fit = useCallback(
    () => setView(fitWindow(notes, analysis, duration)),
    [notes, analysis, duration],
  );

  useTranscribeWheel({ canvasRef, scale, duration, setView });

  // -- editing -------------------------------------------------------------

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

  const deleteSelected = useCallback(() => {
    if (selected === null) return;
    const next = notes.filter((_, index) => index !== selected);
    setSelected(null);
    void commit(next);
  }, [selected, notes, commit]);

  /**
   * Merge the selected note into the one after it.
   *
   * The editor selects one note at a time, and the case this is for is always the same
   * pair: a held note the analysis broke in two, at a vibrato wobble or a slur it read as
   * an attack. Joining forwards covers it without a marquee, and repeating the key walks
   * along a note that came back in four pieces.
   */
  const joinWithNext = useCallback(() => {
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
  }, [selected, notes, commit]);

  /**
   * Move the selected note by whole semitones, or along the take, without a drag.
   *
   * A drag is a distance and this is a step, and the two are not interchangeable: a
   * semitone can be four pixels tall on a take that spans three octaves, and no amount
   * of steadiness turns a four-pixel drag into a reliable one. It is also the only way
   * to make the move at all with a finger, which is why the same two steps are buttons
   * in the bar as well as arrow keys.
   */
  const nudge = useCallback(
    (semitones: number, seconds: number) => {
      if (selected === null) return;
      const note = notes[selected]!;
      const pitch = Math.min(127, Math.max(0, note.pitch + semitones));
      const next = [...notes];
      next[selected] = {
        ...note,
        pitch,
        start_ticks: Math.max(0, note.start_ticks + Math.round(seconds * ticksPerSecond)),
      };
      setNotes(next);
      if (semitones !== 0) onPreviewNote(pitch);
      void commit(next);
    },
    [selected, notes, ticksPerSecond, onPreviewNote, commit],
  );

  // -- keyboard ------------------------------------------------------------

  // Held in a ref so the listener is bound once and still sees the current selection —
  // the same shape the editor's shortcuts use, for the same reason.
  const shortcuts = useRef<(event: KeyboardEvent) => void>(() => {});
  shortcuts.current = (event: KeyboardEvent) => {
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "SELECT")) return;
    if (event.metaKey || event.ctrlKey) return;

    const keys: Record<string, () => void> = {
      Space: togglePlay,
      Delete: deleteSelected,
      Backspace: deleteSelected,
      ArrowUp: () => nudge(event.shiftKey ? 12 : 1, 0),
      ArrowDown: () => nudge(event.shiftKey ? -12 : -1, 0),
      ArrowLeft: () => nudge(0, -NUDGE_SECONDS * (event.shiftKey ? 10 : 1)),
      ArrowRight: () => nudge(0, NUDGE_SECONDS * (event.shiftKey ? 10 : 1)),
    };
    const byCode = keys[event.code] ?? keys[event.key];
    if (byCode) {
      event.preventDefault();
      byCode();
      return;
    }

    switch (event.key.toLowerCase()) {
      case "j":
        event.preventDefault();
        joinWithNext();
        break;
      case "l":
        event.preventDefault();
        setLoop((v) => !v);
        break;
      case "=":
      case "+":
        event.preventDefault();
        zoom(1.6);
        break;
      case "-":
        event.preventDefault();
        zoom(1 / 1.6);
        break;
      case "0":
        event.preventDefault();
        fit();
        break;
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

    drawTranscription(ctx, {
      scale,
      peaks: wave.peaks,
      waveGain: wave.gain,
      waveNote: wave.note,
      notes,
      selected,
      analysis,
      playhead,
      loop: loop ? loopSpan : null,
    });
  }, [scale, size, wave, notes, selected, analysis, playhead, loop, loopSpan]);

  // -- pointers ------------------------------------------------------------

  /** Live touches, by id. Two of them navigate; one edits. */
  const pointers = useRef(new Map<number, TouchPoint>());
  const pinch = useRef<{
    from: readonly [TouchPoint, TouchPoint];
    view: Window;
    scale: Scale;
    axis: PinchAxis;
  } | null>(null);
  /**
   * A press in the waveform lane, before it is known whether it is a tap or a drag.
   *
   * The lane has to carry both: pressing it to play from a moment is the fastest way to
   * check one, and dragging across it is how a loop gets its ends. Nothing is decided
   * until the pointer has travelled — a tap plays, a drag loops — so neither gesture has
   * to be spelled with a modifier a touchscreen does not have.
   */
  const laneDrag = useRef<{ anchorSeconds: number; x: number; edge: "start" | "end" | null } | null>(
    null,
  );

  function localPoint(event: React.PointerEvent) {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function beginPinch() {
    const [a, b] = [...pointers.current.values()];
    if (!a || !b) return;
    pinch.current = { from: [a, b], view, scale, axis: pinchAxisOf(a, b) };
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const point = localPoint(event);
    pointers.current.set(event.pointerId, point);
    canvasRef.current?.setPointerCapture(event.pointerId);

    if (isNavigating(pointers.current.size)) {
      // A second finger turns whatever was happening into navigation. Anything the first
      // one had already moved is committed rather than abandoned, so Rust and the screen
      // do not disagree about where the note is.
      if (drag.type !== "none") {
        setDrag({ type: "none" });
        void commit(notes);
      }
      beginPinch();
      return;
    }

    const grab = event.pointerType === "mouse" ? GRAB_PX.fine : GRAB_PX.coarse;

    // The waveform lane is for listening and for saying which part of the take to hear
    // again; the pitch lane is for editing.
    if (point.y < WAVE_HEIGHT) {
      laneDrag.current = {
        anchorSeconds: secondsOf(scale, point.x),
        x: point.x,
        edge: loopEdgeAt(scale, region, point.x, grab),
      };
      return;
    }

    const hit = hitTest(scale, notes, point.x, point.y, grab);
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
    const point = localPoint(event);
    if (pointers.current.has(event.pointerId)) pointers.current.set(event.pointerId, point);

    const gesture = pinch.current;
    if (gesture && pointers.current.size >= 2) {
      const [a, b] = [...pointers.current.values()];
      if (!a || !b) return;
      setView(
        pinchWindow({
          from: gesture.from,
          to: [a, b],
          view: gesture.view,
          scale: gesture.scale,
          axis: gesture.axis,
          duration,
        }),
      );
      return;
    }

    // Dragging in the waveform lane: an edge of the loop if one was grabbed, otherwise
    // a new loop out of the press point.
    const lane = laneDrag.current;
    if (lane) {
      const at = secondsOf(scale, point.x);
      if (lane.edge) {
        const other = lane.edge === "start" ? loopSpan[1] : loopSpan[0];
        setRegion(normalizeLoop(at, other, duration));
        setLoop(true);
      } else if (Math.abs(point.x - lane.x) > LOOP_DRAG_PX) {
        setRegion(normalizeLoop(lane.anchorSeconds, at, duration));
        setLoop(true);
      }
      return;
    }

    if (drag.type === "none") return;

    setNotes((current) => {
      const next = [...current];
      const note = { ...next[drag.index]! };

      if (drag.type === "move") {
        const startAt = snapSeconds(analysis, secondsOf(scale, point.x) - drag.grabSeconds, snap);
        note.start_ticks = Math.max(0, Math.round(startAt * ticksPerSecond));
        // Semitone steps: the pitch line shows where the source actually sat, and a note
        // is a note. Fractions belong on the line, not in the MIDI.
        const delta = Math.round(midiOf(scale, point.y) - midiOf(scale, drag.startY));
        note.pitch = Math.min(127, Math.max(0, drag.startPitch + delta));
      } else if (drag.type === "left") {
        const end = note.start_ticks + note.duration_ticks;
        const at = Math.round(snapSeconds(analysis, secondsOf(scale, point.x), snap) * ticksPerSecond);
        note.start_ticks = Math.max(0, Math.min(at, end - 1));
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      } else {
        const end = Math.round(snapSeconds(analysis, secondsOf(scale, point.x), snap) * ticksPerSecond);
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

  function onPointerUp(event: React.PointerEvent<HTMLCanvasElement>) {
    pointers.current.delete(event.pointerId);
    if (pointers.current.size < 2) pinch.current = null;

    const lane = laneDrag.current;
    laneDrag.current = null;
    if (lane) {
      // It never travelled, so it was a press on a moment rather than a drag across a
      // stretch: play from there.
      const { x } = localPoint(event);
      if (!lane.edge && Math.abs(x - lane.x) <= LOOP_DRAG_PX) start(lane.anchorSeconds);
      return;
    }

    if (drag.type === "none") return;
    setDrag({ type: "none" });
    auditioned.current = null;
    void commit(notes);
  }

  // -- render --------------------------------------------------------------

  const gridDivisor =
    preview.quantize_ticks === 0
      ? 0
      : (GRIDS.find((g) => Math.round(ppq / g.divisor) === preview.quantize_ticks)?.divisor ?? 0);

  const selectedNote = selected === null ? null : (notes[selected] ?? null);

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

        {/* How close in you are, which the ruler alone does not say once the window is
            shorter than a second. */}
        {/* How close in you are, and — when the take was quiet enough to need it — how
            much the waveform has been amplified to be worth looking at. Said out loud,
            because a normalised overview of a whisper otherwise looks like a shout. */}
        <span className="tedit__readout mono">
          {view.spanSeconds < duration - 0.01
            ? `${view.spanSeconds.toFixed(2)}s across · ${semitonePx(scale).toFixed(0)}px a semitone`
            : `${duration.toFixed(1)}s take`}
          {wave.gain > 1.5 && ` · waveform ×${wave.gain.toFixed(0)}`}
        </span>
      </div>

      <footer className="listen__actions tedit__controls">
        <TranscribeControls
          playing={playing}
          onTogglePlay={togglePlay}
          source={source}
          onSourceChange={changeSource}
          loop={loop}
          onLoopChange={setLoop}
          loopRegion={region}
          onClearLoop={() => setRegion(null)}
          zoomed={view.spanSeconds < duration - 0.01}
          onZoom={zoom}
          onFit={fit}
          selected={
            selectedNote && {
              name: noteName(selectedNote.pitch),
              cents: measuredCents(analysis, selectedNote, ticksPerSecond),
            }
          }
          onNudgePitch={(semitones) => nudge(semitones, 0)}
          gridDivisor={gridDivisor}
          onGridChange={(divisor) =>
            onReprocess(
              preview.use_project_tempo,
              divisor === 0 ? 0 : Math.round(ppq / divisor),
              draft,
            )
          }
          useProjectTempo={preview.use_project_tempo}
          onProjectTempoChange={(value) => onReprocess(value, preview.quantize_ticks, draft)}
          canJoin={selected !== null && selected + 1 < notes.length}
          onJoin={joinWithNext}
          onDelete={deleteSelected}
          onDiscard={onDiscard}
          onApply={onApply}
          canApply={notes.length > 0}
        />

        <TuningDials draft={draft} applied={preview.tuning} disabled={false} onChange={setDraft} />
      </footer>
    </>
  );
}
