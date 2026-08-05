/**
 * Browser-preview stand-in for the Rust editor and transport.
 *
 * Same caveat as `mockBackend`: this exists only so the UI can be exercised without a
 * Mac, and it is unreachable inside Tauri. The authoritative implementation — and the
 * tested one — is `crates/unplugged-core/src/command.rs`.
 *
 * It mirrors the Rust semantics closely enough to be a fair test of the UI: sorted note
 * lists, inverse-based undo, whole-transaction rollback. It does not produce sound; the
 * spec rules out Web Audio for playback, so the preview playhead moves silently.
 */

import type { EditorState, EditRequest, Note, Track, TransportState } from "./types";

type Command =
  | { op: "insert"; track: number; notes: Note[] }
  | { op: "delete"; track: number; indices: number[] }
  | { op: "replace"; track: number; indices: number[]; notes: Note[] };

interface Transaction {
  label: string;
  commands: Command[];
}

const orderKey = (n: Note): number => n.start_ticks * 1000 + n.pitch;
const sortNotes = (notes: Note[]): Note[] => [...notes].sort((a, b) => orderKey(a) - orderKey(b));
const sameNote = (a: Note, b: Note): boolean =>
  a.pitch === b.pitch &&
  a.start_ticks === b.start_ticks &&
  a.duration_ticks === b.duration_ticks &&
  a.velocity === b.velocity &&
  a.channel === b.channel;

export class MockEditor {
  tracks: Track[];
  private undoStack: Transaction[] = [];
  private redoStack: Transaction[] = [];
  private undoLabels: string[] = [];
  private redoLabels: string[] = [];
  dirty = false;
  lastAffected: number[] = [];
  lastTrack = 0;

  constructor(tracks: Track[]) {
    this.tracks = tracks.map((t) => ({ ...t, notes: sortNotes(t.notes) }));
  }

  state(): EditorState {
    return {
      tracks: this.tracks,
      can_undo: this.undoStack.length > 0,
      can_redo: this.redoStack.length > 0,
      undo_label: this.undoLabels[this.undoLabels.length - 1] ?? null,
      redo_label: this.redoLabels[this.redoLabels.length - 1] ?? null,
      history: [...this.undoLabels],
      redo_history: [...this.redoLabels],
      dirty: this.dirty,
      affected: this.lastAffected,
      affected_track: this.lastTrack,
    };
  }

  private indicesOf(track: number, notes: Note[]): number[] {
    const haystack = this.tracks[track]!.notes;
    const used = new Array(haystack.length).fill(false);
    const out: number[] = [];
    for (const note of notes) {
      const index = haystack.findIndex((c, i) => !used[i] && sameNote(c, note));
      if (index >= 0) {
        used[index] = true;
        out.push(index);
      }
    }
    return out.sort((a, b) => a - b);
  }

  private applyCommand(command: Command): Command {
    const track = this.tracks[command.track]!;

    if (command.op === "insert") {
      track.notes = sortNotes([...track.notes, ...command.notes]);
      return { op: "delete", track: command.track, indices: this.indicesOf(command.track, command.notes) };
    }

    if (command.op === "delete") {
      const descending = [...new Set(command.indices)].sort((a, b) => b - a);
      const removed: Note[] = [];
      for (const index of descending) removed.push(track.notes.splice(index, 1)[0]!);
      removed.reverse();
      return { op: "insert", track: command.track, notes: removed };
    }

    // replace — remove then re-insert, since pitch/start changes reorder the list.
    const originals = command.indices.map((i) => track.notes[i]!);
    const descending = command.indices
      .map((index, i) => ({ index, note: command.notes[i]! }))
      .sort((a, b) => b.index - a.index);
    for (const { index } of descending) track.notes.splice(index, 1);
    track.notes = sortNotes([...track.notes, ...descending.map((d) => d.note)]);

    return {
      op: "replace",
      track: command.track,
      indices: this.indicesOf(command.track, command.notes),
      notes: originals,
    };
  }

