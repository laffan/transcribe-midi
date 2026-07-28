import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { AuditionSource } from "../../lib/types";

/** How often to ask Rust where playback has reached. Fast enough for a moving line. */
const POLL_MS = 50;

/**
 * Hearing the take back, from either end of the transcription.
 *
 * The notes play by default. They are what is being decided about — the recording is the
 * evidence you check them against, and offering the evidence first meant the only way to
 * hear the actual result was to accept it and find out.
 *
 * Rust owns both players and answers one position for whichever is running, so nothing
 * here has to know which one made the sound.
 */
export function useAudition(kind: "take" | "notes" = "take") {
  const [source, setSource] = useState<AuditionSource>("midi");
  const [playhead, setPlayhead] = useState<number | null>(null);
  /** Read by the source switch, which restarts playback from wherever it had reached. */
  const playheadRef = useRef<number | null>(null);
  playheadRef.current = playhead;

  const stop = useCallback(() => {
    void api.auditionStop().catch(() => {});
    setPlayhead(null);
  }, []);

  const playFrom = useCallback(
    async (seconds: number, using?: AuditionSource) => {
      const from = Math.max(0, seconds);
      try {
        // A proposal has no recording behind it, so there is no source to choose — the
        // notes are all there is.
        if (kind === "notes") await api.auditionNotes(from);
        else await api.auditionTake(from, using ?? source);
        setPlayhead(from);
      } catch (error) {
        logger.error("Could not play that back", errorMessage(error));
        setPlayhead(null);
      }
    },
    [kind, source],
  );

  /** Switch source. Playing when it happens means playing on, from the same place. */
  const changeSource = useCallback(
    (next: AuditionSource) => {
      setSource(next);
      if (playheadRef.current !== null) void playFrom(playheadRef.current, next);
    },
    [playFrom],
  );

  const toggle = useCallback(() => {
    if (playhead === null) void playFrom(0);
    else stop();
  }, [playhead, playFrom, stop]);

  // Polling rather than an event: playback is not on a timing path here, and a stream of
  // position events from two different clocks would be more machinery than a playhead
  // needs. `null` back from Rust means it reached the end.
  useEffect(() => {
    if (playhead === null) return;
    const timer = window.setInterval(() => {
      api
        .auditionPosition()
        .then(setPlayhead)
        .catch(() => setPlayhead(null));
    }, POLL_MS);
    return () => window.clearInterval(timer);
    // Keyed on whether anything is playing, not on where it has reached: the position
    // changes on every tick and would otherwise rebuild the interval thirty times a
    // second.
  }, [playhead === null]);

  // Never leave anything sounding because the view went away.
  useEffect(() => stop, [stop]);

  return { source, changeSource, playhead, playFrom, stop, toggle };
}
