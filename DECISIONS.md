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

---

## Phase 4 — External MIDI input and loop recording

### Timestamps come from our clock, always

The Phase 0 decision made concrete. `crates/unplugged-midi` deliberately ignores the
timestamp `midir` hands its callback — it is unpopulated on iOS, and a timing path that
works on one platform and not the other is worse than one that is uniformly approximate.
Every event is stamped with `audio.position_ticks()` at the moment it arrives, so macOS
and iOS record identically.

### The recorder is pure

`unplugged_core::recorder` has no I/O and no platform types, so all 17 of its tests run
here. Decisions inside it:

- **A note held across the loop point is closed at the loop end and reopened at the loop
  start.** Closing alone drops the sound from the top of the next pass, which is wrong
  for a held pad. One continuous press therefore becomes one note per pass — unavoidable
  in a bar-looped take, and what playback would sound like anyway.
- **Pending note-ons are a queue per `(channel, pitch)`, not a slot.** A trill or a
  sustained restrike presses the same pitch twice before the first release; a single slot
  loses one of them.
- **Quantisation is applied at capture, not afterwards**, so the take the player hears on
  the next loop pass is the take that was recorded. It can push a start past its release,
  so duration is derived defensively and never reaches zero.
- Input is masked (`& 0x7F`, `& 0x0F`) on the way in, so a malformed packet cannot
  produce a note the model would reject.

The audio thread cannot call into the recorder, so loop wraps are published as a counter
on `SharedTransport` and picked up by the playhead thread. Missing one would leave a held
note running past the loop point in the take.

### Metronome and count-in

Both live in the sequencer, sample-accurate like everything else, and both are tested.

- Clicks route to **`METRONOME_TRACK` (`u16::MAX`)**, which the Swift graph maps to its
  own sampler chain outside the project track list. The click is therefore immune to
  mute, solo and track gain, and can never end up in an exported file.
- Windows are half-open, so a beat landing exactly on a segment boundary fires once — a
  loop wrap splits a buffer into two segments, which is where a naive implementation
  double-clicks.
- Every click is released, including one still ringing when the transport stops. There is
  a test asserting note-on and note-off counts match; a stuck click is the same bug as a
  stuck note but more irritating.
- **Count-in suppresses timeline events but still advances the cursor**, so playback picks
  up cleanly at the record point instead of dumping the lead-in bars' notes at once.

### Takes go through the command layer

A finished take is one `Insert` transaction, so it is a single undo step — the same
guarantee the spec demands of AI edits, for the same reason. Recording is not a special
mutation path; there still is only one.

---

## Phase 5 — Import, export and sharing

### Two SMF paths on purpose

Storage is format 0 per track (Phase 1); interchange is **format 1 with a conductor
track** carrying tempo, time signature and the project name — the layout Logic Pro and
GarageBand expect. Single-track export and drag-out use format 0 *with* tempo included,
because a bare note list opens at 120 bpm regardless of the project, which looks like a
bug to the person who dragged it.

**Imported timing is rescaled to the project's PPQ.** Files in the wild routinely use 96,
192 or 960; without rescaling the material lands at the wrong tempo silently, which is
worse than failing. A short note can round to zero going down, so duration is floored at
one tick. The UI reports the rescale rather than doing it invisibly.

The file's tempo is *reported*, not applied — overwriting the user's tempo setting because
a dropped file disagreed with it is not the app's decision to make.

### Platform surface

`UnpluggedPlatform`, a second target in the same Swift package, same C ABI, same
both-targets linkage. `platform_capabilities` reports what actually works so the UI never
shows a Share button on macOS that fails when pressed.

- **iOS share sheet** via `UIActivityViewController`. On iPad, omitting the popover source
  rect is a hard crash, not a cosmetic issue, so a centred anchor is always supplied.
- **macOS drag-out** via `NSDraggingSession`. A drag started from WKWebView's HTML5 API
  does not carry a real file promise, so it has to begin in AppKit — this is what makes a
  track droppable into Logic. It needs a live `NSEvent`; if the gesture has already ended
  there is nothing to attach to, and that is reported rather than faked.
- **Copy** puts the file on the pasteboard as both a URL and `public.midi-audio` data. The
  UI says plainly that this pastes into Finder or Files, **not** as notes into Logic's
  piano roll — that format is proprietary and the spec rules out reverse-engineering it.

Exports are staged in the app cache directory so the OS can reclaim them and nothing
litters the user's documents. Filenames are sanitised on the way in *and* again in
`stage_export`, because the name makes a round trip through the webview in between.

### What was verified

- **140 Rust tests** (115 core, 14 audio, 7 midi, 4 tauri), clippy clean, both Apple
  targets compile-check.
- The phase 4/5 UI was driven in headless Chromium: record arm/disarm via button and the
  `R` key, loop toggle with brackets drawn in the ruler, metronome toggle, the input
  settings tab, and the interchange panel. Zero console errors, no overflow at 390px.

### Not verified — needs hardware

1. **Anything involving a real MIDI controller.** Port enumeration, connection, live
   monitoring and recording have never seen a byte from actual hardware.
2. Whether the metronome sampler sounds sensible with the default tone (pitches 84/76 are
   a guess pending a real click sample).
3. The share sheet, drag-out and pasteboard paths — all newly written Swift.
4. iOS document types and Open-in-Place. **Not implemented**: the spec asks for `.mid`
   files arriving from the share sheet and the launch/resume intent handling. That needs
   `Info.plist` document-type registration in the generated Xcode project, which does not
   exist until `tauri ios init` is run. Flagged rather than half-built.

### What a human should test manually

- [ ] Connect a controller; confirm it appears in Settings → Input and that playing it
      sounds through the armed track.
- [ ] Record a take without a count-in; confirm the notes land where they were played.
- [ ] Set a 1-bar count-in and record: only the click during the lead-in, recording arms
      at the playhead.
- [ ] Turn the metronome on and confirm the downbeat is distinguishable, and that it stays
      in step over a few minutes (a drifting click means the sample cursor is wrong).
- [ ] Set a loop, record across the boundary holding a chord; confirm no note hangs and
      that held notes appear at the top of the next pass.
- [ ] Hit stop mid-click; confirm the click does not ring on.
- [ ] Export the project, open it in Logic Pro; confirm tempo, time signature and track
      names all arrive.
- [ ] Drag a track from the inspector into Logic's arrange area.
- [ ] Import a file at a different PPQ (96 or 960) and confirm the timing is right and the
      console reports the rescale.
- [ ] On iOS: share a track and confirm the sheet appears — **on an iPad especially**,
      since that is the popover-anchor crash path.

---

## Phase 6 — AI-assisted editing

### The model gets a scratch copy, not the project

The model never sees a `Command`, never touches an `EditSession`, and cannot write a note.
It calls tools against a `Workspace` — a copy of one track's notes — and when the loop
finishes, the *difference* between that copy and the original becomes one transaction the
user accepts or rejects.

Three things the spec asks for fall out of that shape rather than having to be enforced:

- AI edits go through the same command layer as a mouse drag, because the transaction is
  applied by `EditSession::apply` like every other edit.
- The whole conversation is a single undo step, however many tools ran, because it is a
  single transaction.
- The preview diff exists for free, because the diff *is* how the transaction is built.

The transaction is expressed as a delete followed by an insert rather than a `Replace`.
`Replace` indices refer to the pre-command state, and a transaction that both replaced and
deleted would need its indices to survive the reordering the replace itself causes.
Delete-then-insert has no such coupling and is exactly equivalent.

### Notes get identities, but only inside the workspace

`Note` has no id in the domain model — notes are addressed by index — and that is right for
a piano roll, where indices are stable between gestures. It is wrong across a chain of tool
calls: transpose a note and its sort position changes, so a selection held as indices would
silently drift onto different notes between one tool and the next. Inside the workspace
each note carries an id (its original index, or a fresh one if created), the selection is a
list of ids, and the diff is computed by id. The ids never leave the workspace.

### The tool surface is a closed enum

The sixteen tools from the spec deserialise straight from the API's
`{"name": ..., "input": {...}}` block into a `ToolCall` enum. A name the model invented, or
an argument of the wrong type, fails at that boundary and comes back as a tool result the
model can read and correct. There is no path from model output to note data that does not
pass through that type — and a test asserts the schema list and the enum agree, because a
tool present in one and not the other only fails mid-conversation, at runtime.

Tool errors are returned with `is_error: true` rather than ending the conversation. Half of
what makes the loop usable is that "1/7 is not a note value" is something the model can act
on.

### `humanize` takes a seed

A model's output is unpredictable enough without the code under it also being random. The
PRNG is a seeded xorshift, so the same call twice gives the same notes and the operation is
testable.

### The key, concretely

- Stored by `Keychain.swift` (`kSecClassGenericPassword`, `kSecAttrAccessibleWhenUnlocked`)
  — one Security.framework implementation for both targets, over the same C ABI as the rest
  of the platform surface. A Rust keychain crate would have been a third-party bet on iOS
  support that cannot be verified without a device.
- Read immediately before a request and dropped after. It is never returned to `src-tauri`,
  never serialised into a response, never logged.
- The UI can learn two things: whether a key exists, and its last four characters.
- Off-Apple the store is process-local and says so. Writing a key to a file on a dev host
  would be a security regression dressed up as a convenience.

The pending proposal is held in Rust for the same class of reason. A `Transaction` is raw
`Command`s — exactly what `EditRequest` exists to keep the webview from constructing — so
handing one back through the frontend would undo that design. The frontend gets a diff to
look at and two verbs to decide with, and `ai_accept` refuses if the track changed in the
meantime.

### Model list

Fetched from `GET /v1/models` when the key is saved, defaulting to the first Sonnet-class
model returned (which is the newest, since the endpoint returns newest first). If a key can
reach no Sonnet at all, the first model is used rather than leaving the picker empty.

