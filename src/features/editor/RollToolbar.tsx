import { GRID_OPTIONS } from "./pianoRollGeometry";

interface RollToolbarProps {
  gridDivisor: number;
  onGridChange: (divisor: number) => void;
  pxPerTick: number;
  onZoomChange: (pxPerTick: number) => void;
  rowHeight: number;
  onRowHeightChange: (rowHeight: number) => void;
  selectionCount: number;
}

/** Grid, the two zooms, and a line telling you what a click and a drag do. */
export function RollToolbar({
  gridDivisor,
  onGridChange,
  pxPerTick,
  onZoomChange,
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
          onChange={(e) => onGridChange(Number(e.target.value))}
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
          onChange={(e) => onZoomChange(Number(e.target.value))}
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

      <span className="roll__hint muted">
        {selectionCount > 0 ? `${selectionCount} selected` : "click to add · drag to select"}
      </span>
    </div>
  );
}
