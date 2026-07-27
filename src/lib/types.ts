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
  /** The whole undo stack, oldest first. */
  history: string[];
  redo_history: string[];
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
  in_count_in: boolean;
}

// --- Input and recording (phase 4) -----------------------------------------

export interface MidiPort {
  id: string;
  name: string;
}

export interface InputSettings {
  ports: MidiPort[];
  connected: MidiPort | null;
  channel: number | null;
  armed_track: number;
  keyboard_velocity: number;
  count_in_bars: number;
  metronome: boolean;
  recording: boolean;
}

export interface RecordResult {
  recording: boolean;
  count_in_ticks: number;
  captured_notes: number;
}

/** Live input arriving from an external controller, for UI feedback only. */
export interface LiveNoteEvent {
  pitch: number;
  velocity: number;
  on: boolean;
}

// --- Interchange (phase 5) -------------------------------------------------

export interface ExportPayload {
  filename: string;
  bytes: number[];
}

export interface ImportPreview {
  track_names: string[];
  note_counts: number[];
  source_ppq: number | null;
  tempo_bpm: number | null;
  time_signature: TimeSignature | null;
  will_rescale: boolean;
}

export interface ImportResult {
  editor: EditorState;
  tracks_added: number;
  notes_added: number;
  rescaled_from: number | null;
}

export interface PlatformCapabilities {
  share_sheet: boolean;
  drag_out: boolean;
  pasteboard: boolean;
  save_dialog: boolean;
}

// --- AI (phase 6) ----------------------------------------------------------

export interface AiStatus {
  has_key: boolean;
  /** Last four characters. The key itself never crosses this boundary. */
  key_hint: string | null;
  /** False on a build without a Keychain, where the key lasts until the app quits. */
  key_persists: boolean;
  model: string | null;
}

export interface ModelInfo {
  id: string;
  display_name: string;
}

export interface AiModelsResponse {
  models: ModelInfo[];
  default: string | null;
}

export interface NoteChange {
  before: Note;
  after: Note;
}

export interface NoteDiff {
  added: Note[];
  removed: Note[];
  changed: NoteChange[];
}

export interface ToolStep {
  tool: string;
  input: unknown;
  result: string;
  ok: boolean;
}

export interface Usage {
  input_tokens: number;
  output_tokens: number;
}

/**
 * A proposed edit. Deliberately does not contain the transaction — that stays in Rust,
 * so the webview can look at a change but cannot construct one.
 */
/** Where an AI edit is aimed. Mirrors `AiTarget` in src-tauri/src/ai.rs. */
export type AiTarget = "this_track" | "new_track";

export interface AiProposal {
  diff: NoteDiff;
  preview_notes: Note[];
  narration: string;
  steps: ToolStep[];
  usage: Usage;
  truncated: boolean;
  summary: string;
  empty: boolean;
}

// --- Audio to MIDI (phase 7) -----------------------------------------------

export interface CaptureStatus {
  recording: boolean;
  seconds: number;
  sample_rate: number;
  /** Peak level since the last poll, 0–1. */
  level: number;
  at_limit: boolean;
}

export interface DetectedNote {
  note: Note;
  /** Where it was played, before quantisation. */
  start_seconds: number;
  duration_seconds: number;
  confidence: number;
  /** Distance from equal temperament, in cents. */
  cents_off: number;
}

/** One analysis frame — the evidence the transcription editor draws. */
export interface Frame {
  frequency: number;
  /** Fractional MIDI note, so the pitch line sits where it was measured. */
  midi: number;
  confidence: number;
  level: number;
}

export interface Analysis {
  frames: Frame[];
  /** Frame indices where an attack was detected. Note edges snap to these. */
  onsets: number[];
  hop_seconds: number;
  silence_floor: number;
}

export interface TranscriptionPreview {
  notes: DetectedNote[];
  tempo_bpm: number;
  tempo_estimated: boolean;
  tempo_confidence: number;
  duration_seconds: number;
  pitched_fraction: number;
  warning: string | null;
  analysis: Analysis;
  use_project_tempo: boolean;
  quantize_ticks: number;
}

/** Min/max pairs for drawing a waveform. */
export type WaveformPeaks = [number, number][];

/**
 * What playing a take back should sound. Mirrors `AuditionSource` in
 * `src-tauri/src/audition.rs`.
 *
 * `midi` is the default: the notes are the thing being judged, and the recording is what
 * you compare them against rather than the other way round.
 */
export type AuditionSource = "midi" | "take" | "both";

/** Mirrors `BuildInfo` in unplugged-core. */
export interface BuildInfo {
  version: string;
  commit: string;
  /** True when the build came from a working tree with uncommitted changes. */
  dirty: boolean;
  built_at: string;
  profile: string;
}

export const MIN_TEMPO = 20;
export const MAX_TEMPO = 300;
export const DEFAULT_TEMPO = 120;
export const DEFAULT_PPQ = 480;
