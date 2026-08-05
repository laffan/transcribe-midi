import { useEffect, useRef, useState } from "react";

import { api } from "../../lib/api";
import type { WaveformPeaks } from "../../lib/types";

interface ListenCaptureProps {
  onStop: () => void;
  onCancel: () => void;
}

/** How often to redraw. Fast enough to feel live, slow enough not to matter. */
const POLL_MS = 100;

/**
 * What you see while the microphone is open.
 *
 * A level meter tells you the input is alive; it does not tell you whether what you just
 * sang came through, and by the time the transcription appears it is too late to know
 * whether a gap was you or the microphone. Drawing the take as it accumulates answers
 * that while there is still time to start again.
 *
 * The samples are already in Rust — `capture_poll` drains them there — so this is the
 * same `capture_waveform` the fine-tune stage uses, asked for repeatedly.
 */
export function ListenCapture({ onStop, onCancel }: ListenCaptureProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [peaks, setPeaks] = useState<WaveformPeaks>([]);
  const [seconds, setSeconds] = useState(0);
  const [level, setLevel] = useState(0);
  const [atLimit, setAtLimit] = useState(false);
  const [size, setSize] = useState({ width: 900, height: 240 });

  useEffect(() => {
    const timer = window.setInterval(async () => {
      try {
        const status = await api.capturePoll();
        setSeconds(status.seconds);
        setLevel(status.level);
        setAtLimit(status.at_limit);
        if (status.seconds > 0) {
          setPeaks(await api.captureWaveform(0, status.seconds, Math.round(size.width)));
        }
      } catch {
        // A failed poll is not worth reporting: the next one is 100 ms away, and the
        // transcribe call will surface anything that actually matters.
      }
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [size.width]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) {
        setSize({ width: Math.max(240, rect.width), height: Math.max(120, rect.height) });
      }
    });
    observer.observe(canvas.parentElement ?? canvas);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const { width, height } = size;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== width * dpr || canvas.height !== height * dpr) {
      canvas.width = width * dpr;
      canvas.height = height * dpr;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const style = getComputedStyle(document.documentElement);
    ctx.strokeStyle = style.getPropertyValue("--diff-added").trim() || "#6cb08a";

    const mid = height / 2;
    ctx.beginPath();
    peaks.forEach(([min, max], index) => {
      const x = index + 0.5;
      ctx.moveTo(x, mid - max * (mid - 8));
      ctx.lineTo(x, mid - min * (mid - 8));
    });
    ctx.stroke();
  }, [peaks, size]);

  return (
    <>
      <div className="listen__body listen__body--capture">
        <div className="capture__wave">
          <canvas ref={canvasRef} style={{ width: size.width, height: size.height }} />
        </div>

        <div className="capture__readout">
          <span className="capture__dot" aria-hidden="true" />
          <span className="capture__clock mono">{seconds.toFixed(1)}s</span>
          <span className="capture__meter" aria-label="Input level">
            <span
              className="capture__meter-fill"
              style={{ width: `${Math.min(100, level * 130)}%` }}
            />
          </span>
          <span className="field__hint">One note at a time — chords come back as nonsense.</span>
        </div>

        {atLimit && (
          <p className="capture__limit">
            That is as long as one take can be — stop and transcribe what you have.
          </p>
        )}
      </div>

      <footer className="listen__actions">
        <span className="field__hint">
          Nothing is committed yet. Stopping transcribes the take and opens it for
          fine-tuning; the recording is kept until you accept or discard the notes.
        </span>
        <div className="spacer" />
        <button className="btn" onClick={onCancel}>
          Cancel
        </button>
        <button className="btn btn--primary btn--lg" onClick={onStop}>
          Stop &amp; transcribe
        </button>
      </footer>
    </>
  );
}
