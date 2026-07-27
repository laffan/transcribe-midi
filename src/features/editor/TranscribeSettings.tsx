import { useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { TranscribeTuning, TranscriptionPreview } from "../../lib/types";
import "./TranscribeSettings.css";

export interface TranscribeOptions {
  useProjectTempo: boolean;
  /** 0 = no snapping; otherwise ppq / divisor gives the grid in ticks. */
  gridDivisor: number;
}

interface TranscribeSettingsProps {
  trackIndex: number;
  ppq: number;
  options: TranscribeOptions;
  /** Carried from the last take, so a file is read the way the microphone was. */
  tuning: TranscribeTuning;
  disabled: boolean;
  onChange: (options: TranscribeOptions) => void;
  onResult: (preview: TranscriptionPreview) => void;
}

const GRIDS: { label: string; divisor: number }[] = [
  { label: "Off", divisor: 0 },
  { label: "1/4", divisor: 1 },
  { label: "1/8", divisor: 2 },
  { label: "1/16", divisor: 4 },
  { label: "1/8 triplet", divisor: 3 },
];

export function gridTicks(options: TranscribeOptions, ppq: number): number {
  return options.gridDivisor === 0 ? 0 : Math.round(ppq / options.gridDivisor);
}

/**
 * Settings for turning audio into notes, plus the file route in.
 *
 * The *action* lives in the transport — Listen is a peer of Record — so what is left here
 * is the two decisions that change the result and the way to bring in audio that was not
 * played just now. That second one matters more than it looks: a voice memo or a bounce
 * is a likelier source than someone humming at a laptop, and once this is a plugin the
 * host's audio is likelier still.
 */
export function TranscribeSettings({
  trackIndex,
  ppq,
  options,
  tuning,
  disabled,
  onChange,
  onResult,
}: TranscribeSettingsProps) {
  const [busy, setBusy] = useState(false);

  async function openFile() {
    setBusy(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({
        multiple: false,
        filters: [
          {
            name: "Audio",
            extensions: ["wav", "aiff", "aif", "caf", "m4a", "mp3", "aac", "flac"],
          },
        ],
      });
      if (typeof path !== "string") return;

      onResult(
        await api.captureLoadFile(
          path,
          trackIndex,
          options.useProjectTempo,
          gridTicks(options, ppq),
          tuning,
        ),
      );
      logger.info("Transcribed from file");
    } catch (error) {
      logger.error("Could not transcribe that file", errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="settings__group">
      <label className="field">
        <span className="field__label">Snap transcription to</span>
        <select
          className="input"
          value={options.gridDivisor}
          disabled={disabled || busy}
          onChange={(e) => onChange({ ...options, gridDivisor: Number(e.target.value) })}
        >
          {GRIDS.map((grid) => (
            <option key={grid.label} value={grid.divisor}>
              {grid.label}
            </option>
          ))}
        </select>
      </label>

      <label className="transcribe__check">
        <input
          type="checkbox"
          checked={options.useProjectTempo}
          disabled={disabled || busy}
          onChange={(e) => onChange({ ...options, useProjectTempo: e.target.checked })}
        />
        <span>
          Use the project tempo
          <span className="field__hint">
            Right when you played to the click. Off, and the tempo is read from the
            performance.
          </span>
        </span>
      </label>

      <button className="btn" onClick={() => void openFile()} disabled={disabled || busy}>
        {busy ? "Reading…" : "Transcribe an audio file…"}
      </button>
      <span className="field__hint">
        One note at a time. Chords are out of scope in this version — a pitch tracker
        handed a chord returns confident nonsense rather than a chord.
      </span>
    </div>
  );
}
