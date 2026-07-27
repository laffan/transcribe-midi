import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState } from "../../lib/types";
import { type Pending } from "./pending";
import { gridTicks, type TranscribeOptions } from "./TranscribeSettings";

interface PendingDeps {
  ppq: number | undefined;
  selectedTrack: number;
  setEditor: (state: EditorState) => void;
  setSelection: (indices: number[]) => void;
  setSelectedTrack: (index: number) => void;
}

/**
 * Whatever produced notes and is waiting on a decision, and the listening that produces it.
 *
 * One slot for both an AI edit and a transcription, because they are one interaction from
 * the user's side — something proposed notes, and it is accepted or it is not — and only
 * one can be outstanding at a time.
 */
export function usePendingEdit({
  ppq,
  selectedTrack,
  setEditor,
  setSelection,
  setSelectedTrack,
}: PendingDeps) {
  const [pending, setPending] = useState<Pending>(null);
  const [listening, setListening] = useState(false);
  const [listenLevel, setListenLevel] = useState(0);
  const [fineTuning, setFineTuning] = useState(false);
  const [transcribeOptions, setTranscribeOptions] = useState<TranscribeOptions>({
    useProjectTempo: true,
    gridDivisor: 4,
  });

  const discard = useCallback(async () => {
    const current = pending;
    setPending(null);
    setFineTuning(false);
    if (current?.kind === "ai") await api.aiReject().catch(() => {});
    if (current?.kind === "transcription") await api.captureCancel().catch(() => {});
  }, [pending]);

  const apply = useCallback(async () => {
    if (!pending) return;
    try {
      const state = pending.kind === "ai" ? await api.aiAccept() : await api.captureAccept();
      setEditor(state);
      setSelection(state.affected);
      setSelectedTrack(state.affected_track);
      setPending(null);
      setFineTuning(false);
      logger.info("Applied — undo it like any other edit");
    } catch (error) {
      logger.error("Could not apply that", errorMessage(error));
    }
  }, [pending, setEditor, setSelection, setSelectedTrack]);

  const toggleListen = useCallback(async () => {
    if (listening) {
      setListening(false);
      try {
        const preview = await api.captureTranscribe(
          selectedTrack,
          transcribeOptions.useProjectTempo,
          gridTicks(transcribeOptions, ppq ?? 480),
        );
        setPending({ kind: "transcription", preview });
        // Open the waveform straight away. For an AI edit the diff on the roll is the
        // review; for a take, the waveform is — you cannot judge a transcription against
        // a grid, only against the sound it came from.
        setFineTuning(preview.notes.length > 0);
        if (preview.warning) logger.warn(preview.warning);
        else logger.info(`Transcribed ${preview.notes.length} notes`);
      } catch (error) {
        logger.error("Transcription failed", errorMessage(error));
      }
      return;
    }

    await discard();
    try {
      const status = await api.captureStart();
      setListening(status.recording);
      logger.info("Listening — play or hum one note at a time");
    } catch (error) {
      if (isCommandError(error) && error.code === "microphone_denied") {
        logger.error(
          "Microphone access is off",
          "Allow it in System Settings → Privacy & Security, then try again.",
        );
      } else {
        logger.error("Could not start listening", errorMessage(error));
      }
    }
  }, [listening, selectedTrack, transcribeOptions, ppq, discard]);

  const cancelListening = useCallback(() => {
    setListening(false);
    void api.captureCancel().catch(() => {});
  }, []);

  // Poll the level while listening. Nothing here is on a timing path — samples are placed
  // by their position in the buffer, not by when this runs.
  useEffect(() => {
    if (!listening) return;
    const timer = window.setInterval(() => {
      api
        .capturePoll()
        .then((status) => setListenLevel(status.level))
        .catch(() => {});
    }, 100);
    return () => window.clearInterval(timer);
  }, [listening]);

  // Never leave the microphone running because the editor closed.
  useEffect(() => () => void api.captureCancel().catch(() => {}), []);

  // A pending result is bound to one track's notes. Switching tracks makes it
  // meaningless, so it goes rather than sitting there looking applicable.
  useEffect(() => {
    setPending((current) => {
      if (!current) return null;
      if (current.kind === "ai") void api.aiReject().catch(() => {});
      else void api.captureCancel().catch(() => {});
      return null;
    });
    setFineTuning(false);
  }, [selectedTrack]);

  return {
    pending,
    setPending,
    listening,
    listenLevel,
    fineTuning,
    setFineTuning,
    transcribeOptions,
    setTranscribeOptions,
    apply,
    discard,
    toggleListen,
    cancelListening,
  };
}

export type PendingEdit = ReturnType<typeof usePendingEdit>;
