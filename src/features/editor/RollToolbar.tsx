import { GRID_OPTIONS } from "./pianoRollGeometry";

/**
 * The strip above the grid: what notes snap to, how far apart the bars are, how tall a
 * semitone is, and what is selected.
 *
 * A sibling of PianoRoll rather than part of it because it shares nothing with the
 * canvas but four numbers — and because on a phone this is the row that has to argue
 * hardest for its own height against the grid it sits on top of, which is easier to
 * reason about when it is one file with one job.
 */
interface RollToolbarProps {
  gridDivisor: number;
  onGridDivisorChange: (divisor: number) => void;
  /** Horizontal zoom, in pixels per tick. */
  pxPerTick: number;
  onPxPerTickChange: (value: number) => void;
  /** Vertical zoom: the height of one semitone lane, in pixels. */
  rowHeight: number;
  onRowHeightChange: (value: number) => void;
  selectionCount: number;
}

export function RollToolbar({
  gridDivisor,
  onGridDivisorChange,
  pxPerTick,
  onPxPerTickChange,
  rowHeight,
  onRowHeightChange,
  selectionCount,
}: RollToolbarProps) {
  return (
    <div className="roll__toolbar">
      <label className="roll__control">
        <span className="roll__control-label">Grid</span>
        <select
          className="input roll__select"
          value={gridDivisor}
          onChange={(e) => onGridDivisorChange(Number(e.target.value))}
        >
          {GRID_OPTIONS.map((option) => (
            <option key={option.label} value={option.divisor}>
              {option.label}
            </option>
          ))}
        </select>
      </label>

      <label className="roll__control">
        <span className="roll__control-label">Zoom</span>
        <input
          type="range"
          min={0.005}
          max={1}
          step={0.005}
          value={pxPerTick}
          onChange={(e) => onPxPerTickChange(Number(e.target.value))}
          className="roll__range"
          aria-label="Horizontal zoom"
        />
      </label>

      <label className="roll__control">
        <span className="roll__control-label">Rows</span>
        <input
          type="range"
          min={6}
          max={28}
          step={1}
          value={rowHeight}
          onChange={(e) => onRowHeightChange(Number(e.target.value))}
          className="roll__range"
          aria-label="Vertical zoom"
        />
      </label>

      <div className="spacer" />

      {/*
        The standing instruction is hidden at phone widths — it says "click", on a
        device with nothing to click — but the selection count replaces it there, which
        is why the two states carry different classes rather than one element changing
        its text.
      */}
      <span className={`roll__hint muted ${selectionCount > 0 ? "roll__hint--selection" : ""}`}>
        {selectionCount > 0 ? `${selectionCount} selected` : "click to add · drag to select"}
      </span>
    </div>
  );
}
