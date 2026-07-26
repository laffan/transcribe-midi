//! Tempo estimation from the onset envelope.
//!
//! Autocorrelation of the flux, not of the onset *times*: inter-onset intervals are
//! sparse and a single missed onset throws a histogram badly off, while the envelope
//! carries every attack whether or not it survived peak picking.
//!
//! What this cannot do is tell 60 from 120 bpm — they have the same beat positions, and
//! the difference is a matter of how the player counts. The octave is resolved by
//! preferring the range people actually work in.

/// Tempo range considered. Beyond these a beat is being counted at the wrong level.
pub const MIN_BPM: f64 = 50.0;
pub const MAX_BPM: f64 = 208.0;

/// How close to the best score a shorter lag must come to be preferred over it.
///
/// Autocorrelation cannot distinguish a period from its multiples — every pulse that
/// lines up at lag L also lines up at 2L — so half-time scores as well as the real
/// tempo and the choice between them is arbitrary on noise. Taking the shortest lag
/// that scores nearly as well picks the fundamental, which is the beat.
const OCTAVE_TOLERANCE: f32 = 0.75;

/// Half-width, in frames, of the smoothing applied before correlating.
///
/// The beat period is almost never a whole number of frames — at a 10 ms hop, 160 bpm is
/// 37.5 — so successive beats land on alternating frames and a correlation at the true
/// lag catches only half of them, while the lag at *twice* the period catches all of
/// them and wins. Smoothing by a couple of frames gives each beat a little width, so the
/// fundamental correlates properly. It doubles as tolerance for a human player, whose
/// beats are not on a grid either.
const SMOOTH_FRAMES: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoEstimate {
    pub bpm: f64,
    /// 0–1. Low means the envelope had no clear periodicity, and the caller should
    /// prefer the project's own tempo.
    pub confidence: f32,
}

/// Estimate tempo from a flux envelope sampled at `frames_per_second`.
pub fn estimate(flux: &[f32], frames_per_second: f64) -> Option<TempoEstimate> {
    if flux.len() < 16 || frames_per_second <= 0.0 {
        return None;
    }

    // Centre the envelope so the autocorrelation measures periodicity rather than the
    // mean level, which would otherwise swamp it and make every lag look equally good.
    let mean = flux.iter().sum::<f32>() / flux.len() as f32;
    let centred: Vec<f32> = flux.iter().map(|value| value - mean).collect();

    let centred = smooth(&centred, SMOOTH_FRAMES);
    let energy: f32 = centred.iter().map(|v| v * v).sum();
    if energy <= f32::EPSILON {
        return None;
    }

    let min_lag = ((60.0 / MAX_BPM) * frames_per_second).round().max(1.0) as usize;
    let max_lag = (((60.0 / MIN_BPM) * frames_per_second).round() as usize).min(flux.len() / 2);
    if min_lag >= max_lag {
        return None;
    }

    let scores: Vec<f32> = (min_lag..=max_lag)
        .map(|lag| {
            let correlation: f32 = centred[lag..].iter().zip(&centred).map(|(a, b)| a * b).sum();
            // Normalised by overlap, or long lags are penalised purely for having fewer
            // terms to sum and every recording comes out fast.
            let overlap = (centred.len() - lag) as f32;
            correlation / (energy * overlap / centred.len() as f32)
        })
        .collect();

    let best_score = scores.iter().copied().fold(f32::MIN, f32::max);
    if best_score <= 0.0 {
        return None;
    }

    // The shortest lag that comes close to the best, which is the fundamental rather
    // than one of its multiples.
    let threshold = best_score * OCTAVE_TOLERANCE;
    let mut index = scores.iter().position(|&score| score >= threshold)?;

    // That lands on the rising edge of the peak; walk to its top. Only while strictly
    // increasing, so this cannot wander off into the next peak.
    while index + 1 < scores.len() && scores[index + 1] > scores[index] {
        index += 1;
    }

    // Parabolic interpolation, because the lag grid is coarse where it matters most: at
    // a 10 ms hop the lags either side of 120 bpm are 117.6 and 122.4, so without this
    // the answer can only ever be one of a handful of values near the middle of the
    // range, and a jittered performance rounds to the wrong one.
    let refined = if index > 0 && index + 1 < scores.len() {
        let (before, at, after) = (scores[index - 1], scores[index], scores[index + 1]);
        let denominator = 2.0 * (2.0 * at - before - after);
        if denominator.abs() > f32::EPSILON {
            index as f32 + (after - before) / denominator
        } else {
            index as f32
        }
    } else {
        index as f32
    };

    let lag = min_lag as f64 + refined as f64;
    if lag <= 0.0 {
        return None;
    }

    Some(TempoEstimate {
        bpm: 60.0 * frames_per_second / lag,
        confidence: scores[index].clamp(0.0, 1.0),
    })
}

