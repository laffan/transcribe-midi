import { useEffect, useRef, useState } from "react";

import { api } from "../../lib/api";
import type { WaveformPeaks } from "../../lib/types";
import "./ListeningBar.css";

interface ListeningBarProps {
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
 * same `capture_waveform` the fine-tune editor uses, asked for repeatedly.
 */
export function ListeningBar({ onStop, onCancel }: ListeningBarProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [peaks, setPeaks] = useState<WaveformPeaks>([]);
  const [seconds, setSeconds] = useState(0);
  const [atLimit, setAtLimit] = useState(false);
  const [width, setWidth] = useState(600);

  useEffect(() => {
    const timer = window.setInterval(async () => {
      try {
        const status = await api.capturePoll();
        setSeconds(status.seconds);
        setAtLimit(status.at_limit);
        if (status.seconds > 0) {
          setPeaks(await api.captureWaveform(0, status.seconds, Math.round(width)));
        }
      } catch {
        // A failed poll is not worth reporting: the next one is 100 ms away, and the
        // transcribe call will surface anything that actually matters.
      }
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [width]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setWidth(Math.max(120, rect.width));
    });
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const height = 40;
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
      ctx.moveTo(x, mid - max * (mid - 2));
      ctx.lineTo(x, mid - min * (mid - 2));
    });
    ctx.stroke();
  }, [peaks, width]);

  return (
    <div className="listening">
      <span className="listening__dot" aria-hidden="true" />
      <span className="listening__label">
        Listening
        <span className="mono listening__clock">{seconds.toFixed(1)}s</span>
      </span>

      <canvas ref={canvasRef} className="listening__wave" style={{ height: 40 }} />

      {atLimit && (
        <span className="listening__limit">
          That is as long as one take can be — stop and transcribe what you have.
        </span>
      )}

      <button className="btn btn--primary" onClick={onStop}>
        Stop &amp; transcribe
      </button>
      <button className="btn" onClick={onCancel}>
        Cancel
      </button>
    </div>
  );
}
