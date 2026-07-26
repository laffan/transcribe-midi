/**
 * The app's error console (the optional panel beneath the transport).
 *
 * A tiny external store rather than React context: things that need to log — including
 * non-React code, and from Phase 2 the Rust event listeners — should not have to be
 * inside a provider to do it.
 */

import { useSyncExternalStore } from "react";

export type LogLevel = "info" | "warn" | "error";

export interface LogEntry {
  id: number;
  level: LogLevel;
  message: string;
  detail?: string;
  at: number;
}

const MAX_ENTRIES = 500;

let entries: LogEntry[] = [];
let nextId = 1;
const listeners = new Set<() => void>();

function emit(): void {
  // New array identity each time; useSyncExternalStore compares by reference.
  listeners.forEach((l) => l());
}

export function log(level: LogLevel, message: string, detail?: string): void {
  entries = [{ id: nextId++, level, message, detail, at: Date.now() }, ...entries].slice(0, MAX_ENTRIES);
  emit();
}

export const logger = {
  info: (message: string, detail?: string) => log("info", message, detail),
  warn: (message: string, detail?: string) => log("warn", message, detail),
  error: (message: string, detail?: string) => log("error", message, detail),
};

export function clearLog(): void {
  entries = [];
  emit();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): LogEntry[] {
  return entries;
}

export function useLog(): LogEntry[] {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
