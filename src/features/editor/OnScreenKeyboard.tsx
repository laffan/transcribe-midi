import { useCallback, useEffect, useRef, useState } from "react";

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

interface OnScreenKeyboardProps {
  velocity: number;
  channel: number;
  onNoteOn: (pitch: number, velocity: number) => void;
  onNoteOff: (pitch: number) => void;
}

export function OnScreenKeyboard({ velocity, channel, onNoteOn, onNoteOff }: OnScreenKeyboardProps) {
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
      onNoteOn(pitch, velocity);
    },
    [onNoteOn, velocity],
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
  }, [basePitch, press, release]);

  // -- layout --------------------------------------------------------------

  const pitches = Array.from({ length: VISIBLE_SEMITONES + 1 }, (_, i) => basePitch + i);
  const whites = pitches.filter((p) => !isBlackKey(p));

  // Black keys are positioned as a fraction of the white-key run so the two octaves
  // stay proportional at any width.
  const whiteIndexOf = (pitch: number) => whites.findIndex((w) => w > pitch);

  return (
    <div className="keys">
      <div className="keys__controls">
        <button
          className="btn btn--ghost btn--icon"
          onClick={() => setBaseOctave((o) => Math.max(0, o - 1))}
          aria-label="Octave down"
          title="Octave down (Z)"
        >
          −
        </button>
        <span className="keys__octave mono">C{baseOctave - 1}</span>
        <button
          className="btn btn--ghost btn--icon"
          onClick={() => setBaseOctave((o) => Math.min(9, o + 1))}
          aria-label="Octave up"
          title="Octave up (X)"
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
            className={`keys__white ${held.has(pitch) ? "keys__white--on" : ""}`}
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
              className={`keys__black ${held.has(pitch) ? "keys__black--on" : ""}`}
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
        <span className="mono">vel {velocity}</span>
        <span className="mono">ch {channel + 1}</span>
      </div>
    </div>
  );
}
