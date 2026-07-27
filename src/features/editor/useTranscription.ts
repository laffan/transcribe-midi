import { useCallback, useEffect, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, TranscriptionPreview } from "../../lib/types";
import { type Pending } from "./pending";
import { gridTicks, type TranscribeOptions } from "./TranscribeSettings";

interface UseTranscriptionOptions {
  selectedTrack: number;
  ppq: number;
  /** Called with the state accepting a transcription produced. */
  onApplied: (state: EditorState) => void;
}

/** Which stage of the listen overlay is on screen, if any. */
export type ListenStage = "capture" | "review" | null;

/**
 * Turning audio into notes, from pressing Listen to accepting the result.
 *
 * One hook because it is one activity with one outstanding result: the microphone, the
 * transcription waiting on a decision, and which stage of it the overlay is showing are
 * not three pieces of state that happen to change together — they are the same state.
 *
 * `pending` outlives the overlay on purpose. Closing the overlay puts the take back in
 * the review bar rather than throwing it away, so "let me look at the roll first" is not
 * a decision to discard.
 */
export function useTranscription({ selectedTrack, ppq, onApplied }: UseTranscriptionOptions) {
  /**
   * Whatever produced notes and is waiting on a decision — an AI edit or a
   * transcription. One slot, because they are one interaction and only one can be
   * outstanding at a time.
   */
  const [pending, setPending] = useState<Pending>(null);
  const [listening, setListening] = useState(false);
  const [reviewing, setReviewing] = useState(false);
  const [options, setOptions] = useState<TranscribeOptions>({
    useProjectTempo: true,
    gridDivisor: 4,
  });

  const stage: ListenStage = listening
    ? "capture"
    : reviewing && pending?.kind === "transcription"
      ? "review"
      : null;

  const discardPending = useCallback(async () => {
    const current = pending;
    setPending(null);
    setReviewing(false);
    if (current?.kind === "ai") await api.aiReject().catch(() => {});
    if (current?.kind === "transcription") await api.captureCancel().catch(() => {});
  }, [pending]);

  const applyPending = useCallback(async () => {
    if (!pending) return;
    try {
      const state = pending.kind === "ai" ? await api.aiAccept() : await api.captureAccept();
      setPending(null);
      setReviewing(false);
      onApplied(state);
      logger.info("Applied — undo it like any other edit");
    } catch (error) {
      logger.error("Could not apply that", errorMessage(error));
    }
  }, [pending, onApplied]);

  const startListen = useCallback(async () => {
    await discardPending();
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
  }, [discardPending]);

  const stopAndTranscribe = useCallback(async () => {
    setListening(false);
    try {
      const preview = await api.captureTranscribe(
        selectedTrack,
        options.useProjectTempo,
        gridTicks(options, ppq),
      );
      setPending({ kind: "transcription", preview });
      // Straight into review, in the same window the take was performed in. For an AI
      // edit the diff on the roll is the review; for a take, the waveform is — you
      // cannot judge a transcription against a grid, only against the sound it came from.
      setReviewing(true);
      if (preview.warning) logger.warn(preview.warning);
      else logger.info(`Transcribed ${preview.notes.length} notes`);
    } catch (error) {
      logger.error("Transcription failed", errorMessage(error));
    }
  }, [selectedTrack, options, ppq]);

  const cancelListen = useCallback(() => {
    setListening(false);
    setReviewing(false);
    void api.captureCancel().catch(() => {});
  }, []);

  const toggleListen = useCallback(() => {
    void (listening ? stopAndTranscribe() : startListen());
  }, [listening, stopAndTranscribe, startListen]);

  /** A transcription that arrived from a file rather than the microphone. */
  const takeResult = useCallback((preview: TranscriptionPreview) => {
    setPending({ kind: "transcription", preview });
    setReviewing(true);
  }, []);

  // A pending result is bound to one track's notes. Switching tracks makes it
  // meaningless, so it goes rather than sitting there looking applicable.
  useEffect(() => {
    setPending((current) => {
      if (!current) return null;
      if (current.kind === "ai") void api.aiReject().catch(() => {});
      else void api.captureCancel().catch(() => {});
      return null;
    });
    setReviewing(false);
  }, [selectedTrack]);

  // Never leave the microphone running because the editor closed.
  useEffect(() => () => void api.captureCancel().catch(() => {}), []);

  return {
    pending,
    setPending,
    listening,
    stage,
    options,
    setOptions,
    toggleListen,
    stopAndTranscribe,
    cancelListen,
    applyPending,
    discardPending,
    takeResult,
    openReview: () => setReviewing(true),
    closeReview: () => setReviewing(false),
    /** Replace the preview in place, when re-deriving produced a new one. */
    replacePreview: (preview: TranscriptionPreview) =>
      setPending({ kind: "transcription", preview }),
  };
}
