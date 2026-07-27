import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, onPlayhead } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, ProjectManifest } from "../../lib/types";

interface TransportDeps {
  manifest: ProjectManifest | null;
  /** Committing a take returns a new editor state, because a take is an edit. */
  setEditor: (state: EditorState) => void;
  selectedTrack: number;
}

/**
 * Playhead, transport buttons, recording and the loop region.
 *
 * The position is pushed from Rust rather than polled: the sequencer knows where it is to
 * the sample, and anything derived on this side would be a second, disagreeing clock.
 */
export function useTransport({ manifest, setEditor, selectedTrack }: TransportDeps) {
  const [playing, setPlaying] = useState(false);
  const [positionTicks, setPositionTicks] = useState(0);
  const [inCountIn, setInCountIn] = useState(false);
  const [recording, setRecording] = useState(false);
  const [loopRegion, setLoopRegion] = useState<[number, number] | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    onPlayhead((event) => {
      setPositionTicks(event.position_ticks);
      setPlaying(event.playing);
      setInCountIn(event.in_count_in);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((error) => logger.error("Could not subscribe to the playhead", errorMessage(error)));

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const play = useCallback(async () => {
    try {
      const state = await api.transportPlay();
      setPlaying(state.playing);
    } catch (error) {
      logger.error("Could not start playback", errorMessage(error));
    }
  }, []);

  const stop = useCallback(async () => {
    try {
      const state = await api.transportStop();
      setPlaying(state.playing);
    } catch (error) {
      logger.error("Could not stop playback", errorMessage(error));
    }
  }, []);

  const seek = useCallback(async (tick: number) => {
    try {
      const state = await api.transportSeek(tick);
      setPositionTicks(state.position_ticks);
    } catch (error) {
      logger.error("Could not move the playhead", errorMessage(error));
    }
  }, []);

  const toggleRecord = useCallback(async () => {
    try {
      if (recording) {
        setEditor(await api.recordStop());
        setRecording(false);
        logger.info("Take committed — undo it like any other edit");
      } else {
        const result = await api.recordStart();
        setRecording(result.recording);
        if (result.count_in_ticks > 0) logger.info("Counting in…");
      }
    } catch (error) {
      setRecording(false);
      logger.error("Recording failed", errorMessage(error));
    }
  }, [recording, setEditor]);

  const toggleLoop = useCallback(async () => {
    // Default to two bars from the playhead: a loop has to come from somewhere, and
    // dragging brackets before there is anything to loop is more ceremony than it is
    // worth. Once set, the region is visible in the ruler.
    const next: [number, number] | null =
      loopRegion || !manifest
        ? null
        : (() => {
            const bar =
              (manifest.ppq * 4 * manifest.time_signature.numerator) /
              manifest.time_signature.denominator;
            const start = Math.floor(positionTicks / bar) * bar;
            return [start, start + bar * 2];
          })();

    try {
      const state = await api.setLoopRegion(next);
      setLoopRegion(state.loop_region);
    } catch (error) {
      logger.error("Could not set the loop", errorMessage(error));
    }
  }, [loopRegion, manifest, positionTicks]);

  // Keep the armed track in step with the selected one, so recording lands where the
  // user is looking rather than on whichever track was armed last.
  useEffect(() => {
    if (recording) return;
    api.setArmedTrack(selectedTrack).catch(() => {});
  }, [selectedTrack, recording]);

  return {
    playing,
    positionTicks,
    inCountIn,
    recording,
    loopRegion,
    play,
    stop,
    seek,
    toggleRecord,
    toggleLoop,
  };
}

export type Transport = ReturnType<typeof useTransport>;
