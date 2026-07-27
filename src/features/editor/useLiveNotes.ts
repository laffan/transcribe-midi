import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, onLiveNote } from "../../lib/api";
import { logger } from "../../lib/console";

/**
 * Notes sounding right now from live input, and the three ways to make one sound.
 *
 * The `liveNotes` set is purely cosmetic: by the time an event arrives the note has already
 * sounded and, if the transport is recording, already been captured in Rust. It covers the
 * on-screen keyboard as well as an external one, because both go through the same Rust path.
 *
 * No track argument anywhere — Rust routes live input to the armed track, and velocity is
 * left to Rust so the Settings value stays the single source of truth.
 */
export function useLiveNotes(channel: number) {
  const [liveNotes, setLiveNotes] = useState<Set<number>>(new Set());

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    onLiveNote((event) => {
      setLiveNotes((prev) => {
        const next = new Set(prev);
        if (event.on) next.add(event.pitch);
        else next.delete(event.pitch);
        return next;
      });
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const noteOn = useCallback(
    (pitch: number) => {
      void api
        .liveNoteOn(pitch, channel)
        .catch((error) => logger.error("Note failed", errorMessage(error)));
    },
    [channel],
  );

  const noteOff = useCallback(
    (pitch: number) => {
      void api.liveNoteOff(pitch, channel).catch(() => {});
    },
    [channel],
  );

  const previewNote = useCallback(
    (pitch: number) => {
      noteOn(pitch);
      // Auditioning a note in the roll should be a blip, not a held tone.
      window.setTimeout(() => noteOff(pitch), 180);
    },
    [noteOn, noteOff],
  );

  return { liveNotes, noteOn, noteOff, previewNote };
}
