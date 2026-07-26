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

export const MIN_TEMPO = 20;
export const MAX_TEMPO = 300;
export const DEFAULT_TEMPO = 120;
export const DEFAULT_PPQ = 480;
