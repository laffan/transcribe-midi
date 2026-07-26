//! Monophonic pitch detection by the YIN algorithm.
//!
//! YIN rather than plain autocorrelation because autocorrelation's peak is biased toward
//! long lags, which shows up as octave errors — a sung A3 detected as A2. YIN's
//! cumulative mean normalisation is precisely the fix for that, and the whole method is
//! about forty lines.
//!
//! Reference: de Cheveigné & Kawahara, "YIN, a fundamental frequency estimator for
//! speech and music", JASA 111(4), 2002.

/// Below this the frame is called unpitched. YIN's aperiodicity measure, not a
/// correlation — lower is more periodic.
const DEFAULT_THRESHOLD: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchEstimate {
    /// Hz. Zero when the frame is unpitched.
    pub frequency: f32,
    /// 0–1, higher is more confident.
    pub confidence: f32,
}

impl PitchEstimate {
    pub const UNPITCHED: PitchEstimate = PitchEstimate { frequency: 0.0, confidence: 0.0 };

    pub fn is_pitched(&self) -> bool {
        self.frequency > 0.0
    }
}

/// Estimate the fundamental of one frame.
///
/// The frame should be at least twice as long as the longest period of interest: lags up
/// to `frame.len() / 2` are searched, so a 2048-sample frame at 44.1 kHz reaches down to
/// about 43 Hz.
pub fn yin(frame: &[f32], sample_rate: f64, min_hz: f64, max_hz: f64) -> PitchEstimate {
    let half = frame.len() / 2;
    if half < 8 || sample_rate <= 0.0 {
        return PitchEstimate::UNPITCHED;
    }

    let min_tau = ((sample_rate / max_hz).floor() as usize).max(2);
    let max_tau = ((sample_rate / min_hz).ceil() as usize).min(half - 1);
    if min_tau >= max_tau {
        return PitchEstimate::UNPITCHED;
    }

    // Step 1: the squared-difference function.
    let mut difference = vec![0.0f32; max_tau + 1];
    for (tau, slot) in difference.iter_mut().enumerate().skip(1) {
        let mut sum = 0.0f32;
        for j in 0..half {
            let delta = frame[j] - frame[j + tau];
            sum += delta * delta;
        }
        *slot = sum;
    }

    // Step 2: cumulative mean normalisation. This is what removes the bias toward long
    // lags — without it, d[tau] falls off and the global minimum lands an octave low.
    let mut normalised = vec![1.0f32; max_tau + 1];
    let mut running = 0.0f32;
    for tau in 1..=max_tau {
        running += difference[tau];
        normalised[tau] = if running > 0.0 {
            difference[tau] * tau as f32 / running
        } else {
            1.0
        };
    }

    // Step 3: the first lag below threshold, walked to the bottom of its dip. Taking the
    // *first* rather than the global minimum is deliberate — the global minimum is often
    // an octave below, and the first qualifying dip is the fundamental.
    let mut best = None;
    let mut tau = min_tau;
    while tau <= max_tau {
        if normalised[tau] < DEFAULT_THRESHOLD {
            while tau < max_tau && normalised[tau + 1] < normalised[tau] {
                tau += 1;
            }
            best = Some(tau);
            break;
        }
        tau += 1;
    }

    let Some(tau) = best else {
        return PitchEstimate::UNPITCHED;
    };

    // Step 4: parabolic interpolation, so resolution is not limited to whole samples.
    // At 44.1 kHz a whole-sample lag near 1 kHz is nearly a quarter tone.
    let refined = if tau > min_tau && tau < max_tau {
        let (before, at, after) = (normalised[tau - 1], normalised[tau], normalised[tau + 1]);
        let denominator = 2.0 * (2.0 * at - before - after);
        if denominator.abs() > f32::EPSILON {
            tau as f32 + (after - before) / denominator
        } else {
            tau as f32
        }
    } else {
        tau as f32
    };

    if refined <= 0.0 {
        return PitchEstimate::UNPITCHED;
    }

    PitchEstimate {
        frequency: (sample_rate as f32) / refined,
        confidence: (1.0 - normalised[tau]).clamp(0.0, 1.0),
    }
}

