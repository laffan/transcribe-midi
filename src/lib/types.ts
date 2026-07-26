/**
 * Mirrors of the Rust types in `unplugged-core`.
 *
 * These are hand-maintained rather than generated. If you change a `#[derive(Serialize)]`
 * struct over there, change it here too — there is no compile-time link between them.
 * (Generating these from schemars is worth doing once the model stops moving; right now
 * it would be churn.)
 */

export interface Note {
  pitch: number;
  start_ticks: number;
  duration_ticks: number;
  velocity: number;
  channel: number;
}

export interface TimeSignature {
  numerator: number;
  denominator: number;
}

/** Tagged enum: `#[serde(tag = "kind", rename_all = "snake_case")]`. */
export type InstrumentRef = { kind: "built_in_sampler" };

export interface TrackMeta {
  id: string;
  name: string;
  channel: number;
  instrument: InstrumentRef;
  muted: boolean;
  soloed: boolean;
  color: string;
  key_hint: string | null;
}

/** `TrackMeta` is `#[serde(flatten)]`ed into `Track`, so the JSON is flat. */
export interface Track extends TrackMeta {
  notes: Note[];
  ppq: number;
}

export interface ProjectManifest {
  schema_version: number;
  id: string;
  name: string;
  tempo_bpm: number;
  time_signature: TimeSignature;
  ppq: number;
  tracks: TrackMeta[];
  created_at_ms: number;
  modified_at_ms: number;
}

export interface Project {
  manifest: ProjectManifest;
  tracks: Track[];
}

export interface ProjectSummary {
  id: string;
  name: string;
  tempo_bpm: number;
  time_signature: TimeSignature;
  track_count: number;
  created_at_ms: number;
  modified_at_ms: number;
}

export interface ProjectListError {
  id: string;
  message: string;
}

export interface ProjectListing {
  projects: ProjectSummary[];
  errors: ProjectListError[];
}

/** Shape of a rejected `invoke`, matching `CommandError` in `src-tauri/src/error.rs`. */
export interface CommandError {
  code: string;
  message: string;
}

// --- Editor (phase 3) ------------------------------------------------------

export interface EditorState {
  tracks: Track[];
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  dirty: boolean;
  /** Indices in `affected_track` the UI should select after this edit. */
  affected: number[];
  affected_track: number;
}

/**
 * Editor gestures. Mirrors `EditRequest` in src-tauri/src/editor.rs.
 *
 * The frontend describes intent and Rust derives the resulting notes, so clamping lives
 * in exactly one place and the webview cannot write a note that breaks the model.
 */
export type EditRequest =
  | { kind: "insert"; track: number; note: Note }
  | { kind: "delete"; track: number; indices: number[] }
  | { kind: "move"; track: number; indices: number[]; delta_ticks: number; delta_pitch: number }
  | { kind: "resize"; track: number; indices: number[]; delta_ticks: number }
  | { kind: "set_velocity"; track: number; indices: number[]; velocity: number }
  | { kind: "quantize"; track: number; indices: number[]; grid_ticks: number }
  | { kind: "paste"; track: number; notes: Note[]; at_ticks: number };

// --- Transport (phase 2) ---------------------------------------------------

export interface TransportState {
  playing: boolean;
  position_ticks: number;
  tempo_bpm: number;
  loop_region: [number, number] | null;
  engine_running: boolean;
  sample_rate: number;
}

export interface PlayheadEvent {
  position_ticks: number;
  playing: boolean;
}

export const MIN_TEMPO = 20;
export const MAX_TEMPO = 300;
export const DEFAULT_TEMPO = 120;
export const DEFAULT_PPQ = 480;
