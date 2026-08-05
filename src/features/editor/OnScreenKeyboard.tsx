import { useCallback, useEffect, useRef, useState } from "react";

import { PHONE, useMediaQuery } from "../../lib/useMediaQuery";
import { isBlackKey, pitchName } from "./pianoRollGeometry";
import "./OnScreenKeyboard.css";

/**
 * Computer-keyboard mapping, Logic Pro style, as the spec specifies:
 * `A–L` are the white keys, `W/E/T/Y/U` the black keys, `Z`/`X` shift the octave.
 *
 * Offsets are semitones above the low C of the visible range.
 */
const KEY_MAP: Record<string, number> = {
  a: 0,  // C
  w: 1,  // C#
  s: 2,  // D
  e: 3,  // D#
  d: 4,  // E
  f: 5,  // F
  t: 6,  // F#
  g: 7,  // G
  y: 8,  // G#
  h: 9,  // A
  u: 10, // A#
  j: 11, // B
  k: 12, // C
  l: 14, // D
};

const OCTAVE_DOWN = "z";
const OCTAVE_UP = "x";

/** Two octaves visible, per the spec. */
const VISIBLE_SEMITONES = 24;

/**
 * One octave on a phone.
 *
 * This is the one place the keyboard cannot be fixed by styling it. Two octaves is
 * fifteen white keys; across the ~330pt a phone has left after the octave controls that
 * is 22pt per key, and the black keys sitting on top of them are 14pt wide — a third of
 * what a fingertip can aim at, so every press is a coin toss between two semitones. One
 * octave puts a white key at ~40pt, which is the width the notes have to be for the
 * keyboard to be an instrument rather than a picture of one. The octave buttons beside
 * it are how you reach the rest, and they matter much more here than on a desktop.
 */
const COMPACT_SEMITONES = 12;

interface OnScreenKeyboardProps {
  /** Display only. Rust applies the velocity — this just shows what it will be. */
  velocity: number;
  channel: number;
  onNoteOn: (pitch: number) => void;
  onNoteOff: (pitch: number) => void;
  /** Pitches held by live input, shown alongside local presses. */
  externalNotes?: Set<number>;
  /**
   * Whether the computer keyboard is playing these keys.
   *
   * Off, the mouse still works and every letter belongs to the editor's shortcuts. There
   * is no in-between: `J` cannot be Join and B at the same time, and `L` cannot be
   * Listen and D.
   */
  typing: boolean;
  onTypingChange: (typing: boolean) => void;
}