`thinking: {type: "adaptive"}` is requested and dropped on a 400 that names it. The
alternative was a table of which model ids accept it, which would be wrong the moment a
model ships; the user chose the model and should not have to know this.

### `ureq`, and why it is Apple-only

There is no official Anthropic SDK for Rust, so this is raw HTTP. `ureq` is blocking and
pure Rust, and the tool loop already runs on its own thread.

It is an Apple-only dependency using `native-tls`. rustls' crypto backends (`ring`,
`aws-lc-rs`) need a C compiler targeting Apple, which the Linux build host does not have —
pulling one in cost the `cargo check --target aarch64-apple-*` cross-check, which is the
only thing standing between an API mistake and a Mac build failure. `native-tls` on macOS
and iOS is Security.framework through pure-Rust bindings: no C to build, and verification
follows the system trust store, so an enterprise or MDM policy applies here as elsewhere.
Everything that *interprets* a response is platform-independent and tested; only the socket
is behind a `cfg`.

### The on-screen keyboard bug

Reported during Phase 5 review and fixed here. `live_note_on` sounded a note and never
reached the recorder, so only external MIDI was ever captured. The cause was structural:
two note-on paths, one of which recorded. There is now one, in `input.rs`, and the
on-screen keyboard and a controller both go through it. The `track` argument is gone — live
input always goes to the armed track, as external MIDI already did — and velocity comes
from the Settings value rather than a constant in the frontend.

### What was verified

- **222 Rust tests**, clippy clean, both Apple targets compile-check.
- Round-trip tests assert the property that matters: for any chain of tool calls, applying
  the transaction to a real `EditSession` reproduces the workspace exactly, and one undo
  restores the original.
- The AI panel and the Settings → AI tab were driven in headless Chromium with no key
  present; no console errors, no overflow at 390px.

### Not verified — needs a Mac and a key

1. **Every request.** Not one call has been made to the Anthropic API from this code. The
   request shape, the tool-use loop, the thinking fallback and the model list are all
   unexercised against the real service.
2. **The Keychain.** `Keychain.swift` has never been compiled, let alone run.
3. Whether the tool descriptions actually steer a model well. They are the interface the
   model programs against and they will need tuning against real prompts.

### What a human should test manually

- [ ] Paste a key into Settings → AI; confirm it is checked before it is stored and that a
      deliberately wrong key is rejected there rather than at the first edit.
- [ ] Confirm the model picker fills from the live list and defaults to a Sonnet.
- [ ] Quit and relaunch; confirm the key survives and is still not readable in the UI.
- [ ] "Transpose this up a fifth" on a selection — confirm only the selection moves.
- [ ] "Make it swing" on a straight eighth-note line.
- [ ] "Add a ii-V-I in C" into an empty track, then "arpeggiate that in sixteenths".
- [ ] Ask for something impossible ("make it sound like a trumpet") and confirm it says so
      and changes nothing.
- [ ] Accept a proposal, then ⌘Z — the whole edit must undo in one step.
- [ ] Reject a proposal and confirm the roll returns to normal and becomes editable again.
- [ ] Edit the track while a proposal is on screen is *prevented*; confirm the roll is
      read-only, then confirm the staleness check by making a proposal, undoing something
      via the Transport, and pressing Apply.
- [ ] Remove the key and confirm the panel offers Settings rather than an error.

---

## Phase 7 — Audio to MIDI

### Monophonic, and the UI says so

Polyphonic transcription is a different problem — spectral factorisation or a trained
model, not a pitch tracker — and a version of this that quietly did its best on a chord
would produce plausible-looking nonsense. Given a chord, YIN reports one pitch: usually the
loudest partial, sometimes a difference tone, never the chord. The panel states "one note
at a time" before you record, and a test asserts that no two produced notes ever start
together.

### The pipeline

Four stages, each its own module, all pure and all tested against synthetic signals a Linux
host can generate:

1. **Framing** — 2048-sample windows, 10 ms apart. Long enough for two periods of a low E,
   short enough that a 16th at 160 bpm spans several frames.
2. **Pitch** — YIN. Chosen over plain autocorrelation because autocorrelation's peak is
   biased toward long lags, which shows up as octave errors; YIN's cumulative mean
   normalisation is exactly the fix. Verified accurate to within half a semitone on every
   semitone from C2 to C6 on a harmonic-rich tone.
3. **Onsets** — spectral flux. The only thing that can separate two repetitions of the same
   note: a re-struck C4 is identical to a held one in the pitch track.
4. **Assembly** — segment at onsets, at voicing changes, and at sustained pitch breaks;
   median pitch per segment; then optionally quantise.

The FFT is hand-written (one radix-2 transform, forty lines) and tested against a direct
DFT, which is the only way to be sure of an FFT without another FFT to compare against.

### Two things that were not obvious

**Flux has to be normalised.** A held tone still produces raw flux — summed across five
hundred bins, leakage differences between overlapping windows add up to a visible wobble —
and since a held tone produces *nothing but* that wobble, its peaks stand proud of their own
local median and get picked as onsets. Dividing by the previous frame's total magnitude
turns flux from "the spectrum grew by 12 units" into "the spectrum grew by 40%", and the
same wobble becomes a fraction of a percent while a real attack stays a large fraction of
one. One scale-free threshold then works at any recording level.

**Autocorrelation cannot tell a period from its multiples.** Every beat that lines up at
lag L also lines up at 2L, so half-time scores as well as the real tempo and the choice
between them flips on noise. Taking the shortest lag that scores within 75% of the best
picks the fundamental. Two further details were needed: the envelope is smoothed by ±2
frames first, because the beat period is almost never a whole number of frames (160 bpm at
a 10 ms hop is 37.5) and successive beats otherwise land on alternating frames; and the
peak is parabolically interpolated, because the lag grid near 120 bpm only offers 117.6 and
122.4.

An onset at sample zero is undetectable — flux is a change and there is nothing to change
from. That is not a gap: segmentation opens its first note where the signal becomes voiced
and only asks the onset detector about boundaries in the middle.

### Capture

A **separate** `AVAudioEngine` from the playback graph. Sharing one would mean
reconfiguring the running engine to attach an input tap, which on iOS forces the audio
session into `.playAndRecord` for the life of the app — a permission prompt and a routing
change for a user who never transcribes anything. Two engines cost one extra render thread
while recording and nothing when not.

On iOS the session uses `.measurement` mode, which disables the voice processing that would
otherwise gate and EQ the signal — precisely the processing that would ruin a pitch
estimate.

**No audio is written to disk and no take is kept.** The spec puts recorded audio tracks out
of scope and says the microphone exists only to feed transcription, so samples are captured,
analysed, and dropped at the point the notes are produced. A take is capped at two minutes.

`NSMicrophoneUsageDescription` is in `src-tauri/Info.plist` and
`com.apple.security.device.audio-input` in `src-tauri/Entitlements.plist` — without the
first, macOS kills the process the moment it touches the input device rather than showing a
prompt; without the second, a sandboxed build never reaches the prompt at all.

### What was verified

- **262 Rust tests**, clippy clean, both Apple targets compile-check.
- The pitch tracker on every semitone C2–C6; the FFT against a direct DFT; onsets on
  repeated identical notes, on a sustained tone (which must produce none), and on silence;
  tempo across 72–160 bpm with and without human jitter; and the whole pipeline on melodies,
  legato slurs, vibrato, leading silence, dynamics and a chord.
- The transcribe panel in headless Chromium at 390/834/1440 px: no console errors, no
  overflow.

### Not verified — needs a Mac and a microphone

1. **Every sample.** `Capture.swift` has never been compiled or run. Nothing in this phase
   has seen audio from a real microphone — only synthetic signals.
2. Whether the thresholds hold up on a real room recording. Synthetic tones have no noise
   floor, no reverberation and no breath; `SILENCE_FLOOR`, `MIN_CONFIDENCE` and the onset
   parameters are the numbers most likely to need adjusting after the first real take.
3. The macOS permission prompt, and the sandbox entitlement actually granting input access.
4. iOS: the audio session category change, and whether playback and capture coexist as
   intended. `NSMicrophoneUsageDescription` still has to be added to the Xcode project that
   `tauri ios init` generates — the same gap as Phase 5's document types.

### What a human should test manually

- [ ] Press Record and confirm the permission prompt appears with the right explanation.
- [ ] Deny permission and confirm the panel says so and offers System Settings, rather than
      showing a generic error.
- [ ] Hum a simple scale; confirm the pitches are right and the notes land where you sang.
- [ ] Play the same note four times; confirm four notes, not one.
- [ ] Play a legato slur between two pitches; confirm two notes.
- [ ] Play with vibrato; confirm one note, not a stutter of neighbours.
- [ ] Play a chord and confirm you get a monophonic line rather than something that looks
      like a chord — this is the behaviour the UI promises.
- [ ] Record to the click with "Use the project tempo" on; confirm the notes line up.
- [ ] Record freely with it off; confirm the estimated tempo is plausible.
- [ ] Accept a transcription, then ⌘Z — one undo step.
- [ ] On iOS: confirm the metronome still sounds while recording, and that after stopping
      the route returns to normal (playback should not stay quiet or in the earpiece).

---

## Re-centring: what this app is actually for

Stated plainly, because it should have been stated first:

1. **Turning what you play into MIDI.**
2. **Changing that MIDI by describing what you want.**

Everything else serves those two. The build so far does not reflect that. Both features
were added as panels in the Track Inspector, which means they sit in a scrolling sidebar
*below* the name field, the channel readout, a keyboard-shortcut cheat sheet and the
import/export controls — at 1440px they are below the fold. The app currently presents
itself as a piano-roll editor that happens to have two extras, and that is backwards.

