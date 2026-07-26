# Unplugged

A MIDI-sequencing DAW with AI-assisted editing, audio-to-MIDI transcription, a notation
view, and AUv3 instrument hosting. macOS (aarch64) and iOS.

> **Status: Phase 3 of 9.** Project CRUD, the audio engine and transport, the piano roll
> and the undoable command layer are built. External MIDI input, import/export, AI
> editing, transcription, notation and AUv3 hosting are not — each unbuilt panel is
> labelled in the UI with the phase that fills it in.
>
> **The Swift audio layer has never been compiled.** It was written on a Linux host with
> no Xcode; see [DECISIONS.md](./DECISIONS.md) for exactly what that leaves unverified.

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri 2 |
| Backend | Rust |
| Frontend | TypeScript + React + Vite, hand-rolled CSS |
| MIDI ports | `midir` (CoreMIDI) |
| SMF read/write | `midly` |
| Audio | AVAudioEngine via a Swift plugin (Phase 2) |

All MIDI I/O and audio synthesis live in Rust and Swift. WKWebView has no Web MIDI API
on either target and its Web Audio implementation is not suitable for timing-critical
work, so `navigator.requestMIDIAccess` and Web Audio are not used for playback.

See [DECISIONS.md](./DECISIONS.md) for the dependency verification results and the
reasoning behind the architecture — including one place where the original spec is not
achievable as written, and what was built instead.

## Layout

```
crates/unplugged-core/   Domain model, sequencer, command layer, SMF, persistence.
                         Pure Rust, no Tauri, no platform code.
crates/unplugged-audio/  Audio binding: one Rust-facing API, C ABI to Swift,
                         null backend off-Apple.
swift/UnpluggedAudio/    AVAudioEngine graph + render callback. One package, both targets.
src-tauri/               Tauri app: thin command wrappers.
src/                     React frontend.
  lib/                   Typed API layer, theme, console store.
  features/projects/     Project picker (the launch screen).
  features/editor/       Piano roll, transport, on-screen keyboard, console.
  features/settings/     Settings modal.
  styles/tokens.css      Design tokens — the single source of truth for colour and type.
```

Logic lives in `unplugged-core` on purpose. It has no platform dependency, so it
compiles for both Apple targets and its tests run anywhere; `src-tauri` stays thin
because it is the part that cannot be compile-checked without a Mac.

## Development

```bash
npm install
npm run tauri dev        # requires macOS + Xcode
```

Without a Mac you can still work on the UI:

```bash
npm run dev              # http://localhost:1420
```

In a plain browser the Rust backend is unavailable, so the frontend falls back to an
in-memory mock (`src/lib/mockBackend.ts`) backed by localStorage. It exists only to make
the UI clickable — it is not a second implementation of the domain, and it is
unreachable inside Tauri.

### Checks

```bash
cargo test --workspace          # 89 tests (75 core + 14 audio)
cargo clippy --workspace --all-targets
npm run build                   # tsc --noEmit && vite build

# Compile-verify the Apple targets (no linking, but catches API breakage)
cargo check -p unplugged-core -p unplugged-audio --target aarch64-apple-darwin
cargo check -p unplugged-core -p unplugged-audio --target aarch64-apple-ios
```

## Editor shortcuts

| | |
|---|---|
| `Space` | Play / stop |
| `⌘Z` / `⇧⌘Z` | Undo / redo |
| `⌘A` | Select all |
| `⌘C` / `⌘X` / `⌘V` | Copy / cut / paste at playhead |
| `⌘Q` | Quantize selection to the grid |
| `⌫` | Delete selection |
| `↑ ↓ ← →` | Nudge (`⇧` for an octave / a bar) |
| `⌥`-click | Delete a note |
| `A`–`L`, `W/E/T/Y/U` | Play the on-screen keyboard |
| `Z` / `X` | Octave down / up |

Click empty grid to draw a note; drag to marquee-select. `⌘`-scroll zooms about the
pointer, `⇧`-scroll pans horizontally.

## Project format

A project is a directory under the app data dir:

```
<app_data>/projects/<project-id>/
  project.json          name, tempo, time signature, PPQ, track list, per-track settings
  tracks/<track-id>.mid one SMF per track (format 0, notes only)
```

`project.json` is authoritative for tempo, time signature and PPQ — the per-track MIDI
files deliberately do not duplicate them. Writes are atomic (temp file + rename), so an
interrupted save leaves the previous project intact.

## Out of scope

Recorded audio tracks (mic capture exists only to feed transcription), notation
*editing*, VST3 and CLAP hosting, Android, Windows, Linux, collaboration, cloud sync.
