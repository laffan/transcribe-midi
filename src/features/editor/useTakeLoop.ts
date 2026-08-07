import { useCallback, useEffect, useMemo, useState } from "react";

import type { Loop, Window } from "./transcribeGeometry";
import { useAudition } from "./useAudition";

interface UseTakeLoopOptions {
  /** Changes identity when a take is re-read: playback stops and the mark is dropped. */
  preview: unknown;
  duration: number;
  /** What is on screen, which is what Loop plays until a stretch has been marked. */
  view: Window;
}

/**
 * Hearing the take, and the stretch of it being heard.
 *
 * Split from the editor because it is the one piece of state in this view that is not
 * about a note: what is playing, whether it repeats, and which seconds it repeats over.
 * The editor's job is pointers and pixels, and it needs from this only what to draw and
 * what to put on the buttons.
 *
 * `playing` is deliberately not `playhead !== null`. It is whether the user *wants*
 * sound, which the loop needs to know because it restarts playback the moment it ends —
 * so "it stopped" cannot be the signal to stop wanting it.
 */
export function useTakeLoop({ preview, duration, view }: UseTakeLoopOptions) {
  const { source, changeSource, playhead, playFrom, stop } = useAudition();
  const [playing, setPlaying] = useState(false);
  const [loop, setLoop] = useState(false);
  /**
   * The stretch marked on the waveform. `null` means "what is on screen", which is what
   * Loop does before anything has been dragged.
   *
   * Both, rather than one or the other, because they answer at different precisions:
   * "play what I am looking at" needs no gesture and re-aims itself as you zoom, and a
   * marked stretch is what you want when the thing to hear is four notes inside a window
   * you want to keep looking at whole.
   */
  const [region, setRegion] = useState<Loop | null>(null);

  const loopSpan: Loop = useMemo(
    () =>
      region ?? [
        Math.max(0, view.startSeconds),
        Math.min(duration, view.startSeconds + view.spanSeconds),
      ],
    [region, view, duration],
  );

  const start = useCallback(
    (seconds: number) => {
      setPlaying(true);
      void playFrom(seconds);
    },
    [playFrom],
  );

  const halt = useCallback(() => {
    setPlaying(false);
    stop();
  }, [stop]);

  const togglePlay = useCallback(() => {
    if (playing) halt();
    else start(loop ? loopSpan[0] : 0);
  }, [playing, halt, start, loop, loopSpan]);

  // Round and round: back to the top the moment it reaches the end, or the moment the
  // player reports it has stopped while the user still wants sound.
  useEffect(() => {
    if (!loop || !playing) return;
    if (playhead === null || playhead >= loopSpan[1]) void playFrom(loopSpan[0]);
  }, [loop, playing, playhead, loopSpan, playFrom]);

  // Playback that ran off the end with no loop to catch it is playback that stopped.
  useEffect(() => {
    if (playing && !loop && playhead === null) setPlaying(false);
  }, [playing, loop, playhead]);

  // Re-deriving replaces the notes under the playhead, so nothing may still be sounding
  // and the marked stretch belongs to a take that no longer exists.
  useEffect(() => {
    stop();
    setPlaying(false);
    setRegion(null);
  }, [preview, stop]);

  return {
    source,
    changeSource,
    playhead,
    playing,
    loop,
    setLoop,
    region,
    setRegion,
    loopSpan,
    start,
    halt,
    togglePlay,
  };
}