export function OnScreenKeyboard({
  velocity,
  channel,
  onNoteOn,
  onNoteOff,
  externalNotes,
  typing,
  onTypingChange,
}: OnScreenKeyboardProps) {
  // A phone standing up has no room for two octaves of playable keys; lying down it has
  // the width but only ~60pt of height, and a one-octave board keeps the keys square
  // enough to aim at rather than turning them into slivers.
  const compact = useMediaQuery(PHONE);
  const visibleSemitones = compact ? COMPACT_SEMITONES : VISIBLE_SEMITONES;

  // MIDI 48 = C3, so the default two octaves span C3–C5 around middle C.
  const [baseOctave, setBaseOctave] = useState(4);
  const [held, setHeld] = useState<Set<number>>(new Set());
  const heldRef = useRef(held);
  heldRef.current = held;

  const basePitch = baseOctave * 12;

  const press = useCallback(
    (pitch: number) => {
      if (pitch < 0 || pitch > 127) return;
      setHeld((prev) => {
        if (prev.has(pitch)) return prev;
        const next = new Set(prev);
        next.add(pitch);
        return next;
      });
      onNoteOn(pitch);
    },
    [onNoteOn],
  );

  const release = useCallback(
    (pitch: number) => {
      setHeld((prev) => {
        if (!prev.has(pitch)) return prev;
        const next = new Set(prev);
        next.delete(pitch);
        return next;
      });
      onNoteOff(pitch);
    },
    [onNoteOff],
  );

  // -- computer keyboard ---------------------------------------------------

  useEffect(() => {
    // Nothing is bound at all unless the mode is on. Guarding inside the handler would
    // still swallow auto-repeat and preventDefault from keys the editor wanted.
    if (!typing) return;

    function isTypingTarget(target: EventTarget | null): boolean {
      const element = target as HTMLElement | null;
      return !!element && (
        element.tagName === "INPUT" ||
        element.tagName === "TEXTAREA" ||
        element.isContentEditable
      );
    }

    function onKeyDown(event: KeyboardEvent) {
      if (isTypingTarget(event.target)) return;
      // Leave the editor's own shortcuts (cmd-Z, cmd-A…) alone.
      if (event.metaKey || event.ctrlKey || event.altKey) return;

      const key = event.key.toLowerCase();

      if (key === OCTAVE_DOWN) {
        event.preventDefault();
        setBaseOctave((o) => Math.max(0, o - 1));
        return;
      }
      if (key === OCTAVE_UP) {
        event.preventDefault();
        setBaseOctave((o) => Math.min(9, o + 1));
        return;
      }

      const offset = KEY_MAP[key];
      if (offset === undefined) return;

      // Auto-repeat fires keydown continuously; without this the note retriggers
      // dozens of times a second for as long as the key is held.
      if (event.repeat) return;

      event.preventDefault();
      press(basePitch + offset);
    }

    function onKeyUp(event: KeyboardEvent) {
      if (isTypingTarget(event.target)) return;
      const offset = KEY_MAP[event.key.toLowerCase()];
      if (offset === undefined) return;
      release(basePitch + offset);
    }

    /**
     * Releasing everything when the window loses focus prevents a stuck note: a
     * keyup delivered to another window never reaches us, so the note would sound
     * forever.
     */
    function onBlur() {
      heldRef.current.forEach((pitch) => release(pitch));
    }

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
    };
  }, [typing, basePitch, press, release]);

  // Leaving the mode with keys down would strand them, since the keyup handler goes with
  // the mode that was holding them.
  useEffect(() => {
    if (typing) return;
    heldRef.current.forEach((pitch) => release(pitch));
  }, [typing, release]);

  // -- layout --------------------------------------------------------------

  /** A key is lit if it is held locally or by an external controller. */
  const isHeld = (pitch: number) => held.has(pitch) || externalNotes?.has(pitch) === true;

  const pitches = Array.from({ length: visibleSemitones + 1 }, (_, i) => basePitch + i);
  const whites = pitches.filter((p) => !isBlackKey(p));

  // Black keys are positioned as a fraction of the white-key run so the two octaves
  // stay proportional at any width.
  const whiteIndexOf = (pitch: number) => whites.findIndex((w) => w > pitch);

  return (
    <div className={`keys ${typing ? "keys--typing" : ""}`}>
      <div className="keys__controls">
        <button
          className={`btn keys__typing ${typing ? "btn--primary" : "btn--ghost"}`}
          onClick={() => onTypingChange(!typing)}
          aria-pressed={typing}
          title={
            typing
              ? "Typing plays these keys — editor shortcuts are paused. Esc to leave."
              : "Play these keys from the computer keyboard. Editor shortcuts pause while it is on."
          }
        >
          {typing ? "Typing ⏎" : "Typing"}
        </button>

        <button
          className="btn btn--ghost btn--icon"
          onClick={() => setBaseOctave((o) => Math.max(0, o - 1))}
          aria-label="Octave down"
          title={typing ? "Octave down (Z)" : "Octave down"}
        >
          −
        </button>
        <span className="keys__octave mono">C{baseOctave - 1}</span>
        <button
          className="btn btn--ghost btn--icon"
          onClick={() => setBaseOctave((o) => Math.min(9, o + 1))}
          aria-label="Octave up"
          title={typing ? "Octave up (X)" : "Octave up"}
        >
          +
        </button>
      </div>

      <div
        className="keys__board"
        onPointerLeave={() => heldRef.current.forEach((pitch) => release(pitch))}
      >
        {whites.map((pitch) => (
          <button
            key={pitch}
            className={`keys__white ${isHeld(pitch) ? "keys__white--on" : ""}`}
            onPointerDown={(e) => {
              e.currentTarget.releasePointerCapture?.(e.pointerId);
              press(pitch);
            }}
            onPointerUp={() => release(pitch)}
            onPointerEnter={(e) => {
              // Glissando: sliding with the button down plays across the keys.
              if (e.buttons === 1) press(pitch);
            }}
            onPointerLeave={() => release(pitch)}
            aria-label={pitchName(pitch)}
          >
            <span className="keys__label">{pitch % 12 === 0 ? pitchName(pitch) : ""}</span>
          </button>
        ))}

        {pitches.filter(isBlackKey).map((pitch) => {
          const next = whiteIndexOf(pitch);
          if (next <= 0) return null;
          const left = (next / whites.length) * 100;
          return (
            <button
              key={pitch}
              className={`keys__black ${isHeld(pitch) ? "keys__black--on" : ""}`}
              style={{ left: `${left}%`, width: `${(1 / whites.length) * 100 * 0.62}%` }}
              onPointerDown={(e) => {
                e.currentTarget.releasePointerCapture?.(e.pointerId);
                press(pitch);
              }}
              onPointerUp={() => release(pitch)}
              onPointerEnter={(e) => {
                if (e.buttons === 1) press(pitch);
              }}
              onPointerLeave={() => release(pitch)}
              aria-label={pitchName(pitch)}
            />
          );
        })}
      </div>

      <div className="keys__meta muted">
        {typing && <span className="keys__mode">A–L · W/E/T/Y/U · Z/X · Esc</span>}
        <span className="mono">vel {velocity}</span>
        <span className="mono">ch {channel + 1}</span>
      </div>
    </div>
  );
}
