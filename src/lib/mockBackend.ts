/**
 * In-memory stand-in for the Rust backend, used only when the app is opened in a plain
 * browser (`npm run dev` without Tauri).
 *
 * Why this exists: the real backend only runs on macOS and iOS, so without it there is
 * no way to iterate on UI without a Mac in hand. It is *not* a second implementation of
 * the domain — it exists to make the picker clickable, and it deliberately does not try
 * to be faithful beyond that. Anything that matters lives in Rust and is tested there.
 *
 * It is unreachable inside Tauri: `isTauri()` gates every call site.
 */

import { MockEditor, MockTransport } from "./mockEditor";
import type {
  AiModelsResponse,
  CaptureStatus,
  TranscriptionPreview,
  WaveformPeaks,
  AiProposal,
  AiStatus,
  BuildInfo,
  CommandError,
  EditorState,
  EditRequest,
  ExportPayload,
  ImportPreview,
  ImportResult,
  InputSettings,
  MidiPort,
  Note,
  PlatformCapabilities,
  RecordResult,
  Project,
  ProjectListing,
  ProjectManifest,
  TimeSignature,
  Track,
  TrackMeta,
  TransportState,
} from "./types";
import { DEFAULT_PPQ } from "./types";

const SCHEMA_VERSION = 1;
const STORAGE_KEY = "unplugged.mock.projects";

const TRACK_COLORS = [
  "#5b8dd9", "#d97757", "#6cb08a", "#c07ac0",
  "#d9a441", "#5aa8b0", "#b06a6a", "#8a86d9",
];

type Store = Record<string, Project>;

function fail(code: string, message: string): never {
  throw { code, message } satisfies CommandError;
}

function read(): Store {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Store;
  } catch {
    return {};
  }
}

function write(store: Store): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
}

/** Mirrors `store::slugify` closely enough for ids to look right in the UI. */
function slugify(name: string): string {
  const slug = name
    .split("")
    .map((c) => (/[a-zA-Z0-9]/.test(c) ? c.toLowerCase() : "-"))
    .join("")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 64)
    .replace(/-$/, "");
  return slug || "project";
}

function uniqueId(store: Store, base: string): string {
  if (!store[base]) return base;
  for (let n = 2; n < 10000; n += 1) {
    if (!store[`${base}-${n}`]) return `${base}-${n}`;
  }
  return `${base}-${Date.now()}`;
}

function newTrack(index: number, ppq: number): Track {
  return {
    id: `track-${index + 1}`,
    name: `Track ${index + 1}`,
    channel: index % 16,
    instrument: { kind: "built_in_sampler" },
    muted: false,
    soloed: false,
    color: TRACK_COLORS[index % TRACK_COLORS.length]!,
    key_hint: null,
    notes: [],
    ppq,
  };
}

function metaOf(track: Track): TrackMeta {
  const { notes: _notes, ppq: _ppq, ...meta } = track;
  return meta;
}

