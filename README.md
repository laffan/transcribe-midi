# Unplugged

Unplugged is about two things:

1. **Turning what you play into MIDI.** Sing, hum or play a line; get notes.
2. **Changing that MIDI by describing what you want.** "Harmonise a third above",
   "make it swing", "add a ii–V–I under this".

Everything else — the piano roll, the transport, the file handling, the sampler — exists
to serve those two, and to let you check their work. macOS (aarch64) and iOS, standalone
and (from Phase 9) as an AUv3 plugin inside Logic Pro and Ableton Live.

The piano roll is deliberately not the product. Both of the things above produce notes you
did not type, so you need somewhere to *see* what arrived and fix the last five percent —
that is what the editor is for. Every AI edit and every transcription is previewed as a
diff before it is applied, and lands as a single undo step, for the same reason.

> **Status: Phase 7 of 9.** Project CRUD, the audio engine and transport, the piano roll
> and undoable command layer, external MIDI input with loop recording, SMF
> import/export/sharing, AI-assisted editing and monophonic audio-to-MIDI are built.
>
> The two headline features currently sit at the bottom of a scrolling sidebar, which is
> the wrong emphasis for an app that is *about* them. Phase 8 is a re-centring rather than
> a new feature — see [DECISIONS.md](./DECISIONS.md).
>
> The audio engine has been verified making sound on real hardware. Nothing else written
> in Swift has been compiled — MIDI input, the share sheet, drag-out, the Keychain and
> microphone capture are all unverified — and no request has ever been made to the
> Anthropic API from this code. See [DECISIONS.md](./DECISIONS.md) for exactly what that
> leaves untested and what a human needs to check.

## Turning audio into MIDI

Play or hum **one note at a time** and the line becomes notes. Polyphonic transcription is
out of scope in this version: it is a different problem, and a pitch tracker handed a chord
returns confident nonsense rather than a chord.

The pipeline is YIN pitch detection, spectral-flux onsets and autocorrelation tempo
estimation, all hand-written and tested against synthetic signals.

Phase 9 turns this into an editor rather than a one-shot: the take is kept, drawn as a
waveform with the measured pitch traced over it, and the detected notes sit on top as boxes
you can drag in pitch and length — so a wrong note is corrected against the evidence rather
than by ear against a grid. That also makes the settings re-derivable: changing the grid or
the tempo re-reads the same take instead of asking you to play it again.

## Changing MIDI by describing it

Describe an edit and the change previews as a diff before it is applied —
green is new, red is going, amber is moving. Accept and it becomes one undo step.

The model never touches note data directly. It calls a closed set of sixteen tools against
a scratch copy of the track; the difference between that copy and the original becomes a
single transaction through the same command layer as a mouse drag. Anything it invents
fails at the tool boundary and comes back to it as an error.

The Anthropic API key lives in the platform Keychain. Every request originates in Rust —
the key is read immediately before a call and dropped after, and the interface can learn
only that a key exists and its last four characters. The model list is fetched from
`GET /v1/models` at runtime rather than hardcoded.

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri 2 |
| Backend | Rust |
| Frontend | TypeScript + React + Vite, hand-rolled CSS |
| MIDI ports | `midir` (CoreMIDI) |
| SMF read/write | `midly` |
| Audio | AVAudioEngine via a Swift plugin (Phase 2) |
| AI | Anthropic Messages API over raw HTTP, from Rust |
| Transcription | Hand-written DSP (YIN, spectral flux), no dependencies |

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
crates/unplugged-midi/   External MIDI input (CoreMIDI), null backend off-Apple.
crates/unplugged-ai/     Anthropic client and tool loop. The API key never leaves
                         this crate.
crates/unplugged-transcribe/
                         Monophonic audio-to-MIDI. Pure DSP, no audio I/O.
swift/UnpluggedAudio/    AVAudioEngine graph + render callback, microphone capture,
                         and the platform surface (share sheet, pasteboard,
                         drag-out, Keychain). One package, both targets.
src-tauri/               Tauri app: thin command wrappers.
src/                     React frontend.
  lib/                   Typed API layer, theme, console store.
  features/projects/     Project picker (the launch screen).
  features/editor/       Piano roll, transport, on-screen keyboard, AI panel,
                         transcription panel, console.
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
cargo test --workspace          # 262 tests
cargo clippy --workspace --all-targets
npm run build                   # tsc --noEmit && vite build

# Compile-verify the Apple targets (no linking, but catches API breakage)
cargo check --workspace --exclude unplugged --target aarch64-apple-darwin
cargo check --workspace --exclude unplugged --target aarch64-apple-ios
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
| `R` | Record MIDI |
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

Phase 9 adds a `takes/` directory: the audio behind a transcription, kept so the waveform
editor has something to draw and so the transcription settings stay re-derivable. It is
evidence attached to a take, not a track in the arrangement.

## Out of scope

**Hosting** other people's plugins. The original plan had Unplugged host AUv3 instruments;
that is inverted — Unplugged becomes the plugin. See [DECISIONS.md](./DECISIONS.md).

Also out: recorded audio *tracks* — audio is kept as evidence for a transcription, never as
material in the arrangement, so it is not mixed, bounced or exported. Also out: notation
*editing*, polyphonic transcription, Android, collaboration, cloud sync. Windows and Linux
are out for as long as the plugin target is AUv3 only; a VST3 build would change that, and
is deferred rather than refused.