/// Triangular smoothing, `half` frames either side.
fn smooth(values: &[f32], half: usize) -> Vec<f32> {
    if half == 0 {
        return values.to_vec();
    }
    (0..values.len())
        .map(|index| {
            let mut sum = 0.0;
            let mut weight_total = 0.0;
            for offset in -(half as isize)..=(half as isize) {
                let at = index as isize + offset;
                if at < 0 || at as usize >= values.len() {
                    continue;
                }
                let weight = (half as f32 + 1.0 - offset.unsigned_abs() as f32).max(0.0);
                sum += values[at as usize] * weight;
                weight_total += weight;
            }
            if weight_total > 0.0 {
                sum / weight_total
            } else {
                values[index]
            }
        })
        .collect()
}

/// Round a tempo to something a person would type.
///
/// Always to a whole number. Lags are integers, so the reachable tempos are quantised —
/// near 120 bpm at a 10 ms hop the neighbouring lags are 117.6 and 122.4 — and the
/// fractional part is an artefact of that grid rather than a measurement. Half a beat
/// per minute is four parts in a thousand; nobody hears it, and everybody notices a
/// tempo field reading 119.6.
pub fn tidy(bpm: f64) -> f64 {
    bpm.round().clamp(MIN_BPM, MAX_BPM)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f64 = 100.0; // 10 ms hop

    /// An envelope with a spike on every beat.
    ///
    /// `jitter_frames` displaces each beat by up to that many frames, drawn from a fixed
    /// LCG rather than a repeating pattern — a repeating one would give the envelope a
    /// periodicity of its own, which is the very thing being measured.
    fn pulse_train(bpm: f64, seconds: f64, jitter_frames: i32, seed: u32) -> Vec<f32> {
        let frames = (seconds * FPS) as usize;
        let period = 60.0 / bpm * FPS;
        let mut flux = vec![0.02f32; frames];
        let mut state = seed | 1;

        let mut beat = 0;
        loop {
            let offset = if jitter_frames > 0 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 16) as i32 % (2 * jitter_frames + 1) - jitter_frames
            } else {
                0
            };

            let at = (beat as f64 * period).round() as i32 + offset;
            beat += 1;
            if at < 0 {
                continue;
            }
            if at as usize >= frames {
                break;
            }
            flux[at as usize] = 1.0;
        }
        flux
    }

    #[test]
    fn recovers_a_steady_tempo() {
        for bpm in [72.0, 90.0, 120.0, 140.0, 160.0] {
            let flux = pulse_train(bpm, 12.0, 0, 1);
            let estimate = estimate(&flux, FPS).expect("a pulse train has a tempo");
            assert!(
                (estimate.bpm - bpm).abs() < 3.0,
                "{bpm} bpm estimated as {:.1}",
                estimate.bpm
            );
        }
    }

    #[test]
    fn tolerates_a_human_amount_of_jitter() {
        // ±20 ms, which is a competent but not machine-like player.
        for seed in [1u32, 7, 99, 12345] {
            let flux = pulse_train(120.0, 20.0, 2, seed);
            let estimate = estimate(&flux, FPS).unwrap();
            assert!(
                (estimate.bpm - 120.0).abs() < 5.0,
                "seed {seed}: got {:.1}",
                estimate.bpm
            );
        }
    }

    #[test]
    fn a_flat_envelope_has_no_tempo() {
        assert!(estimate(&vec![0.5; 500], FPS).is_none());
        assert!(estimate(&[], FPS).is_none());
        assert!(estimate(&[1.0, 2.0, 3.0], FPS).is_none(), "too short to measure");
    }

    #[test]
    fn a_very_fast_pulse_is_read_at_a_countable_level() {
        // 240 bpm sixteenths are outside the range; the answer should be a tempo someone
        // could count, not a refusal or a number off the top of the scale.
        let flux = pulse_train(240.0, 12.0, 0, 1);
        let estimate = estimate(&flux, FPS).unwrap();
        assert!(
            (MIN_BPM..=MAX_BPM).contains(&estimate.bpm),
            "got {:.1}",
            estimate.bpm
        );
    }

    #[test]
    fn tidying_rounds_to_a_whole_number_within_the_range() {
        assert_eq!(tidy(119.6), 120.0);
        assert_eq!(tidy(120.4), 120.0);
        assert_eq!(tidy(133.42), 133.0);
        assert_eq!(tidy(4.0), MIN_BPM, "clamped rather than nonsense");
        assert_eq!(tidy(900.0), MAX_BPM);
    }

    #[test]
    fn half_time_does_not_win_over_the_beat() {
        // Every pulse that lines up at the true lag also lines up at twice it, so this
        // is the failure the octave rule exists to prevent.
        for bpm in [120.0, 160.0] {
            let flux = pulse_train(bpm, 20.0, 0, 1);
            let estimate = estimate(&flux, FPS).unwrap();
            assert!(
                (estimate.bpm - bpm).abs() < 3.0,
                "{bpm} bpm came back as {:.1}",
                estimate.bpm
            );
        }
    }
}
