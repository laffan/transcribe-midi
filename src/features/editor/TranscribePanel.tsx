import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, Note, TranscriptionPreview } from "../../lib/types";
import "./TranscribePanel.css";

interface TranscribePanelProps {
  trackIndex: number;
  ppq: number;
  onApplied: (state: EditorState) => void;
  /** Notes to draw over the roll while the result is being reviewed. */
  onPreviewChange: (notes: Note[] | null) => void;
}

/** Poll interval while recording. Drives the meter and the elapsed time only. */
const POLL_MS = 100;

const GRIDS: { label: string; divisor: number }[] = [
  { label: "Off", divisor: 0 },
  { label: "1/4", divisor: 1 },
  { label: "1/8", divisor: 2 },
  { label: "1/16", divisor: 4 },
  { label: "1/8 triplet", divisor: 3 },
];

/**
 * Phase 7: play or hum a line and get notes.
 *
 * Monophonic only, and the panel says so rather than letting the user find out. Given a
 * chord the pitch tracker returns one pitch — usually the loudest partial — and a UI
 * that implied otherwise would be making a promise the DSP cannot keep.
 */
export function TranscribePanel({
  trackIndex,
  ppq,
  onApplied,
  onPreviewChange,
}: TranscribePanelProps) {
  const [recording, setRecording] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const [level, setLevel] = useState(0);
  const [atLimit, setAtLimit] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [permissionDenied, setPermissionDenied] = useState(false);
  const [result, setResult] = useState<TranscriptionPreview | null>(null);

  const [useProjectTempo, setUseProjectTempo] = useState(true);
  const [gridDivisor, setGridDivisor] = useState(4);

  const onPreviewChangeRef = useRef(onPreviewChange);
  onPreviewChangeRef.current = onPreviewChange;

  const clearResult = useCallback(() => {
    setResult(null);
    onPreviewChangeRef.current(null);
  }, []);

  // The result is a set of notes bound to one track. Switching tracks makes it
  // meaningless, so it goes rather than sitting there looking applicable.
  useEffect(() => {
    setResult((current) => {
      if (!current) return null;
      void api.captureCancel().catch(() => {});
      onPreviewChangeRef.current(null);
      return null;
    });
  }, [trackIndex]);

  // Poll while recording. Nothing here is on a timing path: the samples are placed by
  // their position in the buffer, not by when this happens to run.
  useEffect(() => {
    if (!recording) return;

    const timer = window.setInterval(() => {
      api
        .capturePoll()
        .then((status) => {
          setSeconds(status.seconds);
          setLevel(status.level);
          setAtLimit(status.at_limit);
        })
        .catch(() => {});
    }, POLL_MS);

    return () => window.clearInterval(timer);
  }, [recording]);

  // Never leave the microphone running because a panel unmounted.
  useEffect(() => () => void api.captureCancel().catch(() => {}), []);

  async function start() {
    setError(null);
    setPermissionDenied(false);
    clearResult();
    try {
      const status = await api.captureStart();
      setRecording(status.recording);
      setSeconds(0);
      logger.info("Listening…");
    } catch (e) {
      if (isCommandError(e) && e.code === "microphone_denied") {
        setPermissionDenied(true);
      }
      setError(errorMessage(e));
    }
  }

  async function stopAndTranscribe() {
    setBusy(true);
    setRecording(false);
    try {
      const quantizeTicks = gridDivisor === 0 ? 0 : Math.round(ppq / gridDivisor);
      const preview = await api.captureTranscribe(trackIndex, useProjectTempo, quantizeTicks);

      setResult(preview);
      onPreviewChangeRef.current(
        preview.notes.length > 0 ? preview.notes.map((detected) => detected.note) : null,
      );

      if (preview.warning) logger.warn(preview.warning);
      else logger.info(`Transcribed ${preview.notes.length} notes`);
    } catch (e) {
      setError(errorMessage(e));
      logger.error("Transcription failed", errorMessage(e));
    } finally {
      setBusy(false);
      setLevel(0);
    }
  }

  async function accept() {
    try {
      const state = await api.captureAccept();
      onApplied(state);
      logger.info(
        `Added ${result?.notes.length ?? 0} notes — undo it like any other edit`,
      );
      clearResult();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function discard() {
    await api.captureCancel().catch(() => {});
    setRecording(false);
    clearResult();
    setError(null);
  }

  const outOfTune = result?.notes.filter((n) => Math.abs(n.cents_off) > 35).length ?? 0;

  return (
    <div className="transcribe">
      <div className="transcribe__head">
        <h3 className="transcribe__title">Audio to MIDI</h3>
      </div>

      <p className="field__hint">
        Play or hum <strong>one note at a time</strong>. Chords are out of scope in this
        version — polyphonic transcription is a different problem, and guessing at one
        would produce confident nonsense.
      </p>

      {recording ? (
        <>
          <div className="transcribe__meter" role="img" aria-label={`Input level ${Math.round(level * 100)}%`}>
            <div className="transcribe__meter-fill" style={{ width: `${Math.min(100, level * 130)}%` }} />
          </div>
          <div className="transcribe__actions">
            <button className="btn btn--primary" disabled={busy} onClick={() => void stopAndTranscribe()}>
              {busy ? "Transcribing…" : "Stop & transcribe"}
            </button>
            <button className="btn" onClick={() => void discard()}>
              Cancel
            </button>
            <span className="mono transcribe__clock">{seconds.toFixed(1)}s</span>
          </div>
          {atLimit && (
            <p className="field__hint">
              That is as long as one take can be. Stop and transcribe what you have.
            </p>
          )}
        </>
      ) : (
        <div className="transcribe__actions">
          <button className="btn btn--primary" disabled={busy} onClick={() => void start()}>
            {busy ? "Transcribing…" : "Record"}
          </button>
        </div>
      )}

      <label className="field">
        <span className="field__label">Snap to</span>
        <select
          className="input"
          value={gridDivisor}
          disabled={recording || busy}
          onChange={(e) => setGridDivisor(Number(e.target.value))}
        >
          {GRIDS.map((grid) => (
            <option key={grid.label} value={grid.divisor}>
              {grid.label}
            </option>
          ))}
        </select>
      </label>

      <label className="transcribe__check">
        <input
          type="checkbox"
          checked={useProjectTempo}
          disabled={recording || busy}
          onChange={(e) => setUseProjectTempo(e.target.checked)}
        />
        <span>
          Use the project tempo
          <span className="field__hint">
            Right when you played to the click. Turn it off and the tempo is estimated
            from what you played instead.
          </span>
        </span>
      </label>

      {error && (
        <p className="transcribe__error">
          {error}
          {permissionDenied && (
            <span className="field__hint">
              Allow microphone access in System Settings → Privacy &amp; Security, then try
              again.
            </span>
          )}
        </p>
      )}

      {result && (
        <div className="transcribe__result">
          {result.warning && <p className="transcribe__warning">{result.warning}</p>}

          <dl className="transcribe__facts">
            <dt>Notes</dt>
            <dd className="mono">{result.notes.length}</dd>
            <dt>Length</dt>
            <dd className="mono">{result.duration_seconds.toFixed(1)}s</dd>
            <dt>Tempo</dt>
            <dd className="mono">
              {result.tempo_bpm.toFixed(0)}
              {result.tempo_estimated ? " (estimated)" : ""}
            </dd>
            <dt>Pitched</dt>
            <dd className="mono">{Math.round(result.pitched_fraction * 100)}%</dd>
          </dl>

          {outOfTune > 0 && (
            <p className="field__hint">
              {outOfTune} note{outOfTune === 1 ? " sat" : "s sat"} more than a third of a
              semitone off — the source may be out of tune, which the pitch has been
              rounded past.
            </p>
          )}

          {result.notes.length > 0 && (
            <div className="transcribe__actions">
              <button className="btn btn--primary" onClick={() => void accept()}>
                Add to track
              </button>
              <button className="btn" onClick={() => void discard()}>
                Discard
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
