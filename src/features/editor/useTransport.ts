import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, onPlayhead } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, ProjectManifest } from "../../lib/types";
import { barTicks } from "./timeFormat";

interface UseTransportOptions {
  manifest: ProjectManifest | null;
  /** Changes when Settings closes, so input settings are re-read. */
  settingsRevision: number;
  /** Called with the state a committed take produced. */
  onRecorded: (state: EditorState) => void;
}

/**
 * Everything the transport knows, in one place.
 *
 * Rust owns the clock — this only observes the playhead it publishes and asks for state
 * changes. Splitting it out of the editor is what lets the toolbar drive playback while
 * the bar below drives recording without either of them holding the other's state.
 */
export function useTransport({ manifest, settingsRevision, onRecorded }: UseTransportOptions) {
  const [playing, setPlaying] = useState(false);
  const [positionTicks, setPositionTicks] = useState(0);
  const [inCountIn, setInCountIn] = useState(false);
  const [tempoBpm, setTempoBpm] = useState(120);
  const [loopRegion, setLoopRegion] = useState<[number, number] | null>(null);
  const [metronome, setMetronome] = useState(false);
  const [recording, setRecording] = useState(false);
  /** Mirror of the Rust-side setting, for display only — Rust applies it. */
  const [keyboardVelocity, setKeyboardVelocity] = useState(100);

  useEffect(() => {
    if (manifest) setTempoBpm(manifest.tempo_bpm);
  }, [manifest]);

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

  useEffect(() => {
    api
      .inputSettings()
      .then((settings) => {
        setKeyboardVelocity(settings.keyboard_velocity);
        setMetronome(settings.metronome);
      })
      .catch(() => {});
  }, [settingsRevision]);

  const play = useCallback(async () => {
    try {
      setPlaying((await api.transportPlay()).playing);
    } catch (error) {
      logger.error("Could not start playback", errorMessage(error));
    }
  }, []);

  const seek = useCallback(async (tick: number) => {
    try {
      setPositionTicks((await api.transportSeek(tick)).position_ticks);
    } catch (error) {
      logger.error("Could not move the playhead", errorMessage(error));
    }
  }, []);

  /** Stop where you are. What a DAW's pause does, and what Rust's stop already did. */
  const pause = useCallback(async () => {
    try {
      setPlaying((await api.transportStop()).playing);
    } catch (error) {
      logger.error("Could not stop playback", errorMessage(error));
    }
  }, []);

  /** Stop and rewind — the other half of what "stop" means to anyone using a DAW. */
  const stop = useCallback(async () => {
    await pause();
    await seek(0);
  }, [pause, seek]);

  const toggleRecord = useCallback(async () => {
    try {
      if (recording) {
        onRecorded(await api.recordStop());
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
  }, [recording, onRecorded]);

  /**
   * Set the looped span outright, or clear it.
   *
   * Beside `toggleLoop` rather than instead of it: the button is for "loop around here,
   * I do not care exactly where", and dragging it out in the ruler is for when you do.
   * Both end at the same command, so the transport cannot hold two ideas of the loop.
   */
  const setLoop = useCallback(async (region: [number, number] | null) => {
    try {
      setLoopRegion((await api.setLoopRegion(region)).loop_region);
    } catch (error) {
      logger.error("Could not set the loop", errorMessage(error));
    }
  }, []);

  const toggleLoop = useCallback(async () => {
    if (!manifest) return;
    // Default to two bars from the playhead: a loop has to come from somewhere, and
    // dragging brackets before there is anything to loop is more ceremony than it is
    // worth. Once set, the region is visible in the ruler.
    const next: [number, number] | null = loopRegion
      ? null
      : (() => {
          const bar = barTicks(manifest.ppq, manifest.time_signature);
          const start = Math.floor(positionTicks / bar) * bar;
          return [start, start + bar * 2];
        })();

    try {
      setLoopRegion((await api.setLoopRegion(next)).loop_region);
    } catch (error) {
      logger.error("Could not set the loop", errorMessage(error));
    }
  }, [loopRegion, manifest, positionTicks]);

  const toggleMetronome = useCallback(async () => {
    const next = !metronome;
    setMetronome(next);
    try {
      await api.setMetronome(next);
    } catch (error) {
      setMetronome(!next);
      logger.error("Could not toggle the metronome", errorMessage(error));
    }
  }, [metronome]);

  const changeTempo = useCallback((bpm: number) => {
    setTempoBpm(bpm);
    void api
      .setTempo(bpm)
      .catch((error) => logger.error("Tempo change failed", errorMessage(error)));
  }, []);

  return {
    playing,
    positionTicks,
    inCountIn,
    tempoBpm,
    loopRegion,
    metronome,
    recording,
    keyboardVelocity,
    play,
    pause,
    stop,
    seek,
    toggleRecord,
    setLoop,
    toggleLoop,
    toggleMetronome,
    changeTempo,
  };
}
