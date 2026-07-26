# DECISIONS.md

Running log of architectural decisions for **Unplugged**. Newest phase last.

---

## Phase 0 — Dependency verification (required before any code)

The spec asked for verification that every named crate supports both targets
(`aarch64-apple-darwin`, `aarch64-apple-ios`) and that conflicts be reported rather
than silently worked around. Results below are from actually compiling against both
targets, not from memory.

### Build host caveat (read first)

This work is being done on **Linux x86_64 with no Xcode, no Swift toolchain, and no
macOS SDK**. That bounds what "verified" can mean here:

| Verification | Possible here? |
|---|---|
| `cargo check` of pure-Rust crates against Apple targets | ✅ yes — done, results below |
| `cargo test` of platform-agnostic domain logic | ✅ yes — done |
| `cargo check` of the Tauri app crate | ✅ yes, but only against the **Linux** backend |
| Compiling/linking the macOS or iOS app | ❌ no — needs Xcode |
| Any Swift code | ❌ no — no Swift toolchain |
| Anything touching CoreMIDI/AVAudioEngine at runtime | ❌ no — needs real hardware |

Everything Swift-side from Phase 2 onward is **written-but-unverified** until someone
builds it on a Mac. Each phase's "needs manual testing" list says exactly what that means.

### Results

| Crate | Version | macOS aarch64 | iOS aarch64 | Notes |
|---|---|---|---|---|
| `midly` | 0.5.3 | ✅ checks | ✅ checks | Pure Rust, no platform code. No concerns. |
| `midir` | 0.11.0 | ✅ checks | ✅ checks | iOS is a **declared** target, not incidental — see below. |
| `coremidi` | 0.9.1 | ✅ checks | ✅ checks | Pulled in by `midir` on both Apple targets. |
| `tauri` | 2.11.5 | ✅ (documented) | ✅ (documented) | Not compile-verified here — needs macOS SDK. |

#### Finding 1 — `midir` on iOS is supported, but its timestamps are not

The spec flagged midir's iOS backend as "untested — verify early." It compiles, and the
support is deliberate rather than accidental: `midir`'s own `Cargo.toml` declares

```toml
[target.'cfg(target_os = "ios")'.dependencies.coremidi]
version = ">=0.8.0, <=0.9"
```

and `src/backend/mod.rs` gates the CoreMIDI backend on `target_os = "ios"` explicitly.
So **no Swift CoreMIDI fallback plugin is needed for basic port I/O**, which removes a
chunk of the risk the spec anticipated.

However, there are exactly two `cfg!(not(target_os = "ios"))` branches in the CoreMIDI
backend, and one of them matters a great deal:

`src/backend/coremidi/mod.rs:97` — on iOS, midir **skips populating
`message.timestamp` entirely**. On macOS it calls `AudioGetCurrentHostTime()` /
`AudioConvertHostTimeToNanos()` and hands you a real host-clock timestamp; on iOS the
timestamp field is simply left unset.

**Consequence, and it is a design-level one:** incoming MIDI on iOS carries no usable
event time. If Phase 4 (loop recording of external MIDI) were built on midir's
timestamps, it would record with correct timing on macOS and audibly wrong timing on
iOS — the kind of bug that only shows up on device, late.

**Decision:** the sequencer never trusts midir's timestamp on any platform. Input
events are stamped against **our own audio-clock sample position** at the moment the
callback fires. macOS and iOS therefore share one timing path, and the iOS gap becomes
a non-issue instead of a platform-specific bug. This costs us the small amount of
jitter-correction the host timestamp would have bought on macOS; that is the right
trade for a single code path, and it is revisitable in Phase 4 if measured jitter is
unacceptable.

The second branch (`:411`, timestamped *send*) only affects output scheduling, which we
do from the audio thread anyway. Not a concern.

#### Finding 2 — "one Swift plugin shared by both targets" is not achievable as written

This is a genuine conflict with the spec and the reason it is called out here rather
than quietly worked around.

