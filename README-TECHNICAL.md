# Unplugged — Technical README

This file is for whoever works on the code next, human or agent. It says how the project
is put together, what the standards are, and what will bite you if you skip it. The
narrative of *why* each decision was made lives in [DECISIONS.md](./DECISIONS.md); read the
section for any area before changing it.

## The one-paragraph architecture

A Tauri 2 shell around a Rust workspace, with a TypeScript/React frontend. All domain
logic lives in pure Rust crates with no platform or framework dependency, so it compiles
for macOS, iOS and the CI host alike, and its tests run anywhere. Platform work (audio,
MIDI, Keychain, share sheet) is Swift behind a C ABI. The same domain crates are linked
into an AUv3 app extension, so the plugin and the app are two thin shells over one core.

```
crates/unplugged-core/       Domain model, sequencer, command layer, SMF, persistence,
                             music theory, audition scheduling, AI tool surface. Pure
                             Rust. No Tauri, no I/O beyond the filesystem, no platform
                             code.
crates/unplugged-audio/      Audio engine binding: Rust API, C ABI to Swift, null backend
                             off-Apple so tests run anywhere.
crates/unplugged-midi/       External MIDI input (CoreMIDI via midir), null backend off-Apple.
crates/unplugged-ai/         Provider clients (Anthropic, OpenAI-compatible) + tool loop.
                             The API key never leaves this crate.
crates/unplugged-transcribe/ Audio-to-MIDI DSP (YIN, spectral flux, tempo). Pure math,
                             no audio I/O, no dependencies.
crates/unplugged-plugin/     The C ABI the AUv3 extension calls. Wraps core. No Tauri.
swift/UnpluggedAudio/        One Swift package, linked into both Apple targets:
                             AVAudioEngine graph, capture, decoding, preview, Keychain,
                             pasteboard/share/drag, group container lookup.
plugin/                      The AUv3: AUAudioUnit subclass, SwiftUI view, XcodeGen
                             project.yml (the .xcodeproj is generated, never committed).
src-tauri/                   Tauri commands. Thin: parse args, call a crate, map errors.
src/                         React frontend.
  lib/api.ts                 The ONLY place the frontend talks to the backend.
  features/<name>/           One directory per feature; components + css side by side.
  styles/tokens.css          Colour, spacing, type, chrome heights, safe-area insets,
                             and the breakpoint ladder every media query in the app uses.
  styles/touch.css           The finger layer: what changes when the pointer is coarse,
                             independently of how wide the screen is.
scripts/                     install-plugin.sh / verify-plugin.sh — build, install,
                             register and interrogate the plugin. build-ios.sh — the
                             preflight the iOS build needs before `tauri ios build`
                             means anything.
```

Dependency direction is one-way and enforced by the workspace: `unplugged-core` depends on
nothing of ours; every other crate may depend on core; `src-tauri` and
`crates/unplugged-plugin` are leaves. If you find yourself wanting core to know about
Tauri, the plugin, or Swift, the design is wrong — invert it.

## Modularity is the paradigm

Every piece of this project that has gone well went well because it was a module with a
narrow, testable seam; every bug that survived past its first build lived on a boundary
nobody had pinned down in code. So the standard is explicit:

1. **One module, one reason to change.** A file holds one concept — a pitch tracker, a
   command's inverse, a picker view. When a file needs a second heading comment to explain
   its second half, that half is a new module.

2. **Logic lives below the shell.** Anything that can be expressed without Tauri, React,
   or an Apple framework goes in a pure crate (or a pure `.ts` module) where it can be
   unit-tested on any machine. The shells — `src-tauri`, the AUv3 classes, React
   components — stay thin enough that a reviewer can hold one file in their head.

3. **Seams are typed and total.** Crate boundaries are Rust types; the FFI boundary is one
   C header (`plugin/Support/UnpluggedPluginFFI.h`) mirrored by `#[repr(C)]` structs; the
   frontend boundary is `src/lib/api.ts`. No side channels: if the frontend needs new
   data, it gets a new function in `api.ts` backed by a new command, not a fetch or a
   global.

4. **Every mutation goes through the command layer.** Mouse, keyboard, AI, transcription —
   all of them emit commands with inverses through `unplugged-core`'s command module.
   That is what makes undo, diff preview and the AI's accept/reject flow one mechanism
   instead of three. Never mutate a project behind its back.

5. **Prefer a new small crate/module over a clever addition to a big one.** `host_sync`,
   `build_info` and `shared_container` are each ~100–200 lines with exhaustive tests.
   That shape — small, pure, hammered by tests — is the target for new work.

### The 700-line rule

