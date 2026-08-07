/**
 * Wheel and trackpad navigation for the transcription editor.
 *
 * Beside the pinch maths in `transcribeGeometry` rather than inside the component, and
 * for the same reason the roll keeps its two together: a pinch and a ⌘-scroll are one
 * behaviour with two input devices, and they have to anchor on the same rule or zooming
 * feels like two different features. A hook rather than an `onWheel` prop because the
 * listener must be non-passive — React's is passive, so `preventDefault` is ignored
 * there and the page scrolls (and a trackpad pinch zooms the whole webview) instead.
 */

import { useEffect, type RefObject } from "react";

import {
  midiOf,
  panWindow,
  secondsOf,
  WAVE_HEIGHT,
  zoomPitch,
  zoomTime,
  type Scale,
  type Window,
} from "./transcribeGeometry";

export interface TranscribeWheelOptions {
  canvasRef: RefObject<HTMLCanvasElement | null>;
  scale: Scale;
  duration: number;
  setView: (update: (previous: Window) => Window) => void;
}

export function useTranscribeWheel({
  canvasRef,
  scale,
  duration,
  setView,
}: TranscribeWheelOptions): void {
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    function onWheel(event: WheelEvent) {
      event.preventDefault();
      const rect = canvas!.getBoundingClientRect();
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;

      if (event.ctrlKey || event.metaKey) {
        // ⌘-scroll zooms time about the cursor, ⇧⌘ zooms pitch about it — the two axes
        // this view has, on the two modifiers a trackpad can spare.
        const factor = Math.exp(-event.deltaY * 0.003);
        setView((view) =>
          event.shiftKey
            ? zoomPitch(view, factor, midiOf(scale, y), duration)
            : zoomTime(view, factor, secondsOf(scale, x), duration),
        );
        return;
      }

      const laneHeight = Math.max(80, scale.height - WAVE_HEIGHT);
      setView((view) => {
        const seconds = (event.deltaX / Math.max(1, scale.width)) * view.spanSeconds;
        // Scrolling down reveals what is below, which in a lane where pitch rises
        // upward means lower notes.
        const semitones = -(event.deltaY / laneHeight) * (view.high - view.low);
        return panWindow(view, seconds, semitones, duration);
      });
    }

    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [canvasRef, scale, duration, setView]);
}
