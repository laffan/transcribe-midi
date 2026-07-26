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
  CommandError,
  EditorState,
  EditRequest,
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

  live_note_on(_args: { track: number; pitch: number; velocity: number; channel: number }): void {},
  live_note_off(_args: { track: number; pitch: number; channel: number }): void {},
  panic_all_notes_off(): void {},
};

let editor: MockEditor | null = null;
let openId: string | null = null;
export const mockTransport = new MockTransport();
const transport = mockTransport;

function requireEditor(): MockEditor {
  if (!editor) fail("internal", "no project is open");
  return editor;
}
