import { useRef, type KeyboardEvent, type PointerEvent } from "react";

import "./Slider.css";

interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** How to read the number back to the user. */
  format: (value: number) => string;
  /** One line under the control saying what moving it does. */
  hint?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}

/**
 * One value on a line, dragged with a finger or a mouse.
 *
 * `<input type="range">` was here first and is the reason this exists. Its thumb is a
 * platform decision — about 12px in WKWebView — and it cannot be made bigger without
 * `::-webkit-slider-thumb`, which then has to be re-implemented for every other engine
 * anyway. A dial you cannot land on with a fingertip is not a dial, and these are the
 * controls you reach for *because* a transcription came back wrong, on the device where
 * the take was just sung.
 *
 * So it is a pointer-driven control: one code path for mouse, pen and touch, a hit row
 * the height of a finger with a line drawn through it, and the same keyboard contract a
 * range input has — arrows to step, Home and End for the ends, Page to move in tens.
 * Pointer capture is what makes a drag survive leaving the track, which is most drags on
 * a control this thin.
 */
export function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  hint,
  disabled = false,
  onChange,
}: SliderProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const fraction = Math.min(1, Math.max(0, (value - min) / (max - min)));

  /** The value under a pointer, snapped to the step and clamped to the ends. */
  function valueAt(clientX: number): number {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return value;
    const t = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    const raw = min + t * (max - min);
    const snapped = Math.round(raw / step) * step;
    // Steps like 0.05 do not divide the range exactly, and floating point makes the
    // result 0.30000000000000004 — which then prints as itself. The decimals the step
    // has are all the decimals the value can need.
    const decimals = (String(step).split(".")[1] ?? "").length;
    return Number(Math.min(max, Math.max(min, snapped)).toFixed(decimals));
  }

  function onPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (disabled) return;
    event.preventDefault();
    dragging.current = true;
    event.currentTarget.setPointerCapture(event.pointerId);
    onChange(valueAt(event.clientX));
  }

  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    if (!dragging.current || disabled) return;
    onChange(valueAt(event.clientX));
  }

  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    dragging.current = false;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (disabled) return;
    const moves: Record<string, number> = {
      ArrowLeft: -step,
      ArrowDown: -step,
      ArrowRight: step,
      ArrowUp: step,
      PageDown: -step * 10,
      PageUp: step * 10,
      Home: min - value,
      End: max - value,
    };
    const delta = moves[event.key];
    if (delta === undefined) return;
    event.preventDefault();
    const decimals = (String(step).split(".")[1] ?? "").length;
    const next = Math.min(max, Math.max(min, value + delta));
    onChange(Number(next.toFixed(decimals)));
  }

  return (
    <div className={`slider ${disabled ? "slider--disabled" : ""}`}>
      <div className="slider__head">
        <span className="slider__label">{label}</span>
        <span className="slider__value mono">{format(value)}</span>
      </div>

      <div
        ref={trackRef}
        className="slider__hit"
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-label={label}
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={value}
        aria-valuetext={format(value)}
        aria-disabled={disabled}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onKeyDown={onKeyDown}
        style={{ ["--slider-fill" as string]: `${fraction * 100}%` }}
      >
        <span className="slider__track">
          <span className="slider__fill" />
        </span>
        <span className="slider__thumb" />
      </div>

      {hint && <span className="slider__hint">{hint}</span>}
    </div>
  );
}