This is a re-prioritisation, not a removal. The piano roll stays and stays good: both
headline features produce notes the user did not type, so there has to be somewhere to see
what arrived and fix the last five percent. That is also why every AI edit and every
transcription is previewed as a diff and lands as one undo step. The roll is the
**verification surface**, not the product.

### What changes

- **A persistent prompt bar**, spanning the editor above the transport, focusable from the
  keyboard. Not a panel you scroll to. This single move is what turns "AI" from a feature
  into the primary interaction.
- **Capture becomes a transport peer.** Today MIDI recording — by far the least distinctive
  thing here — has a transport button, and audio-to-MIDI is buried. Listen gets equal
  billing and its own key.
- **A new project opens with two doors**, not an empty grid: record something, or describe
  something. An empty piano roll and a mouse is how a MIDI editor introduces itself.
- **Transformation history becomes visible.** If prompting is the main verb, the chain
  matters — "quantised 1/16" → "harmonised a third" → "humanised" — and today that exists
  only as labels on an undo stack nobody opens.
- **Vocabulary.** "Track Inspector" is DAW furniture. Name the surfaces after the verbs.

### Two functional gaps this exposes

**Transcription is microphone-only.** For an app whose first job is audio-to-MIDI, being
unable to open a `.wav` or a voice memo is a hole, not a missing convenience — and once
this is a plugin, "point it at an audio track" becomes the *dominant* case and the
microphone the minority one. Needs an audio-file decode path (`AVAudioFile` on Apple) into
the existing pipeline.

**The AI cannot create a track.** "Add a bass line under this" is close to the most natural
sentence a user of this app will type, and the tool surface cannot express it: every tool
operates on one track's notes. Both this and "transcribe into a new track" want a target
concept that Phase 6 does not have.

---

## The transcription editor, and the decision it reverses

The intended shape: a waveform view with the detected notes drawn over it — the measured
pitch track as a continuous line, the notes as adjustable boxes on top — so a wrong note is
corrected against the evidence rather than by ear against a grid.

This is the right idea for a transcription-first app, and it makes several things that were
speculative into requirements.

### It reverses "no audio is kept"

Phase 7 states, twice and approvingly, that the audio is analysed and dropped, and calls
that "the point at which *the microphone exists only to feed transcription* stops being a
claim". **A waveform editor cannot work that way**, so that decision is superseded.

The distinction worth keeping is narrower but still real: audio is retained as **evidence
for a transcription**, not as material in the arrangement. It is attached to the take, not
to the timeline. It is not mixed, not exported, not bounced, and does not become an audio
track. It is played back only inside the transcription editor, to check a note against the
sound it came from. "Recorded audio tracks" stays out of scope; "the audio behind this
transcription" comes in.

That has consequences the current code does not have: takes need somewhere to live in the
project directory, a size budget, and a lifecycle (a take whose notes have been discarded
should not persist forever).

### Most of what it needs is already computed and thrown away

`transcribe()` builds a per-frame track of pitch, confidence and level, and an onset list,
and returns only the notes. Those frames *are* the overlay:

- the per-frame pitch is the continuous line the notes sit on;
- confidence drives how firmly a note is drawn, and marks the ones worth checking;
- onsets are where a note boundary should snap when dragged;
- `cents_off` — already computed per note, currently only a warning count — becomes the
  vertical offset between the drawn note and the measured pitch, which is exactly the
  information a user needs to decide whether a note is wrong or the *source* was flat.

Exposing that is close to free. What is genuinely new: a min/max peak pyramid for drawing
the waveform at any zoom (pure, cheap, testable in `unplugged-transcribe`), audio playback
scrubbing, and the overlay view itself — a second canvas sharing the piano roll's geometry
module.

### And it makes re-derivation the natural model

Once the audio and the frames are kept, the transcription settings stop being burned in at
commit. Changing the quantise grid, switching between the estimated and the project tempo,
adjusting the confidence threshold, or splitting a note at an onset all become cheap
*re-derivations* of the same take rather than a reason to record again. Today every one of
those is a re-record.

That is the change that makes this structurally not a MIDI editor with a transcribe button:
a track gains a **provenance** — this take, these settings, then these edits — and the
first two stay live.

---

## Phase 9 revised — Unplugged as a plugin, not a host

The original Phase 9 was "AUv3 hosting". That is inverted: Unplugged should *be* the plugin,
loaded into Logic Pro and Ableton Live. Hosting other people's instruments is dropped.

### The format is AUv3, and "VST" would not have worked

| | Logic Pro | Live (macOS) | Live (Windows) | iPad |
|---|---|---|---|---|
| **AUv3** | yes | yes | — | yes |
| VST3 | **never** | yes | yes | — |
| CLAP | no | no | no | — |

Logic has never loaded VST and does not intend to; Audio Units only. So AUv3 is the one
format that reaches both named DAWs, and it reaches iPad hosts for free. VST3 buys Windows
and nothing else, and is deferred rather than refused.

### The real problem is getting MIDI *out* of a plugin

This app's product is notes in the host's timeline, and plugins are generally not permitted
to write there. Three routes, and shipping all three is the honest answer:

1. **AUv3 MIDI processor** (`aumi`) in Logic's MIDI FX slot. Real-time MIDI out, native,
   exactly what this app is.
2. **AUv3 instrument** (`aumu`). Loads everywhere and plays through the built-in sampler,
   but most hosts will not capture its MIDI output.
3. **Drag the region out as `.mid`.** Works in every DAW, needs no host cooperation, and is
   already built (`unplugged_platform_begin_file_drag`, Phase 5).

One thing to verify against a real install rather than assume: whether Ableton Live hosts
MIDI-effect plugins at all. The long-standing answer has been no. If that still holds, Live
gets (2) and (3), and (1) is Logic-specific.

### What survives, and why that is not luck

- `unplugged-core`, `unplugged-transcribe` and `unplugged-ai` are pure and move unchanged.
  That is the payoff for keeping platform code out of them from Phase 0.
- `unplugged_audio_render` is already allocation-free, lock-free, and shaped as *"given N
  frames, which events fire and at what sample offset"* — which is precisely an AUv3
  `internalRenderBlock`. `CRenderedEvent` already carries frame offset, pitch, velocity and
  channel, so it maps onto `MIDIOutputEventBlock` directly.
- The React UI survives because `src/lib/api.ts` is the only IPC seam. Two files in the
  whole frontend import from `@tauri-apps`.

### What has to be rebuilt

- **Tauri cannot be a plugin.** It owns the process and its event loop; a plugin is a dylib
  handed an `NSView`. The UI moves into a `WKWebView` inside an `AUViewController`, with
  `invoke` replaced by a `WKScriptMessageHandler` bridge. `src-tauri`'s command bodies
  survive as ordinary functions; it is the shell that goes.
- **The host owns the clock.** `AudioCursor` currently owns a sample cursor and derives
  ticks. In a plugin, tempo and position arrive per-buffer from `musicalContextBlock`. A
  contained change, but a real one, and it needs its own tests.
- **State lives in the host session**, not the app data dir. `Project` is already serde, so
  `fullState` is nearly free — but `ProjectStore` needs a sibling that serialises to a blob.
  Retained audio takes make that blob large, which is a design constraint on the previous
  section.
- **Keychain across the app/extension boundary** needs an app group and a keychain access
  group.

Distribution falls out: on Apple an AUv3 ships as an app extension inside a container app,
so one Xcode project with an app target and an extension target — both linking the same
Rust staticlib — produces the standalone and the plugin together.

### Correction to the Phase 0 record

Phase 0 chose AVAudioEngine over `cpal` **because AUv3 hosting expects an AVAudioEngine
graph**. That justification is now void. The choice happens to survive on its merits — the
standalone still wants AVAudioEngine, and the plugin will not use it at all — so nothing
needs to change. But the reasoning recorded at the time is no longer the reasoning that
holds, and if Windows had ever been in scope, `cpal` would have been the better call.

### Revised phase order

| | |
|---|---|
| **8** | Re-centring: prompt bar, capture as a transport peer, first-run doors, visible history, audio-file input, new-track targeting |
| **9** | The transcription editor: retained takes, exposed frames, waveform + pitch overlay, re-derivation |
| **10** | AUv3 plugin: extension target, host transport, MIDI out, webview bridge |
| **11** | Notation view + MusicXML export |

Notation moves last deliberately. It is a *view* on notes rather than a way of making or
changing them, and building it before the re-centring would deepen exactly the emphasis
this section exists to correct.

---

## Phase 8 built — the re-centring

### One pending thing, one review surface

An AI edit and a transcription turned out to be the same interaction with different
innards: something produces notes you did not type, you look at them on the roll, you
accept or reject. Phase 6 and Phase 7 each built their own panel for that, and the two
rhymed without sharing anything.

They now share a `Pending` type, one `ReviewBar`, one preview overlay on the roll, and one
rule — the roll is read-only while a decision is outstanding, and only one decision can be
outstanding at a time. That last part is not a limitation to work around; it is the
guarantee that makes "undo it in one step" mean something.

The review bar occupies the prompt bar's slot rather than appearing elsewhere. The thing
you were asked to look at and the thing you answer with should not be in two places.

### The prompt bar, and Return

Moved out of the inspector to span the editor above the transport, focusable with `⌘K`
from anywhere — the one shortcut that has to work while another text field has focus,
since taking focus is its whole job.

Return submits; `⇧`Return is a newline. That is the opposite of the Phase 6 panel, and
deliberately: once this is the primary input, one line is the common case and reaching for
a modifier on every use is friction on the main path.

### Listen is a transport button