**No code file may exceed 700 lines.** This applies to every language in the repo — Rust,
TypeScript/TSX, Swift, CSS, shell. Tests count toward a file's total; splitting tests into
a sibling `tests.rs` / `Foo.test.ts` is an accepted and encouraged way to comply.

When a file approaches the limit, split along seams that already exist in its structure:
a Rust module becomes a directory (`ai.rs` → `ai/tools.rs`, `ai/workspace.rs`,
`ai/diff.rs`); a React component sheds subcomponents and hooks into siblings; a CSS file
splits by the component it styles. Do not comply by deleting comments or compressing
style — the limit exists to force *modularity*, not terseness.

**Current violations (debt register).** These predate the rule. Do not add to them; when
you touch one substantially, split it as part of the change:

| File | Lines | Suggested split |
|---|---|---|
| `crates/unplugged-core/src/ai.rs` | ~2030 | `ai/` dir: tool defs, workspace, transaction, rng, tests |
| `crates/unplugged-core/src/sequencer.rs` | ~1150 | scheduling vs. timeline vs. tests |
| `crates/unplugged-core/src/smf.rs` | ~860 | read vs. write vs. tests |
| `crates/unplugged-core/src/music.rs` | ~810 | scales/keys vs. roman-numeral parsing vs. tests |
| `crates/unplugged-plugin/src/lib.rs` | ~765 | plugin state vs. C ABI vs. tests |

Five files have come off this list by being touched, which is the rule working as
intended — the register is a list of files waiting for a reason to be split, not a list
of exemptions:

- **`Editor.tsx`** (830 → 625) — transport and capture state into `useTransport` and
  `usePending`, panels into `Toolbar`, `TrackList` and `Inspector`.
- **`PianoRoll.tsx`** (750 → 683) — touch gestures pushed it to 810, so it split along
  the seams the register had already named: a subcomponent (`RollToolbar.tsx`) and two
  interaction hooks (`rollGestures.ts`, `useRollShortcuts.ts`).
- **`TranscribeEditor.tsx`** (550 → 442) — geometry and canvas drawing into
  `transcribeGeometry.ts` and `transcribeDraw.ts`.
- **`command.rs`** (980) — now a directory: `command/mod.rs` (session and history),
  `command/edits.rs` (the gestures), `command/tests.rs`.
- **`unplugged-transcribe/src/lib.rs`** (1090 → 601) — tests into a sibling `tests.rs`.

`ai.rs` also shed its note diff to `core::diff`, which is a start on its own row rather
than a discharge of it.

## Coding standards

