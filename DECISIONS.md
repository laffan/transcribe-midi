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