/// Hz to a fractional MIDI note number. A440 is 69.
pub fn hz_to_midi(hz: f32) -> f32 {
    if hz <= 0.0 {
        return 0.0;
    }
    69.0 + 12.0 * (hz / 440.0).log2()
}

pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const SAMPLE_RATE: f64 = 44100.0;

    fn sine(hz: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| (2.0 * PI * hz * i as f32 / SAMPLE_RATE as f32).sin())
            .collect()
    }

    /// A tone with harmonics, which is what an instrument actually produces and what
    /// makes naive autocorrelation fail.
    fn harmonic(hz: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                (2.0 * PI * hz * t).sin()
                    + 0.5 * (2.0 * PI * hz * 2.0 * t).sin()
                    + 0.3 * (2.0 * PI * hz * 3.0 * t).sin()
                    + 0.15 * (2.0 * PI * hz * 4.0 * t).sin()
            })
            .collect()
    }

    fn detect(frame: &[f32]) -> PitchEstimate {
        yin(frame, SAMPLE_RATE, 55.0, 2000.0)
    }

    #[test]
    fn finds_a_pure_tone() {
        let estimate = detect(&sine(440.0, 2048));
        assert!(estimate.is_pitched());
        assert!((estimate.frequency - 440.0).abs() < 2.0, "{estimate:?}");
        assert!(estimate.confidence > 0.8);
    }

    #[test]
    fn finds_the_fundamental_of_a_harmonic_tone() {
        // The whole reason for YIN over autocorrelation: this must not come back as 220
        // or as 880.
        for hz in [110.0f32, 220.0, 440.0, 880.0] {
            let estimate = detect(&harmonic(hz, 4096));
            assert!(estimate.is_pitched(), "{hz} Hz was called unpitched");
            let cents = 1200.0 * (estimate.frequency / hz).log2();
            assert!(cents.abs() < 30.0, "{hz} Hz detected as {} Hz", estimate.frequency);
        }
    }

    #[test]
    fn is_accurate_across_the_piano_range() {
        // Every semitone from C2 to C6, which is where a hummed or played line lives.
        for midi in 36..=84 {
            let hz = midi_to_hz(midi as f32);
            let estimate = detect(&harmonic(hz, 4096));
            assert!(estimate.is_pitched(), "MIDI {midi} was called unpitched");

            let detected = hz_to_midi(estimate.frequency);
            assert!(
                (detected - midi as f32).abs() < 0.5,
                "MIDI {midi} detected as {detected:.2}"
            );
        }
    }

    #[test]
    fn silence_and_noise_are_unpitched() {
        assert!(!detect(&vec![0.0f32; 2048]).is_pitched());

        // Deterministic pseudo-noise. A real random source would make this test flaky.
        let mut state = 0x1234_5678u32;
        let noise: Vec<f32> = (0..4096)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / 8_388_608.0 - 1.0
            })
            .collect();
        let estimate = detect(&noise);
        assert!(
            !estimate.is_pitched() || estimate.confidence < 0.5,
            "noise reported as a confident pitch: {estimate:?}"
        );
    }

    #[test]
    fn a_frame_too_short_to_hold_a_period_is_unpitched() {
        assert!(!detect(&sine(440.0, 4)).is_pitched());
        // 64 samples cannot contain two periods of 55 Hz, and the search range collapses.
        assert!(!yin(&sine(60.0, 64), SAMPLE_RATE, 55.0, 60.0).is_pitched());
    }

    #[test]
    fn midi_conversion_round_trips() {
        assert!((hz_to_midi(440.0) - 69.0).abs() < 1e-4);
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((hz_to_midi(midi_to_hz(60.0)) - 60.0).abs() < 1e-3);
        assert_eq!(hz_to_midi(0.0), 0.0, "silence has no pitch to report");
    }
}