Not a panel. MIDI recording — the least distinctive thing this app does — had a transport
button while audio-to-MIDI was three sections down a sidebar. `L` sits next to `R` for the
same reason. What stays in the inspector is the two settings that change the *result*, and
the file route in.

Labelled rather than given an icon: there is no established glyph for "turn audio into
notes", and an unlabelled circle beside the record dot would read as a second record
button.

### An empty project offers two doors

Rather than an empty grid and a mouse, which is how a MIDI editor introduces itself. The
doors float *over* the roll with `pointer-events: none` on the wrapper, so the grid behind
stays live and drawing a note by hand is never more than a click away — and the panel says
so. It disappears the moment the project has a note, by any route.

On a phone the roll is a couple of hundred pixels tall and the explanatory copy does not
fit. The two verbs still do, and the verbs are the message; the explanation is a nicety.

### History is not track-scoped

Making the chain visible exposed that the sidebar was mislabelled: the undo stack spans
the project, so "Track Inspector" was the wrong heading for a panel that now leads with
it. The panel is titled History and the track fields sit below under their own label.

The keyboard cheat sheet became a collapsed `<details>`. It was taller than either feature
and outranked both.

### Two gaps closed

**Audio files.** `AVAudioFile` decodes anything CoreAudio can open — wav, aiff, caf, m4a,
mp3, Voice Memos — to mono at the file's own rate, and hands it to the same pipeline. The
formats are deliberately not enumerated in Rust; the system decides what it can decode and
reports what it cannot.

**The AI can write a new part.** `AiTarget::NewTrack` runs the tools against an *empty*
workspace with the visible track supplied as read-only context, so "add a bass line under
this" has something to be written against. The tools stay single-track — that is what keeps
the diff and the transaction simple — and the track is created at accept time rather than
when the proposal is made, so a rejected suggestion leaves no empty track behind. It is
named after the request, because in an app where parts are described rather than played,
what a part *is* is usually what was asked for.

Adding a track is structure, not notes, so it does not push a history entry: undoing past
it removes the notes and leaves the track. That matches how the Add Track button already
behaves, and the alternative is a second kind of history entry every command would have to
reason about.

---

## Phase 9 built — the transcription editor

### What was already there, and thrown away

`transcribe()` built a per-frame track of pitch, confidence and level plus an onset list,
returned the notes, and dropped the rest. Those frames are the editor:

- the per-frame fractional MIDI is the line the notes sit on — drawn where the pitch was
  *measured*, not where it was rounded;
- confidence drives the line's opacity, so a passage the tracker was unsure about looks
  unsure rather than looking like a fact;
- onsets are drawn through both lanes and are what a dragged note edge snaps to;
- a note drawn off its own pitch line is one the analysis hesitated over, which is exactly
  the note worth checking.

`Analysis` is returned with every transcription. Exposing it cost almost nothing.

### Peaks, not averages

`peaks()` returns min/max pairs per bucket over a requested time window. Extremes rather
than mean or RMS: at any zoom where one pixel covers hundreds of samples, averaging turns
a percussive attack into a low bump and the eye loses the one feature it is looking for.

The window is a parameter so the view asks for what is on screen. Shipping the samples to
the webview instead would be twenty-odd megabytes for a few hundred columns.

### The take is kept — in memory

Phase 7 dropped the audio and called that a principle. It is superseded, as recorded
above: audio is evidence attached to a take, never material in the arrangement.

**In memory, for the session.** On-disk persistence is deliberately not here: two minutes
at 48 kHz is ~23 MB, and putting that in the project directory needs a schema bump, a size
budget, and a lifecycle for takes whose notes were discarded. Record, fine-tune, commit is
one sitting, so a session-scoped take buys nearly all of the value for none of that. The
take is released the moment its notes are committed — holding it after that would be the
retention turning into a leak.

### Re-derivation

`capture_retranscribe` differs from `capture_transcribe` only in where the audio comes
from, and that is the entire point. Changing the grid, or switching between the estimated
tempo and the project's, used to mean playing the phrase again.

### Adjustments are validated, not trusted

Dragged notes go back through `capture_set_notes`, which validates every one before it can
reach the command layer. Pitch drags snap to semitones — the line shows where the source
actually sat, and fractions belong on the line, not in the MIDI. Time drags snap to
detected onsets within 50 ms, which is what makes correcting a boundary land on the attack
rather than near it.

### What was verified

- **270 Rust tests**, clippy clean, both Apple targets compile-check.
- New pure tests: the analysis comes back with the notes and its frames line up with the
  notes they produced; peaks keep a transient that averaging would bury, respect the
  requested window, and survive being asked for more buckets than there are samples; a
  reference track appears in the prompt marked untouchable; new tracks are named from the
  request and cut at a word.
- Headless Chromium at 390 / 834 / 1440 px: prompt bar, Listen button, first-run doors and
  transcription settings all present, no console errors, no horizontal overflow.

### Not verified

Everything that needs a Mac, plus one new thing: **the transcription editor has never been
drawn against real audio.** The waveform, the pitch line and the drag interactions have
only been exercised against an empty take in a browser, because the take comes from
`AVAudioFile` or the microphone and neither exists here.

### What a human should test manually

- [ ] Open an empty project; confirm the two doors, and that clicking the grid behind them
      still draws a note.
- [ ] `⌘K` from anywhere focuses the prompt, including while the tempo field has focus.
- [ ] Press `L`, hum a phrase, press `L` again; confirm the review bar appears and the
      notes are drawn green on the roll.
- [ ] Press Fine-tune. Confirm the waveform matches what you sang, the pitch line follows
      it, and the onset marks land on your attacks.
- [ ] Drag a note in pitch; confirm it moves in semitones and the line stays put.
- [ ] Drag a note edge near an attack; confirm it snaps to it.
- [ ] Change the snap grid inside the editor; confirm it re-reads the take rather than
      asking you to sing again, and that it is quick.
- [ ] Transcribe an audio file — a voice memo is the realistic case.
- [ ] Try a file longer than two minutes and confirm the refusal is legible.
- [ ] "Add a bass line under this" with Write a new part selected; confirm a new track
      appears **only** on Apply, named after the request.
- [ ] Confirm the roll is read-only while a proposal is on screen, and editable again
      after Discard.
- [ ] Confirm the history strip lists the chain and that undone steps stay visible, dimmed.

---

## Preview: hearing the take, and seeing it arrive

Reported after the first real transcription — a hummed phrase, correctly transcribed —
with the observation that there was no way to visualise or preview the audio before
committing it. Two genuine gaps and one discoverability failure.

### You could not hear the take

The waveform editor drew the take from the first release, but there was no way to *play*
it. That matters more than it sounds: accepting the notes and listening to the sampler
tells you what the **transcriber** heard, not what you played, and the whole value of a
transcription editor is comparing those two.

`AudioPreview` is a third small `AVAudioEngine`, for the same reason capture has its own —
a player node and an output is the entire graph, and reconfiguring the sequencer's engine
to schedule a one-off buffer would mean touching a running engine for something unrelated
to the project. Position comes from the player's own render time rather than a wall clock,
so the drawn playhead tracks the audio instead of drifting against it.

Clicking the waveform lane plays from there; clicking the pitch lane edits. Splitting the
two by lane means the fastest way to check a suspicious note is to click just before it.

### You could not see the take arrive

A level meter tells you the input is alive. It does not tell you whether the phrase you
just sang came through, and by the time the transcription appears it is too late to know
whether a gap was you or the microphone.

The listening bar draws the take as it accumulates. This cost almost nothing: the samples
are already in Rust — `capture_poll` drains them there — so it is the same
`capture_waveform` the editor uses, asked for repeatedly.

### The editor was behind a secondary button

"Fine-tune…" sat next to "Add to track", and a feature you have to find is a feature that
does not exist. A transcription now **opens** the editor.

The asymmetry with an AI edit is deliberate rather than an inconsistency. For an AI edit
the diff on the roll *is* the review — you can read what changed at a glance. For a take
the waveform is the review, because a transcription cannot be judged against a grid, only
against the sound it came from.

---

## Phase 10 groundwork — knowing which build you are running

Two things the plugin needs before it exists, both of which are the difference between an
hour of debugging and a minute of it.

### The build stamps itself

In a standalone app "which build is this?" is answered by quitting and looking. In a
plugin it is genuinely hard: Logic caches Audio Unit scans, keeps the extension alive in a
separate hosting process, and will happily run a copy you replaced ten minutes ago.
Without a stamp visible **inside the plugin window** there is no way to tell a fix that did
not work from a fix that was never loaded.

`BuildInfo` is baked in at compile time by `unplugged-core/build.rs` and shown in the
editor titlebar and the About tab. The load-bearing field is `dirty`: "did my edit make it
in?" is the actual question, and a clean hash matching the last commit answers it wrongly
when the build came from a modified tree. A dirty build gets a `+` and turns amber.

The build script degrades rather than failing — no git, or a source tarball, still
compiles and still reports something. A build script that can break the build over a
cosmetic string is a bad trade. `SOURCE_DATE_EPOCH` is honoured for reproducible builds.

### Installing from a Documents folder does not work, and the reason is not obvious

An AUv3 is an app extension: it ships *inside* a container app and macOS discovers it by
scanning the app, not by scanning a plugin folder. There is no
`~/Library/Audio/Plug-Ins/Components/` to drop a file into — that is the AUv2 world. What
actually happens is Launch Services notices an app bundle somewhere it indexes,
`pluginkit` registers the extensions inside it, and hosts ask the system for Audio Units.

The first step is where a build in `~/Documents` fails. Launch Services indexes
`/Applications` and `~/Applications` reliably; a Documents folder is scanned
inconsistently and a build directory is not scanned at all.

