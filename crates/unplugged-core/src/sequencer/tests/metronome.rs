//! The click: where it lands, what pitch it is, and that it always gets released.

use super::*;

#[test]
fn the_metronome_clicks_on_every_beat() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default()); // 4/4
    s.set_metronome(true);
    s.play();

    // 120 bpm, 480 ppq, 48 kHz: a beat is 24000 samples. One bar = 96000.
    let mut out = Vec::new();
    s.render(96_000, &mut out);

    let offsets: Vec<u32> = clicks(&out).iter().map(|(o, _)| *o).collect();
    assert_eq!(offsets, vec![0, 24_000, 48_000, 72_000]);
}

#[test]
fn the_downbeat_is_a_different_pitch_from_the_other_beats() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    s.play();

    let mut out = Vec::new();
    s.render(96_000, &mut out);

    let pitches: Vec<u8> = clicks(&out).iter().map(|(_, p)| *p).collect();
    assert_eq!(
        pitches,
        vec![
            METRONOME_DOWNBEAT_PITCH,
            METRONOME_BEAT_PITCH,
            METRONOME_BEAT_PITCH,
            METRONOME_BEAT_PITCH
        ]
    );
}

#[test]
fn the_bar_length_follows_the_time_signature() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::new(3, 4).unwrap());
    s.set_metronome(true);
    s.play();

    let mut out = Vec::new();
    s.render(6 * 24_000, &mut out); // two bars of 3/4

    let downbeats: Vec<u32> = clicks(&out)
        .iter()
        .filter(|(_, p)| *p == METRONOME_DOWNBEAT_PITCH)
        .map(|(o, _)| *o)
        .collect();
    assert_eq!(downbeats, vec![0, 3 * 24_000], "a downbeat every three beats");
}

#[test]
fn a_beat_on_a_buffer_boundary_clicks_exactly_once() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    s.play();

    // Buffers of exactly one beat put every beat on a seam.
    let mut total = 0;
    let mut out = Vec::new();
    for _ in 0..4 {
        s.render(24_000, &mut out);
        total += clicks(&out).len();
    }
    assert_eq!(total, 4, "no beat may be doubled or dropped at a seam");
}

#[test]
fn the_metronome_is_silent_when_disabled() {
    let mut s = seq();
    s.set_metronome(false);
    s.play();

    let mut out = Vec::new();
    s.render(96_000, &mut out);
    assert!(clicks(&out).is_empty());
}

#[test]
fn every_click_is_released() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    s.play();

    let mut ons = 0;
    let mut offs = 0;
    let mut out = Vec::new();
    for _ in 0..40 {
        s.render(4_800, &mut out); // 192000 samples total, two bars
        ons += out.iter().filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOn).count();
        offs += out.iter().filter(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOff).count();
    }
    assert!(ons >= 8, "expected at least two bars of clicks, got {ons}");
    assert_eq!(ons, offs, "every click must be released or the sampler stacks voices");
}

#[test]
fn stopping_mid_click_releases_it() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    s.play();

    let mut out = Vec::new();
    s.render(64, &mut out); // the click at beat 0 starts but has not ended
    assert_eq!(clicks(&out).len(), 1);

    s.stop(&mut out);
    assert!(
        out.iter().any(|e| e.track == METRONOME_TRACK && e.kind == EventKind::NoteOff),
        "a click ringing at stop must be released"
    );
}

#[test]
fn clicks_keep_firing_across_a_loop_wrap() {
    let mut s = seq();
    s.set_time_signature(TimeSignature::default());
    s.set_metronome(true);
    s.set_loop_region(Some((0, 960))); // two beats
    s.play();

    let mut out = Vec::new();
    s.render(96_000, &mut out); // two full loop passes

    // Beats at 0 and 24000 within each 48000-sample pass.
    assert_eq!(clicks(&out).len(), 4, "the click must survive the wrap");
}

