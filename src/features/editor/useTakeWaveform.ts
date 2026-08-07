import { useEffect, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { WaveformPeaks } from "../../lib/types";
import type { Window } from "./transcribeGeometry";

/** Buckets asked for when measuring the whole take. Enough to catch a single attack. */
const SURVEY_BUCKETS = 512;
/** A pinch changes the window every frame; the peaks are fetched after it settles. */
const SETTLE_MS = 60;
/** Below this, a take has nothing in it and amplifying it would draw noise. */
const SILENT_PEAK = 0.002;
/** How far a quiet take may be amplified before the picture stops being honest. */
const MAX_GAIN = 24;

export interface TakeWaveform {
  /** Min/max pairs, one per pixel of the window. */
  peaks: WaveformPeaks;
  /** What to multiply them by so the take fills the lane. */
  gain: number;
  /** What to say in the lane when there is nothing in it, or null. */
  note: string | null;
}

/**
 * The picture of the take, at the window being looked at and at a height worth looking at.
 *
 * Two requests with two different jobs. The **window** one is redrawn as the view moves —
 * one bucket per pixel of what is on screen, asked of Rust rather than shipping samples,
 * because two minutes at 48 kHz is over twenty megabytes for a few hundred columns. The
 * **survey** one runs once per take and answers a different question: how loud was any of
 * this.
 *
 * That second question is the whole reason this file exists. The lane drew absolute
 * amplitude, and a line hummed at arm's length from a tablet records at about a fifteenth
 * of full scale — three pixels of picture in ninety-six pixels of lane, which reads as a
 * broken waveform rather than as a quiet one. So the peaks are scaled to fill the lane,
 * and the scale is measured **once over the whole take** rather than per window: a
 * window-relative gain would swell a quiet passage the moment you scrolled to it, and
 * comparing one part of a take with another is what this lane is for.
 *
 * Every way of ending up with an empty lane is named rather than left blank, including
 * the one that used to be swallowed — a failed read is not the same as silence, and from
 * a device they look identical.
 */
export function useTakeWaveform(
  /** Changes identity when a take is re-read; the survey re-runs then and not otherwise. */
  preview: unknown,
  duration: number,
  view: Window,
  width: number,
): TakeWaveform {
  const [peaks, setPeaks] = useState<WaveformPeaks>([]);
  const [peak, setPeak] = useState(1);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .captureWaveform(0, duration, SURVEY_BUCKETS)
      .then((survey) => {
        if (cancelled) return;
        const loudest = survey.reduce((so_far, [min, max]) => Math.max(so_far, -min, max), 0);
        setPeak(loudest);
        setNote(
          survey.length === 0
            ? "no recording is attached to this take"
            : loudest < SILENT_PEAK
              ? "that take is almost silent"
              : null,
        );
      })
      .catch((error) => {
        if (cancelled) return;
        setPeak(1);
        setNote("the recording could not be read");
        logger.error("Could not read the take's waveform", errorMessage(error));
      });
    return () => {
      cancelled = true;
    };
  }, [preview]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      const from = Math.max(0, view.startSeconds);
      const to = Math.min(duration, view.startSeconds + view.spanSeconds);
      if (to <= from) {
        setPeaks([]);
        return;
      }
      api
        .captureWaveform(from, to, Math.round(width))
        .then((fetched) => {
          // The window may run off either end of the take. The peaks cover only the part
          // that exists, so they are placed where that part is rather than at x = 0.
          const before = Math.round(((from - view.startSeconds) / view.spanSeconds) * width);
          setPeaks(
            before > 0
              ? [...Array.from({ length: before }, () => [0, 0] as [number, number]), ...fetched]
              : fetched,
          );
        })
        .catch(() => setPeaks([]));
    }, SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [view, duration, width]);

  return {
    peaks,
    gain: peak > SILENT_PEAK ? Math.min(MAX_GAIN, 0.92 / peak) : 1,
    note,
  };
}
