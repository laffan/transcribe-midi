//! BPM and SMF's tempo encoding, in both directions.
//!
//! Kept as a pair in one file because they are inverses: a change to either without the
//! other silently shifts every exported tempo, and the test that catches that has to see
//! both.

use midly::num::u24;

/// Microseconds per quarter note, as SMF's tempo meta event expresses it.
pub fn bpm_to_micros_per_quarter(bpm: f64) -> u24 {
    let micros = (60_000_000.0 / bpm).round().clamp(1.0, 0xFF_FFFF as f64);
    u24::new(micros as u32)
}

/// The inverse, for import.
pub fn micros_per_quarter_to_bpm(micros: u32) -> f64 {
    if micros == 0 {
        return crate::model::DEFAULT_TEMPO;
    }
    60_000_000.0 / micros as f64
}