`scripts/install-plugin.sh` therefore builds, copies to `~/Applications` with `ditto` (not
`cp -r`, which can break a signature), clears any quarantine flag, registers with
`lsregister`, and launches once — the launch is what actually makes `pluginkit` see the
extension; headless registration alone is unreliable.

It also **kills the extension host**. macOS keeps AUv3 extensions alive in
`AUHostingCompatibilityService` between uses, and replacing the bundle underneath one does
not evict it. This is the step people skip and then spend an hour confused by, and it is
precisely the failure the build stamp exists to make visible.

`scripts/verify-plugin.sh` answers "which build will Logic load right now?" by printing
three things that can disagree: what is installed, what `pluginkit` has registered, and
what `auval` says a host will be offered. When a fix "does not work" it is usually because
those are out of step — most often an old extension still registered from a copy that has
since been deleted, which is why the script also runs `mdfind` for stray copies. It ends
by echoing this checkout's HEAD for comparison against the stamp in the window.

### Why the extension target itself is not in this commit

Writing an Xcode project and an AUv3 target blind is the one part of this that would
predictably cost several rounds. The first Mac build of a *much* smaller amount of Swift
took four, and each round is a full round trip. The tooling above is the thing that makes
those rounds survivable — without the stamp, a round where the plugin did not reload is
indistinguishable from a round where the fix was wrong — so it is worth having in place
first rather than discovering the need halfway through.

---

## Phase 10, first pass — the AUv3 extension

### What this pass does and does not do

**Does:** loads in Logic as a MIDI effect, follows the host's transport, and emits a
project's notes as MIDI into whatever instrument follows it. Shows which build is running.

**Does not:** host the editor. That needs the whole Tauri command surface re-homed behind
a bridge that is not Tauri, and doing it in the same step as getting an extension to load
at all would mean two unverified things failing together with no way to tell which broke.
So the projects directory is shared: you author in the app, the plugin plays what you
authored.

That is a real product in its own right — a MIDI FX slot playing your part into Logic's
instrument, in sync — rather than a stub that proves a build works.

### Type `aumi`, and why

A **MIDI processor**, not an instrument. Unplugged makes notes; in a host the sound is
whatever instrument you place after it, which is the point of putting it there. Logic hosts
this in the MIDI FX slot of an instrument track.

The consequence, recorded when the format was chosen and still true: Ableton Live is
believed not to host MIDI-effect plugins at all. If that holds, Live gets the
instrument-plus-drag-out route and this is Logic-specific. It has not been checked against
a real install.

### Following the host's clock without stuttering

The naive implementation — re-seat the sequencer from the host every block — does not work.
Seeking flushes held notes, so a sustained note would re-trigger at the audio block rate.
Trusting our own cursor once started is the opposite failure: it drifts, and misses every
locate and cycle jump.

`host_sync` advances normally and re-seats **only when the disagreement exceeds what normal
playback could produce**. Ordinary advance keeps the two within a fraction of a block; a
locate moves them much further. One threshold separates the cases, and it has two terms —
a block-length term doubled for margin (or every block would seek) and a musical floor of a
32nd note (or a small block would make the threshold collapse into the jitter). Starting
and the first block always locate, because you locate *then* press play, and a plugin that
resumed from where it stopped would play the wrong bar.

It is pure decision logic with no audio and no platform, so the part most likely to be
subtly wrong is exhaustively testable. Ten tests cover steady playback never re-seeking,
locates and cycle jumps being followed, a stopped host being tracked while the playhead is
dragged, and degenerate inputs not producing a zero or NaN threshold.

### The App Group, and the silent failure it prevents

A sandboxed app and a sandboxed extension get **separate containers**. The plugin cannot
read what the app wrote unless both go through an App Group — and when they do not, the
symptom is an empty project list with no error anywhere, indistinguishable from never
having run the app.

So `group.com.unplugged.daw` is declared in three places that must agree: the app's
entitlements, the extension's, and the host stub's.

Existing projects are **copied**, not moved, the first time. This runs on a machine holding
the only copy of someone's work: a failed move is unrecoverable, a failed copy costs disk.
The original is left in place deliberately. That does leave two directories that can
diverge, which is a wart to resolve once the shared location has proved itself — but the
alternative risks the thing that must not be risked.

Migration only ever runs into an *empty* destination. Merging two divergent project
directories is a conflict-resolution problem, and guessing at it would lose work.

### The App Group cannot be the default, so it is not

Risk 4 below — "needs a real signing identity to work at all" — was right, and it is worse
than "may not work". `com.apple.security.application-groups` is **provisioning-profile-backed**:
Xcode refuses to *build* a target that declares one without a team and a matching profile.
The first run of `scripts/install-plugin.sh --debug` therefore did not reach a single line
of Swift; it failed with

```
"UnpluggedAUHost" requires a provisioning profile. Enable development signing and
select a provisioning profile in the Signing & Capabilities editor.
```

And a free Apple ID cannot fix it: App Groups cannot be registered on a personal team at
all. So the choice was to require a paid membership before the plugin can be built once, or
to find another way for two processes to see one directory. Requiring the membership to
*iterate* is the wrong trade — it gates the compiler errors, which is the whole reason for
building this at all.

**What replaces it, ad-hoc:** the extension keeps the App Sandbox — it costs nothing under
ad-hoc signing, and developing without it would hide sandbox bugs until the worst possible
moment — and adds

```
com.apple.security.temporary-exception.files.home-relative-path.read-only
    /Library/Application Support/Unplugged/
```

A temporary exception is a sandbox rule, not a capability, so ad-hoc signing grants it. The
extension gets read access to exactly the one directory it needs, and read-only is not a
concession: the plugin only ever reads projects. The app owns them.

The app's side of that has to be *outside* a container for the exception to reach it, so
the default `Entitlements.plist` drops the sandbox too. `Entitlements-Signed.plist` and
`tauri.signed.conf.json` keep the shipping arrangement intact.

The path is now named in three files — the entitlement, `homeRelativeDataDirectory()` in
Swift, and `HOME_RELATIVE_DIR` in `shared_container.rs` — and a **test reads the other two
files and asserts they agree**, because drift between them fails silently in exactly the
way this whole section exists to prevent: the app writes one path, the plugin reads another,
and the symptom is an empty project list.

One subtlety that had to be got right in Swift: inside the sandbox,
`FileManager.urls(for: .applicationSupportDirectory ...)` and `NSHomeDirectory()` both
answer with *the extension's own container* — precisely the directory the app cannot write
to. `getpwuid(getuid()).pw_dir` reports the account's home regardless, which is what the
exception is written against.

`Sharing` now has three values rather than a boolean, because "the plugin can see this" and
"this is the App Group" stopped being the same claim:

| | Reached by | Needs |
|---|---|---|
| `AppGroup` | sandboxed app + sandboxed extension | paid team, profile |
| `HomeDirectory` | unsandboxed app + sandbox exception | nothing |
| `Private` | nothing | — the plugin sees no projects |

`scripts/install-plugin.sh --signed` with `UNPLUGGED_TEAM_ID` set builds the `Signed`
configuration against the group entitlements. It is untested — nobody here has a paid team —
but it is a build configuration rather than a paragraph, so it will fail loudly when it is
first tried rather than being quietly wrong.

### No panic crosses the boundary

Every C entry point catches. Unwinding into Swift is undefined behaviour, and in a plugin
the process it takes down is the user's DAW along with their unsaved session. A test walks
every entry point with NULL arguments, because each one is reachable from Swift.

The render path allocates nothing and locks nothing — it is called on the audio thread —
and truncates rather than growing when a block produces more events than the caller's
buffer holds. A test seeds 128 simultaneous notes against an 8-slot buffer to prove it.

### A generated Xcode project

`plugin/project.yml` with XcodeGen, rather than a checked-in `.xcodeproj`. A pbxproj is
2000 lines of generated XML with UUID cross-references: unreviewable in a diff and
miserable to merge. The yml is the actual configuration, so a change to the build is a
change someone can read.

The container app is a **stub**. macOS discovers an AUv3 by scanning apps, so the extension
needs a bundle to live in, and the real Unplugged app is the Tauri build. Embedding the
extension there is the next step; until then this lets the plugin be built, installed and
loaded without waiting on it. The stub window says what it is rather than pretending to be
the app.

### Four things remembered wrong, one of which the compiler caught

The first build to reach Swift failed on one error and revealed three more that would not
have failed at all. Worth recording together, because only the first was the compiler's to
find:

**`AUViewControllerBase` is not an SDK type.** It is a typealias Apple's *sample* project
defines for itself. The real base class is CoreAudioKit's `AUViewController`. The tell was
already in the file: a `PlatformViewController` typealias written and then never used —
written for the job, then the class inherited a different remembered name.

**The render block's parameters were off by one.** `AUInternalRenderBlock` ends with the
realtime event list and *then* the pull-input block; the code bound the sixth to
`pullInput`. It also called it with `nil` for the buffer list, which is not optional. Both
are moot now: a MIDI processor has no audio to pull, so it pulls nothing — and it clears
the output buffers, which are not guaranteed silent on arrival and are undefined memory
otherwise.

**`musicalContextBlock`'s arguments were transposed, and would have compiled forever.**
The order is tempo, numerator, denominator, **beat position**, sample offset, **measure
downbeat**. Four of the six are `Double`, so reading the beat position out of the measure
downbeat type-checks perfectly and then follows the bar line instead of the beat. The part
would have played, quantised to the bar, for no visible reason — the exact class of bug the
`host_sync` tests cannot catch, because the wrong number arrives before Rust ever sees it.

**The render path allocated.** `var bytes: [UInt8] = [status, pitch, velocity]` per event is
a heap allocation on the audio thread, in a file whose own comment says it must not contain
one. Preallocated alongside the event buffer now.

