//! What this crate does not claim to do.

use super::*;

#[test]
fn a_chord_is_not_pretended_to_be_understood() {
    // Three simultaneous pitches. The contract is that this yields a monophonic line
    // — never three notes at once — because the UI promises one note at a time and
    // silently returning a wrong chord would be worse than returning less.
    let samples = (SAMPLE_RATE * 1.0) as usize;
    let signal: Vec<f32> = (0..samples)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            0.3 * ((2.0 * PI * 261.6 * t).sin()
                + (2.0 * PI * 329.6 * t).sin()
                + (2.0 * PI * 392.0 * t).sin())
        })
        .collect();

    let result = transcribe(&signal, options());
    let simultaneous = result
        .notes()
        .windows(2)
        .filter(|pair| pair[0].start_ticks == pair[1].start_ticks)
        .count();
    assert_eq!(simultaneous, 0, "no two notes may start together");
}
