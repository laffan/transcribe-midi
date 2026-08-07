import { useEffect, useMemo, useRef, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { AiProposal, Note, TimeSignature } from "../../lib/types";
import { drawProposal } from "./proposalDraw";
import { hitTest, midiOf, secondsOf, type Drag, type Scale } from "./transcribeGeometry";
import { barTicks } from "./timeFormat";
import { useAudition } from "./useAudition";
import "./TranscribeEditor.css";

interface ProposalEditorProps {
  proposal: AiProposal;
  /** The track's notes as they are now, so the diff can be drawn under the offer. */
  base: Note[];
  ppq: number;
  tempoBpm: number;
  timeSignature: TimeSignature;
  onChange: (proposal: AiProposal) => void;
  onApply: () => void;
  onDiscard: () => void;
  onPreviewNote: (pitch: number) => void;
}

/** How much room to leave past the last note, so its tail is not against the edge. */
const TAIL_SECONDS = 0.5;

/**
 * Reviewing notes the model wrote, with the same hands as a transcription.
 *
 * Before this, a described edit could only be *looked at*: the diff was drawn on the roll
 * and the only choices were Apply and Discard. That is a strange asymmetry in an app
 * where a sung line gets a full editor — the notes you did not play are exactly the ones
 * worth hearing before they land, and "nearly right" was a reason to throw the whole
 * thing away and prompt again.
 *
 * Adjustments go to Rust, which validates them and recomputes its own diff; what comes
 * back replaces the proposal. The interaction and the geometry are the transcription
 * editor's, minus the waveform — there is no recording behind a proposal, so the note
 * lane starts at the top.
 */
export function ProposalEditor({
  proposal,
  base,
  ppq,
  tempoBpm,
  timeSignature,
  onChange,
  onApply,
  onDiscard,
  onPreviewNote,
}: ProposalEditorProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 900, height: 420 });
  const [notes, setNotes] = useState<Note[]>(proposal.preview_notes);
  const [selected, setSelected] = useState<number | null>(null);
  const [drag, setDrag] = useState<Drag>({ type: "none" });
  const auditioned = useRef<number | null>(null);

  const { playhead, playFrom, stop, toggle } = useAudition("notes");

  const ticksPerSecond = (tempoBpm / 60) * ppq;

  useEffect(() => setNotes(proposal.preview_notes), [proposal]);

  const scale: Scale = useMemo(() => {
    const all = [...notes, ...base];
    const lastTick = all.reduce((end, n) => Math.max(end, n.start_ticks + n.duration_ticks), 0);
    const pitches = all.map((n) => n.pitch);
    const low = pitches.length ? Math.min(...pitches) - 2 : 48;
    const high = pitches.length ? Math.max(...pitches) + 2 : 72;

    return {
      ...size,
      // The whole proposal, always: there is no recording to zoom into here, and the
      // question being answered is "is this line right", which is a question about all
      // of it at once.
      startSeconds: 0,
      spanSeconds: Math.max(1, lastTick / ticksPerSecond + TAIL_SECONDS),
      ticksPerSecond,
      low,
      high: Math.max(high, low + 12),
      // No waveform to sit under.
      laneTop: 0,
    };
  }, [size, notes, base, ticksPerSecond]);

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

  useEffect(() => stop, [stop]);

  // -- shortcuts -----------------------------------------------------------

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

    drawProposal(ctx, {
      scale,
      notes,
      base,
      selected,
      playhead,
      barSeconds: barTicks(ppq, timeSignature) / ticksPerSecond,
    });
  }, [scale, size, notes, base, selected, playhead, ppq, timeSignature, ticksPerSecond]);

  // -- pointer -------------------------------------------------------------

  function localPoint(event: React.PointerEvent) {
    const rect = canvasRef.current!.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  function onPointerDown(event: React.PointerEvent<HTMLCanvasElement>) {
    const { x, y } = localPoint(event);
    const hit = hitTest(scale, notes, x, y);

    if (hit.type === "none") {
      // Empty space is the timeline: play from where you pointed, which is how you check
      // one bar of a long suggestion without listening to all of it.
      setSelected(null);
      void playFrom(secondsOf(scale, x));
      return;
    }

    canvasRef.current?.setPointerCapture(event.pointerId);
    setDrag(hit);
    setSelected(hit.index);
    const pitch = notes[hit.index]!.pitch;
    auditioned.current = pitch;
    onPreviewNote(pitch);
  }

  function onPointerMove(event: React.PointerEvent<HTMLCanvasElement>) {
    if (drag.type === "none") return;
    const { x, y } = localPoint(event);

    setNotes((current) => {
      const next = [...current];
      const note = { ...next[drag.index]! };

      if (drag.type === "move") {
        const start = secondsOf(scale, x) - drag.grabSeconds;
        note.start_ticks = Math.max(0, Math.round(start * ticksPerSecond));
        const delta = Math.round(midiOf(scale, y) - midiOf(scale, drag.startY));
        note.pitch = Math.min(127, Math.max(0, drag.startPitch + delta));
        if (note.pitch !== auditioned.current) {
          auditioned.current = note.pitch;
          onPreviewNote(note.pitch);
        }
      } else if (drag.type === "left") {
        const end = note.start_ticks + note.duration_ticks;
        const start = Math.round(secondsOf(scale, x) * ticksPerSecond);
        note.start_ticks = Math.max(0, Math.min(start, end - 1));
        note.duration_ticks = Math.max(1, end - note.start_ticks);
      } else {
        const end = Math.round(secondsOf(scale, x) * ticksPerSecond);
        note.duration_ticks = Math.max(1, end - note.start_ticks);
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

  /** Hand the adjusted notes to Rust and adopt the diff it derives from them. */
  async function commit(next: Note[]) {
    try {
      const edit = await api.aiSetNotes(next);
      setNotes(edit.preview_notes);
      onChange({ ...proposal, ...edit });
    } catch (e) {
      logger.error("Could not adjust the notes", errorMessage(e));
    }
  }

  function deleteSelected() {
    if (selected === null) return;
    const next = notes.filter((_, index) => index !== selected);
    setSelected(null);
    void commit(next);
  }

  const outOfRange = notes.length === 0;

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
      </div>

      <footer className="listen__actions tedit__controls">
        <div className="tedit__row">
          <button className="btn btn--lg" onClick={toggle} disabled={outOfRange}>
            {playhead === null ? "▶ Play" : "⏹ Stop"}
          </button>

          <span className="proposal__legend">
            <span className="proposal__swatch proposal__swatch--offer" /> on offer
            {base.length > 0 && (
              <>
                <span className="proposal__swatch proposal__swatch--base" /> what is there now
              </>
            )}
          </span>

          <span className="field__hint">
            Space plays; click the background to play from there. Drag a note to hear and
            move it, its edges to change length, ⌫ to remove it.
          </span>
        </div>

        <div className="tedit__row">
          {proposal.narration && <p className="proposal__narration">{proposal.narration}</p>}
          <div className="spacer" />
          <span className="review__usage mono">
            {proposal.usage.input_tokens.toLocaleString()} in ·{" "}
            {proposal.usage.output_tokens.toLocaleString()} out
          </span>
          <button className="btn" onClick={onDiscard}>
            Discard
          </button>
          <button className="btn btn--primary btn--lg" onClick={onApply} disabled={proposal.empty}>
            Apply
          </button>
        </div>
      </footer>
    </>
  );
}