One deliberate addition while in there: `Int64(someDouble)` traps on a non-finite or
out-of-range value, and a trap in the render block does not fail the plugin — it takes the
host down with the user's unsaved session. The timestamp conversion is guarded.

### What was verified

- **302 Rust tests**, clippy clean, both Apple targets compile-check including the new
  plugin crate.
- The host follower, the C ABI's null-safety, state round-tripping, a session referencing a
  deleted project still loading, the host tempo overriding the project's, and the render
  path never overrunning the caller's buffer.

### Not verified — all of it, on a Mac

Nothing in `plugin/` has been compiled. Specifically at risk, in rough order of likelihood:

1. ~~**The `AUAudioUnit` subclass.** `internalRenderBlock`, `musicalContextBlock` and
   `transportStateBlock` signatures are the kind of API this project has already got wrong
   once from memory (`audioUnit` vs `auAudioUnit`, Phase 2).~~ **Confirmed, four times over
   — see below.** Bodies are still unchecked; the compiler had only reached declarations.
2. **`AudioComponents` in the extension's Info.plist.** A wrong `type`, `subtype` or
   `manufacturer` registers the component and then never offers it to a host — a failure
   with no error message anywhere.
3. **The bridging header.** An app-extension target reaching a Rust staticlib through
   `SWIFT_OBJC_BRIDGING_HEADER` plus `OTHER_LDFLAGS`, which has more ways to go wrong than
   it looks.
4. ~~**The App Group.** Needs a real signing identity to work at all; ad-hoc signing may not
   grant it, in which case the project list is empty and the fallback path is what runs.~~
   **Confirmed, and worse than written** — it blocks the build outright. See "The App Group
   cannot be the default" above for what replaced it.
5. **Whether a sandbox temporary exception is enough.** The replacement for the group. If
   the extension is denied the directory anyway, the project list is empty — check
   `log stream --predicate 'sender == "Sandbox"'` while Logic scans, and look for a deny
   naming `Application Support/Unplugged`.
6. Whether Logic offers `aumi` extensions from an ad-hoc-signed app at all.

### What a human should test manually

- [ ] `brew install xcodegen`, then `scripts/install-plugin.sh --debug`. Expect the first
      run to fail somewhere in Swift; the compiler errors are the deliverable.
- [ ] `scripts/verify-plugin.sh` — confirm the extension is embedded, `pluginkit` lists it,
      and `auval -a` shows it.
- [ ] `auval -v aumi Unpl Lffn` for a full validation pass.
- [ ] In Logic: a software instrument track, MIDI FX slot, Unplugged. Confirm the window
      opens and the build stamp matches `git rev-parse --short=7 HEAD`.
- [ ] Confirm the project list is populated. If it is empty: run the standalone app once
      first, then check that `~/Library/Application Support/Unplugged/projects` exists and
      has something in it. The app prints where it settled on stderr.
- [ ] Pick a project, press play in Logic, confirm notes reach the instrument.
- [ ] Locate mid-playback and confirm the plugin follows without a stuck note.
- [ ] Turn on Cycle and confirm the loop wrap does not hang a note.
- [ ] Change Logic's tempo and confirm the part follows it rather than the project's.
- [ ] Save the Logic project, reopen it, confirm the same Unplugged project is selected.
- [ ] Delete that project in the app, reopen the Logic session, confirm it loads with
      nothing selected rather than failing.

---

## The toolbar, and Listen as a place rather than a strip

Two changes that look like layout and are not. The first says where a control belongs;
the second says what the transcription feature *is*.

### Controls were sorted by when they were built, not by what they are

The bottom bar had become the place a control went when it needed a home: play beside
record beside listen beside loop beside the clock beside undo beside save. Two different
kinds of thing were in one row — "hear this project" is about the whole document, "record
a take" is about making something new — and the row could only grow.

The split is now by scope, and it is a rule rather than a tidy-up:

- **The toolbar is what concerns the project.** What is on screen (keyboard, history),
  what the transport is doing, what is open (settings, console, import).
- **The bar under the roll is what makes and unmakes notes.** Record and Listen — the two
  ways a performance becomes notes — then loop and click, then undo/redo/save/panic.

Play, pause, stop, the clock and the tempo moved up. Nothing is in both places; a control
in two bars is two states to keep in step and one of them will be wrong.

**Pause and stop are now different buttons, and neither is new behaviour.** Rust's
`transport_stop` has always stopped where it was — that is a pause — and the only way to
get back to the top was to press the return-to-zero button beside it. Logic, Live and
every hardware transport since tape distinguish the two; the app was quietly offering one
of them under the other's name. Stop is `transport_stop` followed by `transport_seek(0)`,
which is exactly what the two old buttons did in sequence.

**The clock shows a bar or a note.** Bars|beats|ticks is what a DAW's LCD says, and it is
right for placing an edit. But this app's input is a sung line, and the question asked of
a playhead here is at least as often "what note is that?" — which the roll answers only if
you can find the playhead on it. Clicking the readout switches. It is one panel rather
than three controls near each other for the same reason Logic's LCD is one panel: position
and tempo are read together.

**Keys and History hide by removing their grid track, not by `display: none`.** A hidden
panel that still holds its space is not hidden, and this layout is explicit rows and
columns — an explicit grid row exists whether or not anything is in it, so the row goes
too (`.editor--no-keyboard`, `.editor--no-history`).

**Import moved to the toolbar, out of the inspector's Import/Export group.** Bringing
material in is a top-level act like opening a project; exporting is something you do to a
track you are looking at. The preview-then-import sequence — which is what makes a PPQ
rescale an announcement rather than a surprise — is now in `importSmf.ts` so there is one
of it.

### Listening happened in the smallest space on screen

The flow was: press Listen, get a strip along the bottom of the window, perform, press
stop, *then* get a full-window editor. The least reversible part of the whole feature —
the performance, which cannot be re-run without doing it again — had the least room, and
the part you can redo endlessly had the most.

It is one continuous activity: you play something, you look at what came back, you fix it
or you do it again. So it is one surface for the duration and the stage inside it changes.
`ListenOverlay` is the frame; `ListenCapture` and `TranscribeEditor` are the two stages.

`pending` deliberately outlives the overlay. Closing it puts the take back in the review
bar rather than throwing it away, so "let me look at the roll first" is not a decision to
discard.

### You could hear the recording but not the result

Phase 9 added take playback and its own comment argued for it: hearing the sampler play
the transcription tells you what the *transcriber* heard, hearing the take tells you what
you played. Both halves are true and the conclusion drawn from them was half right. The
notes are the thing being decided about. Offering only the recording meant the one way to
hear the actual result was to accept it and find out — which is the wrong order for a
feature whose entire premise is "look before it lands".

So both play, MIDI is the default, and `Both` exists because the comparison is the point.

**Why a scheduler thread and not the sequencer.** The sequencer is driven by the audio
thread and owns one timeline — the project's. Auditioning a proposal through it would mean
swapping that timeline out and putting it back, with the transport's position and the
playhead events going somewhere strange in between, to play four bars. Instead
`unplugged_core::audition` places the note boundaries in seconds (pure, tested) and
`src-tauri/src/audition.rs` walks them against a wall clock, sounding them on the sampler
by the same path as the on-screen keyboard.

That trades sample accuracy for independence. It is the right trade here and nowhere else:
nothing in this path is on the audio thread, and a couple of milliseconds of jitter is
inaudible in a phrase you are listening to in order to decide whether a note is wrong. It
would not be an acceptable trade for playback of the project, which is why this is a
separate module rather than a second way to play.

Details worth knowing:

- **Cancellation is a counter, not a flag.** `stop` immediately followed by `play` must not
  let the outgoing thread's next boundary land inside the incoming run. The thread checks
  its generation every 2 ms, which is also how long a stop takes to silence a held note.
- **The thread releases what it sounded.** Not `all_notes_off`, which would also kill live
  keyboard notes.
- **The recording's clock wins when it is playing.** `capture_audition_position` prefers
  the preview player's position and falls back to the scheduler's, because the sample clock
  is the one the ear is following when both are running.
- **A partially available source is not an error.** Off-Apple the preview player refuses and
  the sampler is silent; asking for `Both` there still succeeds if the notes were scheduled,
  because the transcription is perfectly reviewable on a machine that cannot play it.
- **The tempo is not recomputed.** Rust derives ticks-per-second from
  `capture.analysis.tempo_bpm` — the tempo the notes were actually placed with — which is
  the same number the editor draws them against. A second derivation would drift.

`capture_preview_play/stop/position` are gone; `capture_audition_play/stop/position`
replace them, with a `source` of `midi` | `take` | `both`.

### What was verified

- **315 Rust tests** (up from 302), clippy clean, `npm run build` clean, both Apple targets
  compile-check.
- New tests cover the audition schedule — a note straddling the start point keeps its
  remainder, a release sorts before a retrigger at the same instant, a nonsense tempo
  schedules nothing — and the run bookkeeping: a stop makes the running generation stale,
  a late-finishing run does not clear a later one's clock, a run past its end reports no
  position.
- The toolbar, its toggles and both overlay stages were driven in the browser preview at
  1440 and 880 px, with the console watched for errors. The mock was faked *temporarily*
  to render the overlay and reverted — `mockBackend.ts` still refuses transcription on
  purpose, and should stay that way.

### Not verified — needs a Mac and a microphone

1. **Whether the MIDI audition makes a sound at all.** The scheduler reaches the sampler
   through `AudioEngine::note_on`, the same call the on-screen keyboard makes, so if keys
   sound this should. But it has only ever run against the null backend.
2. **Whether the timing is good enough.** Wall-clock scheduling with a 2 ms poll should
   place a note within a few milliseconds. If it audibly stutters under load, the fix is
   not a smaller poll — it is to render the pending notes through the preview player as
   audio, which is a bigger change.
