//! The audio-thread path: following the host, and never overrunning the buffer.

use super::*;

#[test]
fn rendering_follows_the_host_and_emits_the_projects_notes() {
    let dir = temp_dir("render");
    let id = seeded(&dir, "Render");

    let mut plugin = Plugin::new(dir.clone());
    plugin.prepare(48_000.0);
    plugin.open(&id).unwrap();

    let mut out = blank();
    // 120 bpm, 48 kHz: a bar is two seconds, so walk a second of 512-frame blocks and
    // the note at tick 0 must fire in the first one.
    let mut total = 0;
    let mut first_note_on = None;

    for block in 0..94 {
        let beats = (block * 512) as f64 / 48_000.0 * 2.0;
        let count = plugin.render(
            HostTransport { beats, tempo_bpm: 120.0, playing: true },
            512,
            &mut out,
        ) as usize;
        for event in &out[..count] {
            if event.kind == 1 && first_note_on.is_none() {
                first_note_on = Some((block, event.pitch));
            }
        }
        total += count;
    }

    assert!(total > 0, "the project's notes never fired");
    let (block, pitch) = first_note_on.expect("a note-on");
    assert_eq!(pitch, 60);
    assert_eq!(block, 0, "the note at tick 0 belongs in the first block");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_stopped_host_produces_nothing() {
    let dir = temp_dir("stopped");
    let id = seeded(&dir, "Stopped");

    let mut plugin = Plugin::new(dir.clone());
    plugin.prepare(48_000.0);
    plugin.open(&id).unwrap();

    let mut out = blank();
    for _ in 0..20 {
        let count = plugin.render(
            HostTransport { beats: 0.0, tempo_bpm: 120.0, playing: false },
            512,
            &mut out,
        );
        assert_eq!(count, 0);
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rendering_with_nothing_open_is_silent_rather_than_a_crash() {
    let dir = temp_dir("empty");
    let mut plugin = Plugin::new(dir.clone());
    plugin.prepare(44_100.0);

    let mut out = blank();
    for block in 0..10 {
        let count = plugin.render(
            HostTransport { beats: block as f64, tempo_bpm: 120.0, playing: true },
            512,
            &mut out,
        );
        assert_eq!(count, 0);
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_output_buffer_is_never_overrun() {
    let dir = temp_dir("overrun");
    let store = ProjectStore::new(dir.join("projects"));
    let manifest = store
        .create("Dense", 120.0, TimeSignature::new(4, 4).unwrap())
        .unwrap();

    // Far more simultaneous notes than the caller's buffer can hold.
    let mut project = store.load(&manifest.id).unwrap();
    project.tracks[0].notes = (0u8..=127)
        .map(|pitch| Note::new(pitch, 0, 480, 100, 0).unwrap())
        .collect();
    store.save(&mut project).unwrap();

    let mut plugin = Plugin::new(dir.clone());
    plugin.prepare(48_000.0);
    plugin.open(&manifest.id).unwrap();

    let mut small = vec![
        CRenderedEvent {
            frame_offset: 0,
            track: 0,
            kind: 0,
            pitch: 0,
            velocity: 0,
            channel: 0,
            _pad: [0; 2],
        };
        8
    ];
    let count = plugin.render(
        HostTransport { beats: 0.0, tempo_bpm: 120.0, playing: true },
        512,
        &mut small,
    );
    assert!(count as usize <= small.len(), "wrote past the caller's buffer");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_host_tempo_wins_over_the_projects() {
    let dir = temp_dir("tempo");
    let store = ProjectStore::new(dir.join("projects"));
    let manifest = store
        .create("Tempo", 90.0, TimeSignature::new(4, 4).unwrap())
        .unwrap();
    let mut project = store.load(&manifest.id).unwrap();
    project.tracks[0].notes = vec![Note::new(60, 960, 480, 100, 0).unwrap()];
    store.save(&mut project).unwrap();

    let mut plugin = Plugin::new(dir.clone());
    plugin.prepare(48_000.0);
    plugin.open(&manifest.id).unwrap();

    // The project says 90; the host says 140. Inside a host, the host is the truth —
    // a plugin that kept its own tempo would drift against everything else.
    let mut out = blank();
    plugin.render(
        HostTransport { beats: 0.0, tempo_bpm: 140.0, playing: true },
        512,
        &mut out,
    );
    assert!((plugin.last_tempo - 140.0).abs() < 1e-9);

    std::fs::remove_dir_all(&dir).ok();
}

