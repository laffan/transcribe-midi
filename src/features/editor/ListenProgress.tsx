import { useEffect, useState } from "react";

import { api } from "../../lib/api";

/** How often to ask how far along it is. */
const POLL_MS = 80;

/**
 * The wait between the last note sung and the transcription appearing.
 *
 * It is seconds of arithmetic — YIN over every frame, then an FFT over every frame — and
 * it used to happen with the overlay closed, which made the app look like it had dropped
 * the take and gone back to the editor. Staying put and reporting is the whole of the
 * fix; Rust publishes a real fraction, so the bar is measuring rather than decorating.
 */
export function ListenProgress() {
  const [fraction, setFraction] = useState(0);
  const [seconds, setSeconds] = useState(0);

  // The take is still in Rust while it is being read, so its length can be asked for
  // rather than carried down from whoever was recording.
  useEffect(() => {
    api
      .capturePoll()
      .then((status) => setSeconds(status.seconds))
      .catch(() => {});
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => {
      api
        .captureProgress()
        // Never backwards: a poll landing between two runs would otherwise snap the bar
        // to zero at the moment it was about to finish.
        .then((at) => setFraction((current) => Math.max(current, at)))
        .catch(() => {});
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <>
      <div className="listen__body listen__body--working">
        <div className="working">
          <p className="working__title">Reading the take</p>

          <div
            className="working__bar"
            role="progressbar"
            aria-label="Transcription progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(fraction * 100)}
          >
            <span className="working__fill" style={{ width: `${fraction * 100}%` }} />
          </div>

          <p className="working__detail mono">
            {Math.round(fraction * 100)}%{seconds > 0 && ` · ${seconds.toFixed(1)}s of audio`}
          </p>
          <p className="field__hint">
            Finding the pitch in every ten milliseconds of it, then the attacks. The
            recording is kept afterwards, so nothing here has to be performed twice.
          </p>
        </div>
      </div>

      <footer className="listen__actions">
        <span className="field__hint">This is arithmetic on your machine, not a request to anywhere.</span>
      </footer>
    </>
  );
}