3. **`Both`, together.** The two players are started one after the other from the same
   command, so they may be a few milliseconds apart. Whether that reads as "in sync" or as
   flam is a question for ears.
4. **Whether stopping ever leaves a note hanging.** The thread releases what it holds
   within 2 ms of a cancellation; Panic is still there if it does not.
5. **The transport glyphs.** `⏸` and `⏹` do not render in the Linux preview's fonts. `⏹`
   and `⏮` were already in use and presumably rendered on macOS, so `⏸` should too — if it
   comes out as a box, that is a font fallback, not a bug.

### What a human should test manually

- [ ] Play, pause, play again — the playhead resumes where it stopped. Stop returns to the
      top.
- [ ] `Space` still plays and pauses; it does nothing while the listen overlay is up.
- [ ] Click the clock readout: it swaps between `1.1.000` and the note under the playhead.
      Play through a phrase and confirm the note name follows.
- [ ] Nudge the tempo with − and +, and type into the field. Confirm playback follows.
- [ ] Toggle Keys and History off: nothing is left holding empty space, and the roll grows
      into it.
- [ ] Import from the toolbar. Confirm a rescale warning still appears for a file at a
      different PPQ.
- [ ] Press Listen: the overlay opens *immediately* and the waveform grows as you sing.
- [ ] Stop & transcribe: the same window becomes the review stage without a flash of the
      editor behind it.
- [ ] Press Play there with **Notes** selected — the sampler plays the transcription.
- [ ] Switch to **Recording** while it is playing: it continues from the same place with
      the take instead.
- [ ] Switch to **Both** and judge whether they line up.
- [ ] Click partway along the waveform: playback starts from there, in the selected source.
- [ ] Drag a note while it is playing; confirm nothing hangs.
- [ ] Close the overlay with ✕ — the take is still offered in the review bar, and
      "Fine-tune…" brings the overlay back with the same notes.
- [ ] Add to track, then undo. One step.
- [ ] Transcribe an audio *file* from the inspector — it should open the same overlay at
      the review stage.

---

## Six corrections to the listen flow

All six came from using it. They divide into one thing that was hidden, three things the
fine-tune stage could not do, and two keys that meant two things at once.

### The wait was invisible, so it looked like a failure

Pressing "Stop & transcribe" closed the overlay, left the editor on screen for several
seconds, and then reopened the overlay with a result. Every part of that is wrong: the
window that came back was not what you were doing, the gap read as a dropped take, and
nothing said the machine was busy.

The overlay now has a third stage between capture and review, and it does not close in
between. The bar in it is **real** — `transcribe_reporting` takes a callback and the
analysis publishes a fraction as it goes:

- **The split is by cost, not by pipeline stage.** YIN over every frame is 65% of the
  work and spectral flux is most of the rest, so those two get the bar between them; the
  tempo estimate and the assembly are a pass each over one value per frame and finish
  before the eye can see them. A bar apportioned by stage would sit at 40% for four
  seconds.
- **It reports every sixteenth frame, not every frame.** The callback crosses into an
  atomic store; the caller may one day do more.
- **A take too short to analyse still completes the bar**, or the overlay would sit at
  zero forever on a take of nothing. There is a test for exactly that.
- The progress lives in an `AtomicU32` in `AppState` rather than behind the capture lock,
  because the analysis holds nothing while it runs and a progress read must not wait on
  it.

`transcribe` is now a wrapper over `transcribe_reporting` with an empty callback, and a
test asserts the two produce identical transcriptions — a reporting path that changed the
answer would be worse than no bar at all.

### Two notes could sound at once, which the source could not have done

A transcription is monophonic by construction: the segmenter walks one frame track, so
its notes cannot overlap. Two things broke that afterwards. **Quantisation** rounds a
start backwards and a length up to a whole grid step, and two sixteenths played slightly
ahead of the beat land on top of each other. **Dragging** in the fine-tune stage can put a
note anywhere at all.

Either way the result is a lie about what was performed. `unplugged_core::monophony` is
the rule, in one place:

- A note running into the next attack is **cut there** — the later attack wins, which is
  what a monophonic instrument does.
- Two notes at the same instant leave **the longer one**. A short note on the same attack
  is far more often an artefact than a real event, and the sort's tie-break is what
  encodes that.
- A note left with nothing is **dropped**, not kept at zero length, which is
  unrepresentable in SMF anyway.

It is applied after quantisation in the transcriber and in `capture_set_notes`, which is
why that command now returns **the notes it kept** rather than a count: the editor must
draw what Rust decided, or the next drag is computed against notes that no longer exist.
The transcriber needs the same decision over `DetectedNote`, which carries the analysis
behind each note, so the rule is also exposed as `flatten_indexed` — kept index and new
duration — and a test asserts the two forms agree.

### The dials were constants, and every one of them was a guess

`MIN_CONFIDENCE`, `SILENCE_FLOOR`, `MIN_NOTE_FRAMES`, `PITCH_BREAK_SEMITONES` and the
onset thresholds are all judgements about the source: how percussive it is, how steady the
singer's pitch is, how much room is in the recording. The defaults suit a hummed line at a
laptop. A plucked string or a breathy voice wants something else, and getting it wrong
produces a *plausible* result rather than an obviously broken one — which is the case
worth exposing rather than tuning once and hiding.

`TranscribeTuning` carries the five, named for the symptom rather than the stage: nobody
looking at a bad transcription thinks "the spectral flux threshold is too high", they
think "it heard one note where I played two".

- **One dial moves all three onset thresholds together**, multiplicatively:
  `scale = 2^(1 - 2s)`. Monotone, nothing can cross zero, and `s = 0.5` reproduces
  `OnsetParams::default()` *exactly* — tested, because a default that missed would
  silently change what every take transcribes to.
- **Rust clamps everything**, including NaN, which `f32::clamp` panics on. These arrive
  from the webview; a minimum note of zero is not a crash, it is ten thousand notes, which
  looks like a broken transcriber rather than a bad setting.
- The tuning **comes back inside the preview**, so the controls open showing what actually
  produced what is on screen, and it is carried to the next take — a setting you had to
  find once should not need finding again.
- Moving one **re-reads the take already in memory**. That is what retaining the audio was
  for. Changes are sent on release rather than per pixel of a drag, because each one is
  seconds of work.

### Three things the fine-tune stage could not do

**You could not hear what you were dragging.** Correcting a transcription is an ear job
and the note under the cursor is the one being judged. It now sounds when grabbed, and
again on each semitone crossed — on each semitone, not each pointer frame, or a drag is a
siren.

**Delete needed the mouse.** `⌫` and `Delete` now remove the selected note.

**Join.** A held note the analysis broke in two — at a vibrato wobble, or a slur it read as
an attack — is the single most common thing wrong with a result, and there was no way to
put it back together. `J` is now bound in both places, with the semantics each one can
support:

- In the **piano roll**, where there is a marquee, it merges the whole selection into one
  note spanning the first attack to the last release.
- In the **fine-tune stage**, which selects one note at a time, it merges the selected
  note into the one after it. Repeating the key walks along a note that came back in four
  pieces. Adding a marquee there to make the two identical would be a bigger change than
  the problem needs.

Either way the **pitch of the earliest note wins**: the note you meant is the one that
started, and the rest are fragments. The command is `Delete` + `Insert` in one
transaction rather than a `Replace` plus a `Delete`, because a transaction's commands
apply in order and indices in a later one would refer to a list the earlier one has
already re-sorted. One undo step, tested.

`J` is unmodified rather than `⌘J`, which is the window manager's on macOS — and because
Join is an editing verb like Record and Listen, which are also bare letters here.

### Two keys meant two things, and the loser depended on where focus was

`L` was Listen and also D on the on-screen keyboard. `J` would have been Join and also B.
`S` is a white key. Three separate `window` keydown listeners each guarded this ad hoc,
and which one won was a question about focus rather than about intent.

Typing on the piano is now **a mode**. While it is on, the letter keys play notes and
every editor shortcut is suspended; while it is off, nothing is bound to the keyboard at
all and the keys still work with the mouse. There is no in-between, because any clever
arrangement would mean one of the two silently losing.

- The listener is **not bound** unless the mode is on. Guarding inside the handler would
  still swallow auto-repeat and `preventDefault` from keys the editor wanted.
- **`Esc` leaves**, and is read before the mode's own suspension — otherwise the only way
  back would be the mouse.
- **Leaving releases whatever is held**, since the keyup handler goes with the mode that
  was holding it.
- The panel is **outlined while it is on**. "Why did Space stop playing?" needs an answer
  on screen, not in a release note.
- The mode holds but does not listen while the listen overlay is up, since the overlay
  covers the keys.
- `PianoRoll` takes a `shortcutsSuspended` prop rather than reaching for a module-level
  flag. The editor owns the state and both modes feed it; a side channel here would be
  the same mistake as a side channel to the backend.

### What was verified

- **341 Rust tests** (up from 315), clippy clean, `npm run build` clean, both Apple
  targets compile-check.
- New tests: the monophonic rule in seven cases including a quantisation pile-up; progress
  monotone from zero to one, and one for a take too short to analyse; the default tuning
  landing exactly on the onset parameters it replaced; each dial moving the result in the
  direction it claims; nonsense dials clamped; join spanning, closing gaps, undoing as one
  step, and surviving indices from a stale selection.
- The confidence dial's test is against a signal with noise added by a small
  deterministic LCG, because a synthetic sine satisfies any threshold and would have
  proved nothing.
- In the browser preview: typing mode on and off via `Esc`, the progress stage reporting a
  real fraction with the overlay staying put, the five dials, and select → `J` → `⌫` in
  the fine-tune stage. The mock was faked temporarily and reverted; `mockBackend.ts` still
  refuses transcription on purpose.

