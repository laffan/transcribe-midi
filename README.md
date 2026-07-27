# Unplugged

**Hum it. See the notes. Say what to change.**

Unplugged is a small music app built around two ideas:

1. **Turn what you play into MIDI.** Sing, hum, whistle or play a line — one note at a
   time — and it becomes editable notes. The recording is kept alongside the notes, so you
   fine-tune against what you actually performed, not against your memory of it.

2. **Change those notes by describing what you want.** "Harmonise a third above." "Make it
   swing." "Add a ii–V–I under this." An AI reads the track, proposes an edit, and shows
   you the difference — green for new, red for going, amber for moving — before anything
   is applied. You accept it or you don't, and either way it is one undo away.

Everything else in the app — the piano roll, the transport, the little keyboard — exists to
let you *check the work* of those two features and fix the last five percent by hand.

## Why

Most ideas for a melody don't arrive at a desk. They arrive as a hum on a walk, a voice
memo, a line played once and half-forgotten. The distance between that moment and "notes I
can actually use in my music software" is the problem Unplugged wants to close: capture the
idea wherever it lands, shape it by talking about it, and hand clean MIDI to the tools you
already use.

## Where it runs

- **Standalone** on macOS (Apple silicon), with iOS intended.
- **Inside your DAW** as an AUv3 plugin — Unplugged appears as a MIDI effect in Logic Pro
  or Ableton Live, follows the host's transport and tempo, and plays the projects you made
  in the app straight into any instrument you put after it.
- Every project also exports as ordinary **MIDI files** you can drag anywhere.

## Goals

- Make transcription and description-driven editing feel *central*, not bolted on.
- Never surprise the user: every automated change is previewed as a diff and lands as a
  single undo step.
- Keep your work yours: projects are plain files on your machine; the AI key lives in the
  system Keychain; nothing leaves the device except the notes you explicitly send to the
  model.

## Status

Working today: projects, playback, recording (MIDI keyboard or on-screen keys),
audio-to-MIDI transcription with a waveform fine-tuning editor, AI edits with diff
preview, MIDI import/export, and a first-pass AUv3 plugin that builds, installs and
registers. Not built yet: notation view, polyphonic transcription.

The code is organised so the folder structure is the architecture: every area of the app is
a directory of small, single-purpose modules, and no file in the repo is longer than 700
lines. README-TECHNICAL.md says why, and how to keep it that way.

## More

- [README-TECHNICAL.md](./README-TECHNICAL.md) — architecture, coding standards, and the
  rules of the road for anyone (human or agent) working on the code.
- [DECISIONS.md](./DECISIONS.md) — the running log of what was decided, what was verified,
  and what still needs a human with a Mac.