The spec says audio out should be "AVAudioEngine, wrapped in a Swift Tauri plugin
shared by both targets," and that the Swift surface "should be one plugin with a stable
Rust-facing API, not three." **Tauri 2's Swift plugin mechanism is iOS-only.** There is
no macOS equivalent — `run_mobile_plugin`, the `ios/` Swift Package convention, and the
whole mobile plugin bridge do not exist on desktop. This is a known, still-open gap
([tauri#12137](https://github.com/tauri-apps/tauri/issues/12137),
[discussion #9594](https://github.com/tauri-apps/tauri/discussions/9594)); the
recommended macOS path is `swift-rs` or hand-rolled C-ABI FFI.

So the *literal* instruction ("one Swift Tauri plugin, both targets") cannot be
followed. The *intent* behind it — one Swift codebase, one stable Rust-facing API, not
three divergent implementations — is achievable, and that is what will be built:

```
crates/unplugged-audio/            <- ONE Rust-facing API (the stable surface)
  src/lib.rs                       <- pub trait AudioBackend + cfg dispatch
  src/ffi_macos.rs                 <- #[cfg(target_os="macos")]  -> C ABI -> Swift
  src/plugin_ios.rs                <- #[cfg(target_os="ios")]    -> Tauri mobile plugin
  swift/UnpluggedAudio/            <- ONE Swift package, shared verbatim by both
    AudioGraph.swift               <- AVAudioEngine, sampler, render callback
    MidiScheduler.swift
    AudioUnitHost.swift            <- Phase 9
```

The Swift *code* is shared; only the ~100-line binding shim differs per target, and it
is `cfg`-selected so callers never see it. Rust callers get one API.

**Risk flagged:** `swift-rs` was last published 2024-08-27 — roughly two years stale as
of this writing. It is the mainstream option and the FFI surface it covers is small and
stable, but if it proves unmaintained the fallback is a hand-written C-ABI shim plus a
`build.rs` invoking `swiftc` directly. That fallback is entirely within our control and
adds maybe a day, so this is a tracked risk, not a blocker. Decide in Phase 2.

> **Superseded in Phase 2 — see "Resolving Finding 2" below.** The binding turned out
> not to need `swift-rs` or a mobile plugin on *either* platform, which removes both the
> divergence and the stale-dependency risk. The finding above is left as written because
> the reasoning that led there still matters; the conclusion changed.

#### Finding 3 — no substitutions were made

`cpal` was not used anywhere (correctly — the spec's reasoning about AUv3 needing an
AVAudioEngine graph in Phase 9 is sound). No crate in the spec was swapped for another.
The only deviation from the letter of the spec is Finding 2, which is a platform
limitation rather than a preference.

---

## Phase 1 — Scaffold, project CRUD, persistence

### Where the logic lives

All real logic lives in **`crates/unplugged-core`**, a pure-Rust crate with no Tauri and
no platform dependencies. `src-tauri` is a thin command-wrapper over it.

This is partly good hygiene and partly forced: the Tauri app crate cannot be compiled
for macOS or iOS on this host, so anything living there is unverifiable. `unplugged-core`
compiles and its tests run anywhere, including against both Apple targets. Keeping the
domain logic there means the parts that are hard to test on a Mac are the parts that
barely have any logic in them.

### On-disk format

A project is a directory under the app data dir, per spec:

```
<app_data>/projects/<project-id>/
  project.json          manifest — name, tempo, time sig, PPQ, track list, settings
  tracks/<track-id>.mid one SMF per track
```

Decisions within that:

- **`project.json` is authoritative for tempo, time signature and PPQ.** The per-track
  `.mid` files store *only* note events plus a track-name meta event. Tempo is
  deliberately not duplicated into them — two copies of tempo is a desync bug waiting
  to happen.
- **Per-track files are SMF format 0** (single track). Export (Phase 5) assembles
  format 1 with a conductor track carrying tempo/time-signature, as the spec requires.
  Storage format and interchange format are separate concerns.
- **PPQ is stored once, at project level.** The spec lists PPQ as part of the track
  model; in memory each `Track` does carry it, but tracks within a project cannot have
  differing PPQ (they would not play together), so the manifest holds the single
  canonical value and `Project::load` stamps it onto each track.
- **`schema_version`** is in the manifest from day one so migrations are possible later.
- **Writes are atomic** — serialize to a temp file in the same directory, then `rename`.
  A crash or a full disk mid-save leaves the previous project intact rather than a
  truncated `project.json`.
- **Project id is a slug derived from the name, deduplicated with a numeric suffix.**
  The id is stable across renames; only the display name changes. Renaming does not move
  the directory, so nothing breaks if a rename is interrupted.

### Frontend

Two screens, no router: the picker is the launch screen and opening a project swaps in
the editor. The editor shell is the full layout from the spec (wide left column, narrow
inspector, fixed bottom bar, optional console) with each unbuilt panel labelled by the
phase that fills it in, so the scaffold never reads as a finished-but-broken feature.

One addition not in the spec: **`src/lib/mockBackend.ts`**, an in-memory stand-in used
only when the app is opened in a plain browser. Since the real backend runs only on
macOS and iOS, without it there is no way to exercise the UI at all on a non-Mac. It is
gated behind `isTauri()` and unreachable in the real app. It is explicitly *not* a
second implementation of the domain — it makes the picker clickable and nothing more.

### Deferred deliberately

Undo/redo and the command layer are **Phase 3**. Phase 1 mutates only at project
granularity (create/rename/delete), never note data, so nothing here establishes a
mutation path that would later bypass the command layer. This matters — the spec is
explicit that *every* edit path goes through commands, and the cheapest way to honor
that is to not create a competing path now.

### What was verified, and how

Automated, on this Linux host:

- **31 unit tests** in `unplugged-core`, all passing. They cover SMF round-tripping
  (including the two cases that break naive implementations: overlapping same-pitch
  notes, and note-on-with-velocity-0 as note-off), atomic writes, path-traversal
  rejection on delete, schema-version refusal, corrupt-project isolation, and slugging
  of non-ASCII and punctuation-only names.
- `cargo clippy --workspace --all-targets` — clean, no warnings.
- `cargo check -p unplugged-core` against **both** `aarch64-apple-darwin` and
  `aarch64-apple-ios` — clean.
- `cargo check -p unplugged` (the Tauri crate) — clean, but **against the Linux
  backend only**. This catches command-signature and API breakage; it does not verify
  anything macOS- or iOS-specific.
- `tsc --noEmit && vite build` — clean.
- The full UI was driven end-to-end in headless Chromium against the browser-preview
  build: create → validate → editor → add/delete track → console → settings → theme
  switch → back → rename → delete, at 1440px, 834px and 390px. Zero console errors;
  no horizontal overflow at any width.

That last check found and fixed a real bug: grid and flex children size to `min-content`
by default, so a single nowrap string in the transport bar pushed the whole layout wider
than a phone screen. Invisible at desktop width.

### Not verified — needs a Mac

Nothing in Phase 1 is platform-specific *in its logic*, but these cannot be confirmed here:

1. `npm run tauri dev` / `tauri build` actually launching on macOS.
2. `tauri ios init` and a simulator or device build.
3. That `app_data_dir()` resolves as expected on each platform, and that the app has
   write permission there inside the iOS container.
4. Bundle identity: the icon is a generated placeholder (`src-tauri/icons/icon.png`),
   and only a single PNG rather than the full `.icns`/`.iconset` matrix a real bundle
   wants. `tauri icon` should regenerate these from real artwork.
5. Code signing and notarization — no profile available here.

### What a human should test manually

- [ ] `npm install && npm run tauri dev` on macOS — the picker appears, dark by default.
- [ ] Create a project; confirm the directory appears under
      `~/Library/Application Support/com.unplugged.daw/projects/<id>/` with a
      `project.json` and `tracks/track-1.mid`.
- [ ] Quit and relaunch — the project is listed, with the right tempo, time signature
      and track count.
- [ ] Rename a project, then confirm in Finder that the **directory name is unchanged**
      and only `name` in `project.json` differs.
- [ ] Create three projects all named "Untitled"; confirm they become `untitled`,
      `untitled-2`, `untitled-3` and none overwrites another.
- [ ] Delete a project; confirm the whole directory is gone.
- [ ] Corrupt a `project.json` by hand, relaunch, and confirm the other projects still
      list and the console reports the bad one by name.
- [ ] Open a `tracks/*.mid` in Logic Pro — it should import as an empty (or note-bearing)
      track at the right PPQ.
- [ ] `tauri ios init` then run in the simulator; confirm the layout stacks correctly and
      nothing scrolls horizontally on an iPhone-sized screen.

---

## Phase 2 — Audio engine, sampler, transport

### Resolving Finding 2: one C ABI, both targets

Phase 0 concluded that "one Swift Tauri plugin shared by both targets" was impossible and
proposed a `swift-rs` shim on macOS beside a Tauri mobile plugin on iOS — one Swift
codebase, two binding paths. While building it, that turned out to be solving a problem
we do not have.

**Tauri's Swift plugin mechanism exists so JavaScript can call Swift. Nothing in this app
does.** The spec puts Rust in charge of the sequencer, the MIDI I/O and the API key; the
webview only ever talks to Rust. Once that is true, the mobile plugin bridge has no job,
and the reason macOS and iOS had to differ disappears with it.

So the Swift package exports a plain C ABI (`@_cdecl`) and is linked into **both**
binaries identically:

```
swift/UnpluggedAudio/          ONE Swift package, byte-identical on both platforms
  Sources/CUnpluggedFFI/       C header declaring the Rust symbols Swift calls
  Sources/UnpluggedAudio/
    AudioGraph.swift           AVAudioEngine, samplers, the render callback
    Bridge.swift               @_cdecl exports — the entire Rust-facing surface
crates/unplugged-audio/
  src/backend.rs               AppleBackend (both targets) | NullBackend (everywhere else)
  src/shared.rs                lock-free transport state
  src/lib.rs                   AudioEngine + the audio-thread entry point
```

This is strictly better than the Phase 0 plan: **zero** platform divergence rather than a
100-line shim per target, and `swift-rs` — the stale dependency flagged as a risk — is not
needed at all. The spec's "one plugin with a stable Rust-facing API, not three" is met
literally, not just in spirit.

### Who owns what

Per the spec: **Rust owns the sequencer and decides when notes fire; Swift owns the graph
and the render callback.** Concretely, Swift installs `AudioUnitAddRenderNotify` on the
main mixer, which fires on the audio thread before each render quantum, and calls
`unplugged_audio_render` to ask Rust what happens in the next buffer. Rust answers with
sample offsets; Swift applies them via `scheduleMIDIEventBlock` at
`AUEventSampleTimeImmediate + offset`. That is what makes output sample-accurate rather
than buffer-quantised.

The audio thread never blocks:

- Note data crosses via `ArcSwap` — a lock-free pointer swap — with a generation counter
  so the cursor is only re-seated when the timeline actually changed.
- Transport commands are atomics. A seek is an *edge*, not a state, so it carries a
  generation counter and is applied exactly once.
- Every buffer is preallocated. `Sequencer` holds a fixed-capacity sounding-note list
  (`MAX_SOUNDING`), and `RenderState` owns a preallocated event `Vec`.

### The sequencer is pure, and that is the point

`unplugged-core::sequencer` has no atomics, no FFI and no platform types. Tick-to-sample
conversion, loop wrapping and note-off bookkeeping all live there, covered by 21 unit
tests that run on any host. If that logic sat in Swift it would be untestable on this
machine — and it is exactly the kind of logic where an off-by-one is inaudible until it
is a stuck note in a live take.

Decisions inside it worth recording:

- **The cursor is kept in samples, not ticks.** Samples are what the audio clock counts;
  deriving ticks from samples means rounding error cannot accumulate across buffers.
- **A note-off held across a loop boundary is emitted explicitly at the wrap.** Its real
  off event lies past the loop end and would never be reached — the classic hanging-note
  bug. There is a test for exactly this.
- **A zero-length or inverted loop region is rejected, not obeyed.** A zero-length loop
  would spin forever inside one render call; `render` is also tested against a loop
  shorter than a single buffer.
- **Tempo changes hold musical position fixed** and recompute the sample cursor, so the
  playhead does not jump on the timeline when tempo changes.
- **Solo beats mute**, matching every DAW, including for a track that is both.

### The built-in instrument

`AVAudioUnitSampler`, one per track, each feeding a per-track mixer before the main mixer.
The per-track mixer exists now so Phase 9 can drop an AUv3 in where the sampler sits
without disturbing anything downstream.

No SF2 is bundled yet. `loadDefaultInstrument` looks for one and falls back to the
sampler's built-in tone with a console warning rather than failing. The spec calls this
the test instrument and not a feature, so a soft failure that keeps the app audible is
the right trade — but **a real sample set still needs adding** (e.g. the referenced
`fuhton/piano-mp3`, or any SF2 dropped into the bundle as `Piano.sf2`).

### On-screen keyboard

Two octaves with octave shift, touch/click, and the Logic-style computer mapping the spec
names: `A–L` white, `W/E/T/Y/U` black, `Z`/`X` octave down/up. Two details that are bugs
if missed:

- `event.repeat` is ignored, or a held key retriggers dozens of times a second.
- All held notes are released on window blur. A keyup delivered to another window never
  arrives, and the note would sound forever.

Live notes bypass the sequencer entirely — they are not on the timeline, so they play
immediately rather than being scheduled.

---

## Phase 3 — Piano roll, command layer, undo/redo

### The command layer is enforced, not just documented

The spec requires every edit path to emit commands from one layer. That is structural
here: `EditSession` owns the tracks and exposes only `&`-access, so `apply` is the only
way to change a note. There is deliberately no `&mut` accessor.

The Tauri surface reinforces it. The frontend cannot send raw commands — it sends
`EditRequest`, a closed set of *intents* ("move these notes by this much"), and Rust
derives the resulting note values. Clamping rules therefore live in exactly one place, and
a buggy or hostile webview cannot write a note that violates the model's invariants.

### Undo stores inverses, not snapshots

Snapshotting each track would be simpler, but Phase 6's AI edits can touch thousands of
notes per transaction and history would grow without bound. Instead each applied command
returns its inverse.

Consequences that needed care, each with a test:

- **Replace removes and re-inserts rather than assigning in place.** An edit can change
  `start_ticks` or `pitch`, which changes sort position; assigning in place would silently
  break the sorted-notes invariant. The test that catches this drags a note past its
  neighbour.
- **Duplicate notes are matched positionally, not by equality.** Two identical notes must
  map to two distinct indices or one vanishes on undo.
- **A transaction validates fully before mutating anything.** A half-applied transaction
  would be un-undoable. Rejected transactions leave no history entry.
- **Empty transactions push no history.** A drag that ends where it started should not
  leave a mystery undo step.
- History is capped at `MAX_HISTORY` (200) transactions.

Musical operations (transpose, quantize, humanize, harmonize — the Phase 6 tool surface)
are deliberately *not* command variants. They compose from `Insert`/`Delete`/`Replace`, so
there is one inverse implementation to get right instead of twenty.

### Piano roll

Canvas rather than DOM: a track can hold thousands of notes, and DOM nodes at that count
make zoom and scroll stutter. The cost is manual hit-testing, which is why the coordinate
maths lives in `pianoRollGeometry.ts` — separable and reasoned about on its own.

Interaction decisions:

- **Click empty space inserts; drag empty space marquee-selects.** Distinguished by a
  3px threshold, so a slightly-shaky click still draws a note.
- **The drag delta is snapped, not the absolute position.** Snapping positions would
  collapse a dragged chord onto grid lines and destroy its internal rhythm.
- **Selection is re-derived from Rust after every edit.** Indices shift when notes
  reorder, so the backend returns the affected indices and the UI adopts them. Keeping
  selection purely client-side would desynchronise on the first reordering drag.
- Velocity is edited in a lane beneath the roll and also drives note opacity, so dynamics
  are readable without opening anything.
- The wheel handler is registered non-passively so `preventDefault` works — otherwise the
  page scrolls and the trackpad pinch-zooms the whole webview instead of the roll.

### What was verified

- **89 Rust tests** (75 in `unplugged-core`, 14 in `unplugged-audio`), all passing.
  Clippy clean. Both Apple targets compile-check, now including `unplugged-audio`.
- The editor was driven end-to-end in headless Chromium: draw, undo/redo, marquee,
  drag, select-all, nudge, quantize, copy/paste, delete, transport play/stop/RTZ,
  spacebar, on-screen keys by mouse and by computer keyboard, octave shift, save. Zero
  console errors, no horizontal overflow at 390px or 834px.
- The on-screen keyboard's layout was checked numerically rather than visually — 10 black
  keys across two octaves, correctly named, with non-uniform spacing at E–F and B–C. (A
  glance at the screenshot suggested evenly-spaced black keys, which would have been
  wrong; measuring showed the layout was correct. Worth noting as a caution about
  eyeballing low-resolution screenshots.)

### Not verified — needs a Mac

**Everything Swift is written but never compiled.** No Swift toolchain exists on this
host. Specifically unverified:

1. `swift/UnpluggedAudio` compiling at all — syntax, API signatures, availability.
2. `crates/unplugged-audio/build.rs`. It has never executed; the `xcrun`/`swift build`
   invocation and the `--triple` values for device vs simulator are best-effort.
3. The linkage itself: whether the Rust staticlib and Swift static library resolve each
   other's symbols, and whether the Swift runtime search path is right.
4. ~~That `AudioUnitAddRenderNotify` on `mainMixerNode` fires before the samplers
   render.~~ **Corrected on first Mac build** — `AVAudioNode` has no `audioUnit` member;
   that API was invented. It exposes `auAudioUnit` (an `AUAudioUnit`), whose modern
   equivalent is `token(byAddingRenderObserver:)`. The observer now attaches to the
   **output** node rather than the main mixer, because the output node drives the pull:
   its pre-render runs before it pulls the mixer, which pulls the samplers. Whether
   events actually land in the same buffer is still unverified by ear — see the timing
   check below.
5. Whether `scheduleMIDIEventBlock` is non-null on `AVAudioUnitSampler` in practice.
6. `struct` layout agreement between `CRenderedEvent` (Rust) and `UnpluggedRenderedEvent`
   (C). Field order and the explicit 2-byte padding must match; a mismatch would produce
   garbled events rather than a compile error.
7. iOS audio session behaviour and background audio.

### What a human should test manually

- [ ] `swift build` inside `swift/UnpluggedAudio` on a Mac — expect to fix compile errors.
- [ ] `npm run tauri dev`; press a key on the on-screen keyboard and confirm sound.
- [ ] Confirm `A`–`L` and `W/E/T/Y/U` sound the right pitches and `Z`/`X` shift octaves.
- [ ] Draw notes, press play, confirm they sound at the right times.
- [ ] **Timing check:** draw four notes exactly on beats at 120bpm and confirm they land
      on the click, not consistently early or late by one buffer (~10ms at 512 frames).
- [ ] Hold a chord, hit stop mid-chord, confirm nothing hangs. Then try the Panic button.
- [ ] Set a loop region, play across the boundary, confirm no note hangs at the wrap.
- [ ] Change tempo during playback; confirm the playhead does not jump.
- [ ] Drag a 50-note selection and confirm the roll stays responsive.
- [ ] Undo/redo a long editing session and confirm it lands exactly where it started.
- [ ] On iOS: confirm audio plays with the device muted-switch on (playback category),
      and that backgrounding does not kill the engine.

### First Mac build — what the round trip actually cost

Three build-and-report cycles to get Swift compiling, and the first two were spent on
problems I created rather than on the audio design:

1. **`build.rs` hid the error.** It emitted `cargo:warning` on a `swift build` failure and
   let the link proceed, producing a 200-line "undefined symbols" wall that named the
   symptom and not the cause. Cargo also hides build-script output unless run with `-vv`,
   so Swift's diagnostics were never printed at all. Fixed: the script now captures
   Swift's output, re-emits it line by line through `cargo:warning`, and panics at the
   point of failure. This is the change that actually unblocked diagnosis.
2. **`AVAudioNode.audioUnit` does not exist.** I wrote a plausible-looking API from
   memory. The real one is `auAudioUnit`.

The lesson worth keeping: for native code that cannot be compiled on the development
host, **the error-reporting path is part of the deliverable**. Getting a real diagnostic
back on the first Mac build is worth more than any amount of careful guessing, and a
soft-failing build script destroys exactly that.

A useful detail from the failing log: `Emitting module` and `Compiling Bridge.swift` both
succeeded before `AudioGraph.swift` failed, which means the whole FFI surface and every
declaration type-checked — including the C `uint8_t _pad[2]` → Swift `(UInt8, UInt8)`
tuple import that item 6 above flagged as a risk. That narrows the remaining unknowns to
runtime behaviour rather than API shape.