  private applyTransaction(tx: Transaction): Transaction {
    // Validate everything up front so a bad command cannot half-apply.
    for (const command of tx.commands) {
      const track = this.tracks[command.track];
      if (!track) throw { code: "track_not_found", message: `no track ${command.track}` };
      const indices = command.op === "insert" ? [] : command.indices;
      for (const index of indices) {
        if (index < 0 || index >= track.notes.length) {
          throw { code: "invalid_note", message: `note index ${index} out of range` };
        }
      }
      if (command.op === "replace" && command.indices.length !== command.notes.length) {
        throw { code: "invalid_note", message: "replace needs one note per index" };
      }
    }

    const inverses = tx.commands.map((c) => this.applyCommand(c));
    inverses.reverse();
    return { label: tx.label, commands: inverses };
  }

  private commit(tx: Transaction): void {
    if (tx.commands.length === 0) {
      this.lastAffected = [];
      return;
    }
    const inverse = this.applyTransaction(tx);
    this.undoStack.push(inverse);
    this.undoLabels.push(tx.label);
    this.redoStack = [];
    this.redoLabels = [];
    this.dirty = true;

    const affected: number[] = [];
    for (const command of tx.commands) {
      if (command.op !== "delete") affected.push(...this.indicesOf(command.track, command.notes));
    }
    this.lastAffected = [...new Set(affected)].sort((a, b) => a - b);
    this.lastTrack = tx.commands[0]!.track;
  }

  apply(request: EditRequest): EditorState {
    const tx = this.toTransaction(request);
    this.commit(tx);
    return this.state();
  }

  private toTransaction(request: EditRequest): Transaction {
    switch (request.kind) {
      case "insert":
        return { label: "Insert note", commands: [{ op: "insert", track: request.track, notes: [request.note] }] };

      case "delete":
        return { label: "Delete notes", commands: [{ op: "delete", track: request.track, indices: request.indices }] };

      case "move": {
        const notes = request.indices.map((i) => {
          const note = { ...this.tracks[request.track]!.notes[i]! };
          note.start_ticks = Math.max(0, note.start_ticks + request.delta_ticks);
          note.pitch = Math.min(127, Math.max(0, note.pitch + request.delta_pitch));
          return note;
        });
        return { label: "Move notes", commands: [{ op: "replace", track: request.track, indices: request.indices, notes }] };
      }

      case "resize": {
        const notes = request.indices.map((i) => {
          const note = { ...this.tracks[request.track]!.notes[i]! };
          note.duration_ticks = Math.max(1, note.duration_ticks + request.delta_ticks);
          return note;
        });
        return { label: "Resize notes", commands: [{ op: "replace", track: request.track, indices: request.indices, notes }] };
      }

      case "set_velocity": {
        const velocity = Math.min(127, Math.max(1, request.velocity));
        const notes = request.indices.map((i) => ({ ...this.tracks[request.track]!.notes[i]!, velocity }));
        return { label: "Set velocity", commands: [{ op: "replace", track: request.track, indices: request.indices, notes }] };
      }

      case "quantize": {
        if (request.grid_ticks <= 0) return { label: "Quantize", commands: [] };
        const notes = request.indices.map((i) => {
          const note = { ...this.tracks[request.track]!.notes[i]! };
          note.start_ticks = Math.round(note.start_ticks / request.grid_ticks) * request.grid_ticks;
          return note;
        });
        return { label: "Quantize", commands: [{ op: "replace", track: request.track, indices: request.indices, notes }] };
      }

      case "join": {
        const selected = [...new Set(request.indices)]
          .filter((i) => this.tracks[request.track]?.notes[i])
          .sort((a, b) => a - b);
        if (selected.length < 2) return { label: "Join notes", commands: [] };

        const notes = selected.map((i) => this.tracks[request.track]!.notes[i]!);
        const first = notes.reduce((a, b) => (orderKey(a) <= orderKey(b) ? a : b));
        const start = Math.min(...notes.map((n) => n.start_ticks));
        const end = Math.max(...notes.map((n) => n.start_ticks + n.duration_ticks));

        return {
          label: "Join notes",
          commands: [
            { op: "delete", track: request.track, indices: selected },
            {
              op: "insert",
              track: request.track,
              notes: [{ ...first, start_ticks: start, duration_ticks: Math.max(1, end - start) }],
            },
          ],
        };
      }

      case "paste": {
        if (request.notes.length === 0) return { label: "Paste", commands: [] };
        const earliest = Math.min(...request.notes.map((n) => n.start_ticks));
        const notes = request.notes.map((n) => ({
          ...n,
          start_ticks: request.at_ticks + (n.start_ticks - earliest),
        }));
        return { label: "Paste", commands: [{ op: "insert", track: request.track, notes }] };
      }
    }
  }