export const mockBackend = {
  list_projects(): ProjectListing {
    const store = read();
    const projects = Object.values(store)
      .map((p) => ({
        id: p.manifest.id,
        name: p.manifest.name,
        tempo_bpm: p.manifest.tempo_bpm,
        time_signature: p.manifest.time_signature,
        track_count: p.manifest.tracks.length,
        created_at_ms: p.manifest.created_at_ms,
        modified_at_ms: p.manifest.modified_at_ms,
      }))
      .sort((a, b) => b.modified_at_ms - a.modified_at_ms || a.name.localeCompare(b.name));
    return { projects, errors: [] };
  },

  create_project(args: { name: string; tempoBpm: number; timeSignature: TimeSignature }): ProjectManifest {
    const name = args.name.trim();
    if (!name) fail("invalid_name", "project name cannot be empty");
    if (args.tempoBpm < 20 || args.tempoBpm > 300 || !Number.isFinite(args.tempoBpm)) {
      fail("invalid_tempo", `tempo ${args.tempoBpm} bpm is out of range (20–300)`);
    }

    const store = read();
    const id = uniqueId(store, slugify(name));
    const now = Date.now();
    const track = newTrack(0, DEFAULT_PPQ);

    const manifest: ProjectManifest = {
      schema_version: SCHEMA_VERSION,
      id,
      name,
      tempo_bpm: args.tempoBpm,
      time_signature: args.timeSignature,
      ppq: DEFAULT_PPQ,
      tracks: [metaOf(track)],
      created_at_ms: now,
      modified_at_ms: now,
    };

    store[id] = { manifest, tracks: [track] };
    write(store);
    return manifest;
  },

  load_project(args: { id: string }): Project {
    const project = read()[args.id];
    if (!project) fail("project_not_found", `no project with id '${args.id}'`);
    return project;
  },

  save_project(args: { project: Project }): ProjectManifest {
    const store = read();
    const next = structuredClone(args.project);
    next.manifest.modified_at_ms = Date.now();
    store[next.manifest.id] = next;
    write(store);
    return next.manifest;
  },

  rename_project(args: { id: string; name: string }): ProjectManifest {
    const name = args.name.trim();
    if (!name) fail("invalid_name", "project name cannot be empty");

    const store = read();
    const project = store[args.id];
    if (!project) fail("project_not_found", `no project with id '${args.id}'`);

    project.manifest.name = name;
    project.manifest.modified_at_ms = Date.now();
    write(store);
    return project.manifest;
  },

  delete_project(args: { id: string }): void {
    const store = read();
    if (!store[args.id]) fail("project_not_found", `no project with id '${args.id}'`);
    delete store[args.id];
    write(store);
  },

  add_track(args: { projectId: string; name?: string | null }): TrackMeta {
    const store = read();
    const project = store[args.projectId];
    if (!project) fail("project_not_found", `no project with id '${args.projectId}'`);

    const track = newTrack(project.tracks.length, project.manifest.ppq);
    while (project.tracks.some((t) => t.id === track.id)) {
      track.id = `${track.id}-${project.tracks.length + 1}`;
    }
    if (args.name?.trim()) track.name = args.name.trim();

    project.tracks.push(track);
    project.manifest.tracks.push(metaOf(track));
    project.manifest.modified_at_ms = Date.now();
    write(store);
    return metaOf(track);
  },

  delete_track(args: { projectId: string; trackId: string }): void {
    const store = read();
    const project = store[args.projectId];
    if (!project) fail("project_not_found", `no project with id '${args.projectId}'`);

    const index = project.tracks.findIndex((t) => t.id === args.trackId);
    if (index < 0) fail("track_not_found", `no track with id '${args.trackId}'`);

    project.tracks.splice(index, 1);
    project.manifest.tracks.splice(index, 1);
    project.manifest.modified_at_ms = Date.now();
    write(store);
  },

  projects_root(): string {
    return "(browser preview — projects are in localStorage, not on disk)";
  },

  build_info: (): BuildInfo => ({
    version: "0.0.0",
    commit: "preview",
    dirty: false,
    built_at: "",
    profile: "browser",
  }),

  // --- Editor -------------------------------------------------------------

  open_project(args: { id: string }): EditorState {
    const store = read();
    const project = store[args.id];
    if (!project) fail("project_not_found", `no project with id '${args.id}'`);

    openId = args.id;
    editor = new MockEditor(project.tracks);
    transport.setPpq(project.manifest.ppq);
    transport.setTempo(project.manifest.tempo_bpm);
    return editor.state();
  },

  close_project(): void {
    transport.stop();
    editor = null;
    openId = null;
  },

  save_open_project(): EditorState {
    const e = requireEditor();
    const store = read();
    const project = store[openId!];
    if (project) {
      project.tracks = e.tracks;
      project.manifest.tracks = e.tracks.map((t) => metaOf(t));
      project.manifest.modified_at_ms = Date.now();
      write(store);
    }
    e.dirty = false;
    return e.state();
  },

  editor_state(): EditorState {
    return requireEditor().state();
  },

  apply_edit(args: { request: EditRequest }): EditorState {
    return requireEditor().apply(args.request);
  },

  undo(): EditorState {
    return requireEditor().undo();
  },

  redo(): EditorState {
    return requireEditor().redo();
  },

  // --- Transport ----------------------------------------------------------

  transport_play: (): TransportState => transport.play(),
  transport_stop: (): TransportState => transport.stop(),
  transport_seek: (args: { tick: number }): TransportState => transport.seek(args.tick),
  transport_get: (): TransportState => transport.state(),
  set_tempo: (args: { bpm: number }): TransportState => transport.setTempo(args.bpm),
  set_loop_region: (args: { region: [number, number] | null }): TransportState =>
    transport.setLoopRegion(args.region),

  // --- Live input ---------------------------------------------------------
  // Silent by design: the spec rules out Web Audio for playback, so the preview
  // shows keys lighting up without pretending to be an instrument.

  live_note_on(_args: { pitch: number; velocity: number | null; channel: number }): void {},
  live_note_off(_args: { pitch: number; channel: number }): void {},
  panic_all_notes_off(): void {},

  // --- Input and recording -------------------------------------------------
  //
  // No MIDI hardware in a browser tab, so ports are always empty. Recording state is
  // tracked so the transport UI can be exercised; nothing is actually captured.

  input_settings: (): InputSettings => ({ ...inputSettings, ports: [], connected: null }),
  midi_ports: (): MidiPort[] => [],
  midi_connect(_args: { portId: string }): MidiPort {
    return fail("internal", "MIDI input is unavailable in the browser preview");
  },
  midi_disconnect(): void {},
  midi_set_channel(args: { channel: number | null }): void {
    inputSettings.channel = args.channel;
  },
  set_armed_track(args: { track: number }): void {
    inputSettings.armed_track = args.track;
  },
  set_keyboard_velocity(args: { velocity: number }): void {
    inputSettings.keyboard_velocity = args.velocity;
  },
  set_count_in_bars(args: { bars: number }): void {
    inputSettings.count_in_bars = args.bars;
  },
  set_metronome(args: { enabled: boolean }): void {
    inputSettings.metronome = args.enabled;
  },
  record_start(): RecordResult {
    inputSettings.recording = true;
    transport.play();
    return { recording: true, count_in_ticks: 0, captured_notes: 0 };
  },
  record_stop(): EditorState {
    inputSettings.recording = false;
    transport.stop();
    return requireEditor().state();
  },
  record_cancel(): void {
    inputSettings.recording = false;
    transport.stop();
  },

  // --- Interchange ---------------------------------------------------------

  export_project_smf(): ExportPayload {
    const e = requireEditor();
    const count = e.tracks.reduce((n, t) => n + t.notes.length, 0);
    // The preview cannot produce real SMF — that lives in Rust. A placeholder keeps the
    // export flow clickable and is clearly labelled as such if it ever reaches disk.
    return { filename: "preview.mid", bytes: Array.from(`unplugged-preview:${count}`, (c) => c.charCodeAt(0)) };
  },
  export_track_smf(args: { track: number }): ExportPayload {
    const e = requireEditor();
    const t = e.tracks[args.track];
    return {
      filename: `${t?.name ?? "track"}.mid`,
      bytes: Array.from(`unplugged-preview:${t?.notes.length ?? 0}`, (c) => c.charCodeAt(0)),
    };
  },
  write_export(args: { path: string }): string {
    return args.path;
  },
  stage_export(args: { filename: string }): string {
    return `/preview/${args.filename}`;
  },
  preview_import(): ImportPreview {
    return fail("internal", "import is unavailable in the browser preview");
  },
  import_smf(): ImportResult {
    return fail("internal", "import is unavailable in the browser preview");
  },
  copy_file_to_pasteboard(): void {},
  share_file(): void {
    fail("internal", "sharing is unavailable in the browser preview");
  },
  begin_file_drag(): void {
    fail("internal", "drag-out is unavailable in the browser preview");
  },
  platform_capabilities: (): PlatformCapabilities => ({
    share_sheet: false,
    drag_out: false,
    pasteboard: false,
    save_dialog: false,
  }),

  // --- AI ------------------------------------------------------------------
  //
  // Not stubbed with fake suggestions. Every AI request originates in Rust and needs a
  // Keychain and a real API key, neither of which a browser tab has; a mock that
  // invented a diff would only teach the UI to trust something that cannot happen.

  ai_status: (): AiStatus => ({
    has_key: false,
    key_hint: null,
    key_persists: false,
    model: null,
  }),
  ai_set_key(): AiModelsResponse {
    return fail("internal", "AI editing needs the macOS or iOS build");
  },
  ai_clear_key: (): AiStatus => ({
    has_key: false,
    key_hint: null,
    key_persists: false,
    model: null,
  }),
  ai_models(): AiModelsResponse {
    return fail("internal", "AI editing needs the macOS or iOS build");
  },
  ai_set_model(): AiStatus {
    return fail("internal", "AI editing needs the macOS or iOS build");
  },
  ai_propose(): AiProposal {
    return fail("internal", "AI editing needs the macOS or iOS build");
  },
  ai_accept(): EditorState {
    return fail("internal", "AI editing needs the macOS or iOS build");
  },
  ai_reject(): void {},

  // --- Audio to MIDI -------------------------------------------------------
  //
  // getUserMedia exists in a browser, but the analysis and the capture both live in
  // Rust — and a mock that transcribed something would be a second implementation of
  // the one part of this app that most needs a single source of truth.

  capture_start(): CaptureStatus {
    return fail("internal", "microphone capture needs the macOS or iOS build");
  },
  capture_poll: (): CaptureStatus => ({
    recording: false,
    seconds: 0,
    sample_rate: 0,
    level: 0,
    at_limit: false,
  }),
  capture_transcribe(): TranscriptionPreview {
    return fail("internal", "transcription needs the macOS or iOS build");
  },
  capture_accept(): EditorState {
    return fail("internal", "transcription needs the macOS or iOS build");
  },
  capture_cancel(): void {},
  capture_retranscribe(): TranscriptionPreview {
    return fail("internal", "transcription needs the macOS or iOS build");
  },
  capture_load_file(): TranscriptionPreview {
    return fail("internal", "transcription needs the macOS or iOS build");
  },
  capture_waveform: (): WaveformPeaks => [],
  capture_progress: (): number => 0,
  capture_set_notes(): Note[] {
    return fail("internal", "transcription needs the macOS or iOS build");
  },
  capture_audition_play(): void {
    fail("internal", "playing a take back needs the macOS or iOS build");
  },
  capture_audition_stop(): void {},
  capture_audition_position: (): number | null => null,
};

const inputSettings: InputSettings = {
  ports: [],
  connected: null,
  channel: null,
  armed_track: 0,
  keyboard_velocity: 100,
  count_in_bars: 0,
  metronome: false,
  recording: false,
};

let editor: MockEditor | null = null;
let openId: string | null = null;
export const mockTransport = new MockTransport();
const transport = mockTransport;

function requireEditor(): MockEditor {
  if (!editor) fail("internal", "no project is open");
  return editor;
}