### Not verified — needs a Mac and a microphone

1. **Whether the progress bar is smooth on a real take.** It has only run against a mock
   that advances on a timer. The shape to watch for is a stall around 65%, which would
   mean the flux stage costs more than the third of the bar it has been given.
2. **Whether dragging by ear is pleasant.** The blip is the same 180 ms audition the piano
   roll uses. On a fast drag across an octave that is twelve of them.
3. **Whether the defaults are right.** Now that the dials exist, the interesting question
   is which one a real bad result needs — that is a question for takes, not for tests.
4. **Whether joining forwards is the right default in the fine-tune stage.** It is the
   direction that fits "the analysis split this", but the first time it eats a note you
   wanted, it is wrong.

### What a human should test manually

- [ ] Record a long take. The overlay stays up, the bar moves, and the review stage
      arrives without the editor ever showing through.
- [ ] Cancel during the wait — nothing is left running and no take is committed.
- [ ] Drag a note up and down and confirm it sounds at each semitone, not continuously.
- [ ] Drag one note on top of another. The result is still one note at a time, and the
      picture matches what plays.
- [ ] Select a note, `⌫`. Then select another and press `J` — it should swallow the one
      after it.
- [ ] Quantise a fast run to 1/16 and confirm nothing overlaps.
- [ ] Open Analysis and pull "Split repeated notes" to each end. More notes one way,
      fewer the other, and the take is never re-recorded.
- [ ] Reset, and confirm the result matches what you first got.
- [ ] In the roll: select several notes, press `J`, confirm one note from first attack to
      last release, and that one undo puts them all back.
- [ ] Turn Typing on. `L` plays a note instead of opening Listen; `Space` does nothing;
      the panel is outlined. `Esc` gives everything back.
- [ ] Hold a key, click Typing off with the mouse, and confirm the note stops.

### Two files came off the debt register on the way past

The 700-line rule says to split a file on the register when you touch it substantially,
and two of these changes did:

- **`command.rs`** (837 → 984 with the join command) is now a directory along the seam it
  already had: `command/mod.rs` holds the session and its history, `command/edits.rs` the
  gestures that were an inline `pub mod edits`, `command/tests.rs` the tests.
- **`unplugged-transcribe/src/lib.rs`** (769 → 1089 with the tuning and the progress
  callback) keeps the pipeline and sheds its tests to a sibling `tests.rs`.

`PianoRoll.tsx` grew by seven lines — the `shortcutsSuspended` prop and its guard — and is
still on the register at ~755. That is not a substantial touch and it was not split;
saying so here is the alternative to pretending it did not happen.

---

## The Keychain prompt on every launch

**Reported:** "unplugged wants to use your confidential information stored in
'com.unplugged.daw' in your keychain" on every open. It was a real bug, and it violated
this project's own stated rule.

### What was happening

`PromptBar` asks `ai_status()` when it mounts, which is every time a project opens — it
needs to know whether to show "Set up AI". `ai_status` called `has_api_key()` and
`key_hint()`, and *both* of those called `api_key()`, which is a full
`SecItemCopyMatching` with `kSecReturnData`.

So the app decrypted the user's Anthropic API key twice at launch, to render a boolean and
four characters, before being asked to do anything. macOS consults an item's access
control list when the **secret** is released, so that read is exactly what raises the
dialog — and because the dialog says "macOS" rather than "this line of code", it read as
an OS quirk rather than as the app doing something it should not.

README-TECHNICAL has said since Phase 6 that the key "is read only inside `unplugged-ai`,
immediately before a request". `ai_status` is not immediately before a request. The rule
was right; the code had drifted from it, and nothing failed when it did.

### The fix

**Status is answered from the item's attributes, which never releases the secret.**

- `unplugged_platform_keychain_has` — `SecItemCopyMatching` with `kSecReturnAttributes`
  and no `kSecReturnData`. Existence, silently.
- `unplugged_platform_keychain_hint` — the last four characters, read from
  `kSecAttrComment`, where `set` now writes them.

Storing the hint as an attribute is the part worth arguing about, and it holds: those four
characters were *already* designed to cross into the webview. Keeping them where they can
be read without decrypting anything is strictly less exposure than decrypting the whole key
to derive them, which is what happened before. A key stored by an older build has no
comment, so its hint comes back `None` and the panel shows "stored" — which it already did
for that case.

`unplugged_platform_keychain_get` is now the only thing in the app that can prompt, and it
is reached from exactly two places, both immediately before an HTTPS request:
`propose_edit` and `available_models`.

**There is a test, because nothing fails if this drifts back.** It reads `Keychain.swift`,
strips the comments, and asserts `kSecReturnData` appears exactly once and inside the
getter. Same shape as `the_three_places_that_name_the_shared_path_agree`, and for the same
reason: the compiler cannot connect these two files, and the symptom of them disagreeing is
a system dialog nobody traces back to a commit.

### What this does not fix, and should not

Opening **Settings** with a key set still reads it, because it lists the available models —
that is a real request, and a prompt there is the Keychain working. Pressing **⌘K** and
describing an edit will prompt the first time too.

What *will* still prompt more than once is a rebuild. A Keychain ACL trusts a specific code
signature, and an ad-hoc-signed build gets a new one every time it is built, so "Always
Allow" only holds until the next `install-plugin.sh`. That is a consequence of the
no-paid-team decision recorded in Phase 10, not of this code, and it goes away with a
stable signing identity.

### Not verified — needs a Mac

None of the Swift compiles here. In rough order of risk:

1. **Whether an attributes-only query really is silent.** This is the mechanism the whole
   fix rests on, and it is remembered rather than measured: macOS gates the ACL on
   releasing `kSecValueData`, so a `kSecReturnAttributes` query should not prompt. If the
   dialog still appears at launch, that premise is wrong and the hint has to move to
   `ai.json` instead, leaving `has` as the only Keychain call.
2. **`unplugged_platform_keychain_set` gained a third parameter.** Both sides of the ABI
   are updated in this commit, but a mismatch here is a link error at best.
3. **`kSecAttrComment` on a generic password.** Believed available on both platforms. If
   `SecItemAdd` starts returning `errSecParam` (-50) after this, that attribute is the
   first thing to drop.
4. Whether an existing key survives. It should — nothing touches the stored item until the
   next `set` — but its hint will read "stored" until the key is entered again.

### What a human should test manually

- [ ] Open the app with a key already stored. **No Keychain dialog.**
- [ ] Confirm the prompt bar still knows a key is set (no "Set up AI" button).
- [ ] Open Settings. A dialog here is expected and correct; choose Always Allow.
- [ ] The key row shows "stored" rather than the last four — that is the old item having no
      comment. Re-enter the key and confirm it becomes "…abcd".
- [ ] Quit, reopen: still no dialog at launch, and the hint persists.
- [ ] Clear the key in Settings, confirm the prompt bar offers "Set up AI" again.
- [ ] In Keychain Access, confirm the item is now labelled "Unplugged — Anthropic API key"
      rather than only by its service.

---

## One bar, not two

The last change split the controls by scope: the toolbar took what concerns the project,
the bar under the roll kept what makes notes. That was a better rule than the one before
it and still the wrong shape, because the split it produced was invisible. Someone hunting
for Loop does not know it arrived with the transport rather than with the toolbar, and
"which row is this in?" is a question about the project's history rather than about the
work.

So all of it is in the toolbar, arranged by what it does:

- **Left** — what is on screen (Keys, History) and the history you can walk back through
  (undo/redo, as icons: they are pressed by muscle memory and never read).
- **Centre** — the transport, both ways of making notes (Record, Listen), Loop and Click,
  and the clock they all run against.
- **Right** — what is open (Import, Console, Settings) and what becomes of the result
  (Panic, Save).

`Transport.tsx` is gone rather than reduced to a wrapper. What is left below the roll is
the prompt bar and the keys — the two surfaces you *type* into, which is a different kind
of thing from a button, and the reason those two did not follow the rest up. The roll
gained the ~56px the transport row was holding.

### The bar has to give way in a defined order

Everything in one row is a lot of row. Rather than let whatever happens to be last get
clipped, the order is set and it drops what is repeated or inferable elsewhere first:

| Below | Goes | Because |
|---|---|---|
| 1500px | the project name | it is also in the picker, and this bar is for controls |
| 1460px | the build stamp | "is this my fix?" is not a small-window question — and a 1440 window, the common one, should have slack rather than fit exactly |
| 1180px | the time signature | the clock keeps bar·beat and tempo, which is what is read while playing |
| 1040px | *nothing* | the bar becomes two rows: transport centred above, the rest split beneath |

Two rows rather than a scrolling bar because a control you have to scroll to is a control
you will not find. The editor's toolbar row is `auto`, so the layout below simply starts
lower; `--h-transport` is gone from the tokens, since nothing has a fixed height here any
more.

### What was verified

- Measured at 1600, 1440, 1200 and 1000 px in the browser preview: nothing clipped
  (`scrollWidth === clientWidth` at every width), and Listen, Loop, Click, Panic, Undo,
  Redo, Import and Console all present and visible at each. The 1000px case wraps to two
  rows at 89px tall.
- Hiding the keys leaves no empty row behind — the bottom bar collapses to the prompt bar
  and ends flush with the window.
- 343 Rust tests, clippy clean, `npm run build` clean, both Apple targets compile-check.

### What a human should test manually

- [ ] Record, Listen, Loop and Click all work from the toolbar exactly as they did below.
- [ ] Undo/redo icons: hover shows what will be undone, and they grey out with nothing to
      undo.
- [ ] Save still lights up when the project is dirty, and Panic still silences a stuck
      note.
- [ ] Resize the window down past 1040px and confirm the bar splits into two rows rather
      than losing a control.
