import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type {
  AiProposal,
  AiTarget,
  EditorState,
  TranscribeTuning,
  TranscriptionPreview,
} from "../../lib/types";
import { DEFAULT_TUNING } from "../../lib/types";
import { type Pending } from "./pending";
import { gridTicks, type TranscribeOptions } from "./TranscribeSettings";

interface UsePendingOptions {
  selectedTrack: number;
  ppq: number;
  /** Called with the state accepting a pending result produced. */
  onApplied: (state: EditorState) => void;
}

/** Which stage of the full-window overlay is on screen, if any. */
export type OverlayStage = "capture" | "working" | "thinking" | "review" | "proposal" | null;

/**
 * The two ways notes arrive, and the one decision outstanding.
 *
 * One hook because there is one slot: a transcription and a described edit are the same
 * interaction with different innards — something produces notes, you look at them, you
 * accept or reject — and only one can be outstanding at a time. Which stage the overlay
 * is showing is not separate state; it is this state read differently.
 *
 * `pending` outlives the overlay on purpose. Closing it puts the result back in the
 * review bar rather than throwing it away, so "let me look at the roll first" is not a
 * decision to discard.
 */
export function usePending({ selectedTrack, ppq, onApplied }: UsePendingOptions) {
  const [pending, setPending] = useState<Pending>(null);
  const [listening, setListening] = useState(false);
  const [working, setWorking] = useState(false);
  const [thinking, setThinking] = useState(false);
  /** What was asked for, so the wait can say what it is waiting on. */
  const [prompt, setPrompt] = useState("");
  const [reviewing, setReviewing] = useState(false);
  const [options, setOptions] = useState<TranscribeOptions>({
    useProjectTempo: true,
    gridDivisor: 4,
  });
  /**
   * The tuning the *next* take is read with. Re-deriving answers with the tuning it
   * used, so this only has to carry it from one take to the next — settings you had to
   * find once should not need finding again.
   */
  const [tuning, setTuning] = useState<TranscribeTuning>(DEFAULT_TUNING);

  const stage: OverlayStage = listening
    ? "capture"
    : working
      ? "working"
      : thinking
        ? "thinking"
        : reviewing && pending?.kind === "transcription"
          ? "review"
          : reviewing && pending?.kind === "ai"
            ? "proposal"
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

  // -- listening -----------------------------------------------------------

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

  /** Adopt an analysis of the take, whether it is the first or the fifth. */
  const adopt = useCallback((preview: TranscriptionPreview) => {
    setPending({ kind: "transcription", preview });
    setTuning(preview.tuning);
    setReviewing(true);
  }, []);

  const stopAndTranscribe = useCallback(async () => {
    setListening(false);
    // Reading a two-minute take is seconds of work. The overlay stays up and says so:
    // closing it and reopening when the answer arrived made the wait look like a
    // failure, and the window it left was the editor, which is not what you were doing.
    setWorking(true);
    try {
      const preview = await api.captureTranscribe(
        selectedTrack,
        options.useProjectTempo,
        gridTicks(options, ppq),
        tuning,
      );
      adopt(preview);
      if (preview.warning) logger.warn(preview.warning);
      else logger.info(`Transcribed ${preview.notes.length} notes`);
    } catch (error) {
      logger.error("Transcription failed", errorMessage(error));
    } finally {
      setWorking(false);
    }
  }, [selectedTrack, options, ppq, tuning, adopt]);

  /**
   * Read the retained take again with different settings.
   *
   * Every bit as expensive as the first pass — the same pipeline over the same samples —
   * so it shows the same progress stage rather than leaving the editor frozen with stale
   * notes on it and no sign that anything is happening.
   */
  const reprocess = useCallback(
    async (useProjectTempo: boolean, quantizeTicks: number, next: TranscribeTuning) => {
      setWorking(true);
      try {
        adopt(await api.captureRetranscribe(useProjectTempo, quantizeTicks, next));
      } catch (error) {
        logger.error("Could not re-read the take", errorMessage(error));
      } finally {
        setWorking(false);
      }
    },
    [adopt],
  );

  const cancelListen = useCallback(() => {
    setListening(false);
    setWorking(false);
    setReviewing(false);
    void api.captureCancel().catch(() => {});
  }, []);

  const toggleListen = useCallback(() => {
    void (listening ? stopAndTranscribe() : startListen());
  }, [listening, stopAndTranscribe, startListen]);

  // -- describing ----------------------------------------------------------

  /**
   * Set when the user stops waiting for an answer.
   *
   * A ref rather than state because it is read inside the request's own continuation,
   * which closed over whatever the value was when it started.
   */
  const abandoned = useRef(false);

  const describe = useCallback(
    async (prompt: string, selection: number[], target: AiTarget) => {
      abandoned.current = false;
      setPrompt(prompt);
      setThinking(true);
      try {
        const proposal = await api.aiPropose(selectedTrack, prompt, selection, target);
        if (abandoned.current) {
          // It arrived after the user walked away. Rust is still holding a transaction
          // against notes that may have moved since, so it goes.
          await api.aiReject().catch(() => {});
          return;
        }
        setPending({ kind: "ai", proposal });
        setReviewing(true);
        if (proposal.truncated) logger.warn("The model ran out of turns before finishing");
      } catch (error) {
        if (!abandoned.current) throw error;
      } finally {
        setThinking(false);
      }
    },
    [selectedTrack],
  );

  const cancelDescribe = useCallback(() => {
    abandoned.current = true;
    setThinking(false);
    logger.info("Stopped waiting — the answer is discarded if it arrives");
  }, []);

  /** Adopt a proposal whose notes the user adjusted. Rust re-derives its own diff. */
  const replaceProposal = useCallback((proposal: AiProposal) => {
    setPending({ kind: "ai", proposal });
  }, []);

  // -- lifecycle -----------------------------------------------------------

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
    listening,
    thinking,
    prompt,
    stage,
    options,
    setOptions,
    tuning,
    toggleListen,
    stopAndTranscribe,
    cancelListen,
    reprocess,
    describe,
    cancelDescribe,
    applyPending,
    discardPending,
    replaceProposal,
    /** A transcription that arrived from a file rather than the microphone. */
    takeResult: adopt,
    openReview: () => setReviewing(true),
    closeReview: () => setReviewing(false),
  };
}
