import { useEffect, useMemo, useRef, useState } from "react";

import {
  gridTicks,
  KEY_WIDTH,
  MAX_PITCH,
  VELOCITY_LANE_HEIGHT,
  type Viewport,
  xToTick,
} from "./pianoRollGeometry";

/**
 * Where the roll is looking, and everything that moves it.
 *
 * Zoom and scroll are one concern: a ⌘-scroll zoom has to adjust the scroll position in the
 * same gesture so the tick under the pointer stays where it is, and separating them would
 * put those two updates in different files.
 */
export function usePianoRollView(ppq: number) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const [size, setSize] = useState({ width: 800, height: 400 });
  const [gridDivisor, setGridDivisor] = useState(16);
  const [pxPerTick, setPxPerTick] = useState(0.12);
  const [rowHeight, setRowHeight] = useState(12);
  const [scrollTicks, setScrollTicks] = useState(0);
  const [topPitch, setTopPitch] = useState(84);

  const view: Viewport = useMemo(
    () => ({ pxPerTick, rowHeight, scrollTicks, topPitch }),
    [pxPerTick, rowHeight, scrollTicks, topPitch],
  );

  const snap = gridTicks(ppq, gridDivisor);
  const rollHeight = Math.max(80, size.height - VELOCITY_LANE_HEIGHT);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) setSize({ width: Math.max(200, rect.width), height: Math.max(160, rect.height) });
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Registered non-passively so `preventDefault` works — otherwise the whole page
    // scrolls (and on a trackpad, pinch-zooms the webview) instead of the roll.
    function onWheel(event: WheelEvent) {
      event.preventDefault();
      const rect = canvas!.getBoundingClientRect();
      const x = event.clientX - rect.left;

      if (event.ctrlKey || event.metaKey) {
        // Zoom about the pointer, so the tick under the cursor stays put.
        const anchorTick = xToTick(x, view);
        const factor = Math.exp(-event.deltaY * 0.003);
        const next = Math.min(4, Math.max(0.005, pxPerTick * factor));
        setPxPerTick(next);
        setScrollTicks(Math.max(0, anchorTick - (x - KEY_WIDTH) / next));
        return;
      }

      if (event.shiftKey) {
        setScrollTicks((prev) => Math.max(0, prev + event.deltaY / pxPerTick));
        return;
      }

      setScrollTicks((prev) => Math.max(0, prev + event.deltaX / pxPerTick));
      setTopPitch((prev) =>
        Math.min(MAX_PITCH, Math.max(12, prev - Math.round(event.deltaY / rowHeight))),
      );
    }

    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [view, pxPerTick, rowHeight]);

  return {
    containerRef,
    canvasRef,
    size,
    view,
    snap,
    rollHeight,
    gridDivisor,
    setGridDivisor,
    pxPerTick,
    setPxPerTick,
    rowHeight,
    setRowHeight,
  };
}

export type PianoRollView = ReturnType<typeof usePianoRollView>;
