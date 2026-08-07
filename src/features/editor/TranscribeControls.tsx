import type { AuditionSource } from "../../lib/types";

const SOURCES: { value: AuditionSource; label: string; title: string }[] = [
  { value: "midi", label: "Notes", title: "Play the transcription on the sampler" },
  { value: "take", label: "Recording", title: "Play the take you performed" },
  { value: "both", label: "Both", title: "Play them together and compare" },
];

export const GRIDS: { label: string; divisor: number }[] = [
  { label: "Off", divisor: 0 },
  { label: "1/4", divisor: 1 },
  { label: "1/8", divisor: 2 },
  { label: "1/16", divisor: 4 },
  { label: "1/8 triplet", divisor: 3 },
];

interface TranscribeControlsProps {
  playing: boolean;
  onTogglePlay: () => void;
  source: AuditionSource;
  onSourceChange: (source: AuditionSource) => void;
  loop: boolean;
  onLoopChange: (loop: boolean) => void;
  /** The stretch dragged out on the waveform, if any, in seconds. */
  loopRegion: [number, number] | null;
  onClearLoop: () => void;
  /** True when the window is showing less than the whole take. */
  zoomed: boolean;
  onZoom: (factor: number) => void;
  onFit: () => void;
  /** The selected note, named, with how far the take actually sat from it. */
  selected: { name: string; cents: number | null } | null;
  onNudgePitch: (semitones: number) => void;
  gridDivisor: number;
  onGridChange: (divisor: number) => void;
  useProjectTempo: boolean;
  onProjectTempoChange: (value: boolean) => void;
  canJoin: boolean;
  onJoin: () => void;
  onDelete: () => void;
  onDiscard: () => void;
  onApply: () => void;
  canApply: boolean;
}

/**
 * Everything under the take: what plays, what is being looked at, and what happens next.
 *
 * Split from the editor because the two have nothing to say to each other beyond a list
 * of callbacks — this file is a row of controls, and the one it came out of is pointer
 * arithmetic over a canvas. The order across the bar is the order of the work: hear it,
 * find the part that is wrong, fix that note, then decide about the take.
 */
export function TranscribeControls({
  playing,
  onTogglePlay,
  source,
  onSourceChange,
  loop,
  onLoopChange,
  loopRegion,
  onClearLoop,
  zoomed,
  onZoom,
  onFit,
  selected,
  onNudgePitch,
  gridDivisor,
  onGridChange,
  useProjectTempo,
  onProjectTempoChange,
  canJoin,
  onJoin,
  onDelete,
  onDiscard,
  onApply,
  canApply,
}: TranscribeControlsProps) {
  return (
    <>
      <div className="tedit__row">
        <button className="btn btn--lg" onClick={onTogglePlay}>
          {playing ? "⏹ Stop" : "▶ Play"}
        </button>

        <div className="segmented" role="group" aria-label="What to play">
          {SOURCES.map((option) => (
            <button
              key={option.value}
              className={`segmented__option ${source === option.value ? "segmented__option--on" : ""}`}
              onClick={() => onSourceChange(option.value)}
              aria-pressed={source === option.value}
              title={option.title}
            >
              {option.label}
            </button>
          ))}
        </div>

        {/* Loop and the stretch it plays. The label says which of the two it is on,
            because "round and round" over the wrong four seconds is a confusing thing
            to listen to and the difference is otherwise only visible as shading. */}
        <div className="tedit__loop" role="group" aria-label="Loop">
          <button
            className={`btn ${loop ? "btn--active" : ""}`}
            onClick={() => onLoopChange(!loop)}
            aria-pressed={loop}
            title={
              loopRegion
                ? "Play the marked stretch round and round (L)"
                : "Play what is on screen round and round (L) — drag across the waveform to mark a stretch"
            }
          >
            ⟲ Loop
          </button>
          {loopRegion && (
            <button
              className="btn btn--ghost"
              onClick={onClearLoop}
              title="Forget the marked stretch and loop what is on screen instead"
            >
              <span className="mono">{(loopRegion[1] - loopRegion[0]).toFixed(2)}s</span> ✕
            </button>
          )}
        </div>

        {/* Zoom is the editing tool here, not a view preference: a semitone and a tenth
            of a second are both too small to work with until you are in close. */}
        <div className="tedit__zoom" role="group" aria-label="Zoom">
          <button className="btn btn--icon" onClick={() => onZoom(1 / 1.6)} title="Zoom out (−)">
            −
          </button>
          <button className="btn btn--icon" onClick={() => onZoom(1.6)} title="Zoom in (+)">
            +
          </button>
          <button className="btn" onClick={onFit} disabled={!zoomed} title="Show the whole take (0)">
            Fit
          </button>
        </div>

        {/*
          The selected note, and what the take actually did under it. The arrows are the
          only way to move a note by exactly a semitone on a touchscreen — a drag is a
          drag, and this is a step. Cents are shown because a note the analysis rounded
          up from 44 cents flat is a note worth listening to again, and nothing else on
          screen says so in a number.
        */}
        {selected && (
          <div className="tedit__note" aria-label="Selected note">
            <button
              className="btn btn--icon"
              onClick={() => onNudgePitch(-1)}
              title="Down a semitone (↓)"
              aria-label="Down a semitone"
            >
              ▼
            </button>
            <span className="tedit__note-name mono">{selected.name}</span>
            {selected.cents !== null && (
              <span className="tedit__note-cents mono" title="How far the take sat from this note">
                {selected.cents > 0 ? "+" : ""}
                {selected.cents}¢
              </span>
            )}
            <button
              className="btn btn--icon"
              onClick={() => onNudgePitch(1)}
              title="Up a semitone (↑)"
              aria-label="Up a semitone"
            >
              ▲
            </button>
          </div>
        )}

        <span className="field__hint">
          Drag a note to move it, its ends to change length. Drag across the waveform to
          mark a stretch to loop; press it to play from there. Pinch or ⌘-scroll to zoom —
          sideways for time, up and down for pitch.
        </span>
      </div>

      <div className="tedit__row">
        <label className="tedit__control">
          <span className="field__label">Snap to</span>
          <select
            className="input"
            value={gridDivisor}
            onChange={(e) => onGridChange(Number(e.target.value))}
          >
            {GRIDS.map((grid) => (
              <option key={grid.label} value={grid.divisor}>
                {grid.label}
              </option>
            ))}
          </select>
        </label>

        <label className="tedit__check">
          <input
            type="checkbox"
            checked={useProjectTempo}
            onChange={(e) => onProjectTempoChange(e.target.checked)}
          />
          <span>Use the project tempo</span>
        </label>

        <button
          className="btn"
          onClick={onJoin}
          disabled={!canJoin}
          title="Join this note to the one after it (J)"
        >
          Join
        </button>
        <button
          className="btn"
          onClick={onDelete}
          disabled={!selected}
          title="Delete the selected note (⌫)"
        >
          Delete note
        </button>

        <div className="spacer" />

        {/* The two decisions, kept together so a narrow screen can give them a row of
            their own: this is the one place in the app where a mis-tap either keeps a
            transcription you had not read or throws away one you had. */}
        <div className="tedit__decide">
          <button className="btn" onClick={onDiscard}>
            Discard take
          </button>
          <button className="btn btn--primary btn--lg" onClick={onApply} disabled={!canApply}>
            Add to track
          </button>
        </div>
      </div>
    </>
  );
}
