/**
 * The only place the frontend talks to Rust.
 *
 * Every function here is a typed wrapper over one `#[tauri::command]`. Keeping the
 * `invoke` string literals in a single file means a renamed command is one edit, and it
 * gives the browser-preview fallback exactly one place to hook into.
 */

import { invoke } from "@tauri-apps/api/core";

import { mockBackend } from "./mockBackend";
import type {
  CommandError,
  Project,
  ProjectListing,
  ProjectManifest,
  TimeSignature,
  TrackMeta,
} from "./types";

/** True when running inside a Tauri webview rather than a plain browser tab. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as CommandError).code === "string" &&
    typeof (value as CommandError).message === "string"
  );
}

/** Turn anything thrown by `invoke` into a message worth showing a user. */
export function errorMessage(error: unknown): string {
  if (isCommandError(error)) return error.message;
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Unexpected error";
}

type MockFn = (args: never) => unknown;

async function call<T>(command: keyof typeof mockBackend, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    // Browser preview. Async so the call signature matches the real thing and no
    // component accidentally depends on synchronous resolution.
    return (mockBackend[command] as MockFn)(args as never) as T;
  }
  return invoke<T>(command, args);
}

export const api = {
  listProjects: () => call<ProjectListing>("list_projects"),

  createProject: (name: string, tempoBpm: number, timeSignature: TimeSignature) =>
    call<ProjectManifest>("create_project", { name, tempoBpm, timeSignature }),

  loadProject: (id: string) => call<Project>("load_project", { id }),

  saveProject: (project: Project) => call<ProjectManifest>("save_project", { project }),

  renameProject: (id: string, name: string) => call<ProjectManifest>("rename_project", { id, name }),

  deleteProject: (id: string) => call<void>("delete_project", { id }),

  addTrack: (projectId: string, name?: string) =>
    call<TrackMeta>("add_track", { projectId, name: name ?? null }),

  deleteTrack: (projectId: string, trackId: string) =>
    call<void>("delete_track", { projectId, trackId }),

  projectsRoot: () => call<string>("projects_root"),
};