**Comments explain constraints, not mechanics.** The house style — visible in any file in
`crates/` — is a comment that says *why the code must be this way* and what breaks
otherwise ("velocity 0 would be read as a note-off by some instruments, so…"). Never
narrate what the next line does, and never reference the change you are making ("now
fixed", "new version") — comments describe the code as it is, timelessly.

**Tests are the spec.** Test names are sentences
(`a_directory_is_never_migrated_onto_itself`). Every bug fixed gets a test that would have
caught it. Pure crates aim for exhaustive coverage of decision logic; shells are exercised
by compile checks and the manual checklists in DECISIONS.md. If an invariant spans files
that the compiler cannot connect — like a path named in Rust, Swift and an entitlements
plist — write a test that reads the other files and asserts agreement.

**Errors are reported, not swallowed.** A failure the user can do something about goes to
the UI console; a degraded mode (no audio engine, no shared container) starts anyway and
says so on stderr. `unwrap()` is acceptable only in tests and at startup where continuing
would be worse than dying.

**Realtime code is a different country.** Anything reachable from the audio thread —
`Sequencer::render`, `unplugged_plugin_render`, the `internalRenderBlock` — allocates
nothing, locks nothing, and never panics. Preallocate buffers at init; truncate rather
than grow; guard every float→int conversion. In Swift, capture what the block needs
outside the block; ARC traffic on the audio thread counts as a violation.

**Every C ABI entry point catches panics.** Unwinding into Swift is undefined behaviour,
and in the plugin the process it kills is the user's DAW. Wrap bodies in the existing
`guard(fallback, || ...)` pattern and return a fallback. Strings cross the boundary as
`strdup`'d C strings with a matching `_free` function; every `create` has a `destroy`.

**Naming.** Rust: modules are nouns (`store`, `recorder`), functions say what they return
or do (`tolerance_ticks`, `migrate_projects`). TypeScript: components PascalCase, one
component per file, hooks in `useX` form. No abbreviations that save three characters.

**Frontend.** All backend access through `src/lib/api.ts` — components never import Tauri
directly (this is also what keeps the browser mock working). Styling is hand-rolled CSS
against the design tokens in `src/styles/tokens.css`; no CSS-in-JS, no UI framework.
State that belongs to a feature stays in that feature's directory.

**Responsive rules answer two separate questions.** *Width* decides what the layout is and
uses one of the five rungs written out in `tokens.css` — never a new number. *Pointer*
decides how big things have to be and whether hover means anything, and keys off
`pointer: coarse` / `hover: none` at any width. Keeping them apart is what stops an iPad
at 1024px getting cursor-sized hit targets and a 400px-wide desktop window getting
finger-sized ones. Every `:hover` rule belongs inside `@media (hover: hover)`: iOS
synthesises a hover on tap and leaves it there, so an unguarded one reads as a stuck
control. A phone rule lives in the same file as the thing it restyles; only what is
genuinely cross-cutting goes in `touch.css`.

**Insets go through the `--safe-*` variables**, never a bare `env(safe-area-inset-*)`. A
fixed grid track has to fold the inset into its own height, and repeating the `env()`
fallback at every such site is how two of them drift apart. It also means the whole phone
layout can be checked in a desktop browser by overriding four variables.

**Dependencies.** Adding one is a decision, not a reflex: it must be verified working on
both Apple targets, and recorded in DECISIONS.md with the alternatives considered. The DSP
crate has zero dependencies on purpose; keep it that way.

## Invariants that span the codebase

These agree by convention, and disagreement fails silently. Each has (or must keep) a
test or a greppable comment chain:

| Thing | Places that must agree |
|---|---|
| App Group id `group.com.unplugged.daw` | `src-tauri/Entitlements-Signed.plist`, `plugin/Support/*-Signed.entitlements`, `shared_container.rs::APP_GROUP`, `UnpluggedAudioUnit.swift` |
| Shared data path `~/Library/Application Support/Unplugged` | `shared_container.rs::HOME_RELATIVE_DIR`, `UnpluggedAudioUnit.swift::homeRelativeDataDirectory`, `plugin/Support/UnpluggedAU.entitlements` (tested: `the_three_places_that_name_the_shared_path_agree`) |
| AU identity `aumi` / `Unpl` / `Lffn` | `plugin/Support/Info.plist`, `scripts/verify-plugin.sh`, any docs |
| `CRenderedEvent` layout | `crates/unplugged-plugin/src/lib.rs` ↔ `plugin/Support/UnpluggedPluginFFI.h` |
| Tool definitions | described once in `unplugged-core::ai`; `openai::tool_schema` rewraps them, and a second set of definitions would be a second place to forget |
| Only the read path decrypts the API key | `Keychain.swift` ↔ `keychain.rs` — status is answered from item *attributes*, never `kSecReturnData` (tested: `only_the_read_path_asks_the_keychain_for_the_secret`) |
| Bundle-id prefix rule | extension id must be prefixed by its container app's id (`project.yml` explains) |

## Project format

A project is a directory under the shared data dir (see the invariants table for where
that is):

```
<data_dir>/projects/<project-id>/
  project.json          name, tempo, time signature, PPQ, track list, per-track settings
  tracks/<track-id>.mid one SMF per track (format 0, notes only)
```

`project.json` is authoritative for tempo, time signature and PPQ — the per-track MIDI
files deliberately do not duplicate them. All writes are atomic (temp file + rename).
Audio takes are held in memory for the session, never written into the project; giving
them a place on disk needs a schema bump and a lifecycle, and is future work.

## Hard constraints (from the original spec — do not relitigate)

- All MIDI I/O and audio synthesis live in Rust/Swift. **Never** use `navigator.requestMIDIAccess`
  or Web Audio for playback; WKWebView cannot do either acceptably.
- The Anthropic API key lives in the platform Keychain and is read only inside
  `unplugged-ai`, immediately before a request. It must never reach the webview, a config
  file, or a log.
- The model list comes from `GET /v1/models` at runtime. No hardcoded model strings. This
  holds for a local server too, where it is the only way to know what is loaded.
- Logic's note clipboard format is proprietary; do not attempt to reverse-engineer it.
  Interchange is SMF files.
- AI edits operate through the fixed tool surface against a scratch workspace and land as
  one transaction. The model never writes note data directly.

**Out of scope** (decided, not forgotten): hosting other people's plugins — Unplugged *is*
the plugin, the inversion is documented in DECISIONS.md; recorded audio as arrangement
material — audio is evidence for a transcription, never mixed or bounced; polyphonic
transcription; notation *editing*; Windows/Linux (follows from AUv3-only; a VST3 build
would reopen it); collaboration and cloud sync.

## Verification

```bash
cargo test --workspace                                              # all pure-Rust tests
cargo clippy --workspace --all-targets                              # must be clean
npm run build                                                       # tsc --noEmit && vite build
cargo check --workspace --exclude unplugged --target aarch64-apple-darwin
cargo check --workspace --exclude unplugged --target aarch64-apple-ios
```

All five must pass before any commit is pushed. The Apple `cargo check`s catch API
breakage without a Mac; they do not catch Swift, which only a Mac build verifies —
which is why every Swift-touching change ends with "run `scripts/install-plugin.sh
--debug` and send the errors" rather than a claim of success.

For anything that touches layout, also:

```bash
npm run preview -- --port 4173 &                    # serve the build
npm install --no-save playwright-core               # not a dependency; see the script
node scripts/check-phone-layout.mjs
```

It drives the app at six screen sizes with each device's real safe-area insets simulated,
and asserts what a phone actually breaks on: horizontal overflow, controls under the
status bar or home indicator, hit targets below 44pt, text fields under 16px (the
threshold at which WKWebView zooms in on focus and never zooms back out), and bars whose
`overflow: hidden` is quietly eating a control. Then it drives real multi-touch at the
piano roll. It is not a substitute for a device — see the "Not verified" list in
DECISIONS.md Phase 11 — but every failure it reports is real.

On a Mac:

```bash
npm run tauri dev                    # the standalone app
npm run build:ios                    # the iOS app — see below
npx tauri ios dev                    # …on a simulator or a connected device
scripts/install-plugin.sh --debug    # build + install + register the AUv3
scripts/verify-plugin.sh             # what is installed / registered / offered to hosts
auval -v aumi Unpl Lffn              # full AU validation
```

`npm run build:ios` wraps `tauri ios build`, which needs five things this repository
cannot provide and does not report clearly when they are missing: macOS, full Xcode
rather than the Command Line Tools, CocoaPods, the three Rust iOS targets, and the
generated Xcode project under `src-tauri/gen/apple` — which is untracked, because a
pbxproj is unreviewable in a diff and the tree is regenerable. The script checks each,
runs `tauri ios init` when there is no project yet, and passes every argument through
(`npm run build:ios -- --debug`, `-- --no-sign`, `-- --open`). Its header explains why
each check is there; `npm run build:ios -- --help` prints it.

It also copies `NSMicrophoneUsageDescription` from `src-tauri/Info.plist` into the
generated iOS plist on every build. That is the one entry the app genuinely cannot ship
without — iOS kills the process the moment it touches the input device rather than
denying permission — and the file it has to live in is rewritten by `tauri ios init`, so
setting it by hand does not stay set.

Builds are stamped (version + short commit + dirty flag) via `unplugged-core/build.rs`,
shown in the app titlebar and the plugin window. When testing "did my fix load", trust the
stamp, not the file timestamps — Logic caches AU scans and keeps extension processes alive.

## Editor shortcuts (for manual testing)

`Space` play/pause · `R` record · `L` listen/transcribe · `J` join selection · `⌘K`
prompt · `⌘Z`/`⇧⌘Z` undo/redo · `⌘A` select all · `⌘C/X/V` copy/cut/paste at playhead ·
`⌘Q` quantize · `⌫` delete · arrows nudge (`⇧` = octave/bar) · `⌥`-click delete note ·
`⌘`-scroll zoom · `⇧`-scroll pan · click empty grid draws, drag marquee-selects.

**Two modes take the keyboard, and while either is on the editor's shortcuts are
suspended.** This is deliberate: the letter keys mean different things in each, and there
is no arrangement in which `L` can be both Listen and D.

- **The listen overlay**, while it is up. `Space` plays the take back rather than the
  project; `⌫` deletes the selected note; `J` joins it to the note after it.
- **Typing mode**, toggled from the on-screen keyboard panel and left with `Esc`. `A`–`L`
  + `W/E/T/Y/U` play the keys, `Z`/`X` shift the octave. The panel is outlined while it
  is on, because "why did Space stop playing?" needs an answer on screen.

On a touchscreen: one finger does what the mouse does — draw, select, drag, resize, scrub.
**Two fingers pan the roll, and moving them apart or together zooms time.** That is the
only way to navigate the roll without a wheel, so it is the first thing to try if the grid
appears to be stuck where it opened.

## Working agreements

- Work in phases; stop for review when a phase lands. Do not run ahead.
- Record every decision, risk and unverified assumption in DECISIONS.md as you go — it is
  the project's memory, and the "what a human should test" checklists there are the
  handoff contract.
- When you remember an Apple API rather than know it, say so in DECISIONS.md; four of the
  five plugin build failures were correctly predicted there, which made them cheap.
- Compiler errors from the user's Mac are a deliverable, not a failure. Fix what the log
  shows, then read the surrounding code for the *next* error the compiler has not reached
  yet — bodies are only type-checked after declarations resolve.
- Commit messages explain why, in prose, like the existing history. Push to the designated
  branch only; never open a PR unless asked.