  undo(): EditorState {
    const inverse = this.undoStack.pop();
    if (!inverse) return this.state();
    const label = this.undoLabels.pop() ?? "";
    const redo = this.applyTransaction(inverse);
    this.redoStack.push(redo);
    this.redoLabels.push(label);
    this.dirty = true;
    this.lastAffected = [];
    return this.state();
  }

  redo(): EditorState {
    const tx = this.redoStack.pop();
    if (!tx) return this.state();
    const label = this.redoLabels.pop() ?? "";
    const inverse = this.applyTransaction(tx);
    this.undoStack.push(inverse);
    this.undoLabels.push(label);
    this.dirty = true;
    this.lastAffected = [];
    return this.state();
  }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

type PlayheadListener = (event: { position_ticks: number; playing: boolean; in_count_in: boolean }) => void;

/** Silent playhead simulation. No Web Audio — the spec rules it out for playback. */
export class MockTransport {
  private playing = false;
  private positionTicks = 0;
  private tempoBpm = 120;
  private ppq = 480;
  private loopRegion: [number, number] | null = null;
  private timer: number | null = null;
  private lastTickMs = 0;
  private listeners = new Set<PlayheadListener>();

  setPpq(ppq: number): void {
    this.ppq = ppq;
  }

  state(): TransportState {
    return {
      playing: this.playing,
      position_ticks: Math.floor(this.positionTicks),
      tempo_bpm: this.tempoBpm,
      loop_region: this.loopRegion,
      engine_running: this.playing,
      sample_rate: 48000,
    };
  }

  onPlayhead(listener: PlayheadListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(): void {
    const event = {
      position_ticks: Math.floor(this.positionTicks),
      playing: this.playing,
      // The preview has no count-in; recording is Rust's job.
      in_count_in: false,
    };
    this.listeners.forEach((l) => l(event));
  }

  play(): TransportState {
    if (this.playing) return this.state();
    this.playing = true;
    this.lastTickMs = performance.now();

    this.timer = window.setInterval(() => {
      const now = performance.now();
      const elapsedMs = now - this.lastTickMs;
      this.lastTickMs = now;

      const ticksPerMs = (this.tempoBpm * this.ppq) / 60000;
      this.positionTicks += elapsedMs * ticksPerMs;

      if (this.loopRegion && this.positionTicks >= this.loopRegion[1]) {
        this.positionTicks = this.loopRegion[0];
      }
      this.emit();
    }, 1000 / 30);

    this.emit();
    return this.state();
  }

  stop(): TransportState {
    this.playing = false;
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
    this.emit();
    return this.state();
  }

  seek(tick: number): TransportState {
    this.positionTicks = Math.max(0, tick);
    this.emit();
    return this.state();
  }

  setTempo(bpm: number): TransportState {
    this.tempoBpm = bpm;
    return this.state();
  }

  setLoopRegion(region: [number, number] | null): TransportState {
    this.loopRegion = region && region[1] > region[0] ? region : null;
    return this.state();
  }
}
