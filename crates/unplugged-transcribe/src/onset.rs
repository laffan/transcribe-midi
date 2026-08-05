//! Onset detection by spectral flux.
//!
//! Flux — the frame-to-frame rise in magnitude, half-wave rectified — is what separates
//! two repetitions of the same note, which pitch tracking alone cannot do: a re-struck
//! C4 looks identical to a held one in the pitch track and completely different in the
//! spectrum. Everything downstream depends on getting those boundaries right, because a
//! missed onset becomes one long note where two were played.

use crate::dsp::{magnitude_spectrum, median, Complex};

/// Peak picking parameters.
///
/// The defaults are tuned for a hummed or single-instrument line, which is what v1
/// supports. They are fields rather than constants so a future noisier source has
/// somewhere to go.
#[derive(Debug, Clone, Copy)]
pub struct OnsetParams {
    /// Half-width, in frames, of the moving median used as the threshold.
    pub median_window: usize,
    /// How far above the local median a peak must rise, as a fraction of the overall
    /// mean flux. Scale-invariant, so a quiet take needs no retuning.
    pub delta: f32,
    /// How many times the local median a peak must reach.
    ///
    /// The additive test alone is not enough. A steady tone still produces a little
    /// flux — the analysis window slides across a waveform that is not bin-aligned, so
    /// spectral leakage wobbles frame to frame — and when the mean flux is near zero,
    /// the additive margin is near zero too and every one of those wobbles is a local
    /// maximum standing above its neighbours. A ratio test rejects them, because for a
    /// sustained note the peak and its median are the same size.
    pub ratio: f32,
    /// Absolute floor on the normalised flux, as a proportion of the previous frame's
    /// magnitude. Below this the change is not audible as an attack.
    pub min_flux: f32,
    /// Minimum gap between onsets, in frames. Suppresses the second trigger on an
    /// attack transient.
    pub min_gap_frames: usize,
}

impl Default for OnsetParams {
    fn default() -> Self {
        OnsetParams {
            median_window: 6,
            delta: 0.25,
            ratio: 2.0,
            min_flux: 0.05,
            // At a 10 ms hop this is 50 ms — faster than a human plays repeated notes,
            // and slow enough to swallow the double-trigger on a plucked attack.
            min_gap_frames: 5,
        }
    }
}

/// The detection function plus the frames it peaks at.
#[derive(Debug, Clone, Default)]
pub struct OnsetTrack {
    /// Half-wave rectified spectral flux, one value per frame.
    pub flux: Vec<f32>,
    /// Frame indices where a note begins.
    pub onsets: Vec<usize>,
}

/// Compute the flux envelope for a pre-framed signal.
///
/// `frames` are equal-length, overlapping windows; `window` is applied to each.
///
/// The result is **normalised by the previous frame's total magnitude**, which turns it
/// from an absolute quantity into a proportional one: "the spectrum grew by 40%" rather
/// than "the spectrum grew by 12 units". That matters more than it sounds. A held tone
/// still produces raw flux — summed across five hundred bins, floating-point and leakage
/// differences between overlapping windows add up to a visible wobble — and since a held
/// tone produces *nothing but* that wobble, its peaks stand proud of their own local
/// median and get picked as onsets. Divided by the magnitude they came from, the same
/// wobble is a fraction of a percent and a real attack is a large fraction of one, so a
/// single scale-free threshold separates them at any recording level.
pub fn spectral_flux(frames: &[&[f32]], window: &[f32]) -> Vec<f32> {
    spectral_flux_reporting(frames, window, &mut |_| {})
}

/// [`spectral_flux`], saying how far through it is as it goes.
///
/// One FFT per frame is the slowest thing in onset detection and the second slowest in
/// the whole pipeline, so a progress bar that ignored it would sit still for most of the
/// time it was on screen. `progress` receives 0–1 and is called every few frames, not
/// every frame — the caller may be doing real work in it.
pub fn spectral_flux_reporting(
    frames: &[&[f32]],
    window: &[f32],
    progress: &mut dyn FnMut(f32),
) -> Vec<f32> {
    /// How often to report, in frames. Frequent enough to look continuous on a bar that
    /// is on screen for a second or two.
    const REPORT_EVERY: usize = 16;

    let mut scratch: Vec<Complex> = Vec::new();
    let mut previous: Vec<f32> = Vec::new();
    let mut raw: Vec<f32> = Vec::with_capacity(frames.len());
    let mut energy: Vec<f32> = Vec::with_capacity(frames.len());

    for (index, frame) in frames.iter().enumerate() {
        if index % REPORT_EVERY == 0 {
            progress(index as f32 / frames.len().max(1) as f32);
        }
        let spectrum = magnitude_spectrum(frame, window, &mut scratch);
        let total: f32 = spectrum.iter().sum();

        if previous.len() != spectrum.len() {
            // First frame — nothing to compare against. Zero rather than the frame's own
            // energy, or a recording that opens mid-note reports an onset at frame 0
            // that is really just the start of the buffer.
            raw.push(0.0);
        } else {
            // Rises only. A note ending is a fall, and counting it would put an onset at
            // every release.
            raw.push(
                spectrum
                    .iter()
                    .zip(&previous)
                    .map(|(now, before)| (now - before).max(0.0))
                    .sum(),
            );
        }

        energy.push(total);
        previous = spectrum;
    }

    // A floor derived from the recording's own loudness, so the first sound after
    // silence reads as a large rise rather than dividing by nothing.
    let mean_energy = if energy.is_empty() {
        0.0
    } else {
        energy.iter().sum::<f32>() / energy.len() as f32
    };
    let floor = (mean_energy * 0.05).max(f32::EPSILON);

    raw.iter()
        .enumerate()
        .map(|(index, &value)| {
            let reference = if index == 0 { 0.0 } else { energy[index - 1] };
            value / (reference + floor)
        })
        .collect()
}

/// Pick peaks from a flux envelope.
///
/// A moving median rather than a fixed threshold: a quiet passage and a loud one produce
/// flux an order of magnitude apart, and any single threshold is wrong for one of them.
pub fn pick_peaks(flux: &[f32], params: OnsetParams) -> Vec<usize> {
    if flux.len() < 3 {
        return Vec::new();
    }

    let mean = flux.iter().sum::<f32>() / flux.len() as f32;
    if mean <= f32::EPSILON {
        return Vec::new();
    }
    let margin = params.delta * mean;

    let mut onsets: Vec<usize> = Vec::new();
    for index in 1..flux.len() - 1 {
        let value = flux[index];

        if value < params.min_flux {
            continue;
        }

        // Must be a local maximum...
        if value <= flux[index - 1] || value < flux[index + 1] {
            continue;
        }

        let low = index.saturating_sub(params.median_window);
        let high = (index + params.median_window + 1).min(flux.len());
        // ...and stand clear of its neighbourhood, by both measures.
        let local = median(&flux[low..high]);
        if value < local + margin || value < local * params.ratio {
            continue;
        }

        if let Some(&last) = onsets.last() {
            if index - last < params.min_gap_frames {
                // Keep whichever is stronger, so a slow attack reports the peak of the
                // rise rather than its foot.
                if value > flux[last] {
                    onsets.pop();
                } else {
                    continue;
                }
            }
        }
        onsets.push(index);
    }

    onsets
}

/// Flux and onsets together.
pub fn detect(frames: &[&[f32]], window: &[f32], params: OnsetParams) -> OnsetTrack {
    detect_reporting(frames, window, params, &mut |_| {})
}

/// [`detect`], reporting progress through the flux stage. Peak-picking is a single pass
/// over one value per frame and finishes too fast to be worth reporting from.
pub fn detect_reporting(
    frames: &[&[f32]],
    window: &[f32],
    params: OnsetParams,
    progress: &mut dyn FnMut(f32),
) -> OnsetTrack {
    let flux = spectral_flux_reporting(frames, window, progress);
    let onsets = pick_peaks(&flux, params);
    OnsetTrack { flux, onsets }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::hann;
    use std::f32::consts::PI;

    const SAMPLE_RATE: f32 = 44100.0;
    const FRAME: usize = 1024;
    const HOP: usize = 441; // 10 ms

    /// Frame a signal the way the transcriber does.
    fn frames(signal: &[f32]) -> Vec<&[f32]> {
        let mut out = Vec::new();
        let mut start = 0;
        while start + FRAME <= signal.len() {
            out.push(&signal[start..start + FRAME]);
            start += HOP;
        }
        out
    }

    /// A sequence of notes, each with a sharp attack and a decay.
    fn played(notes: &[(f32, f32)], seconds_each: f32) -> Vec<f32> {
        let per_note = (SAMPLE_RATE * seconds_each) as usize;
        let mut signal = Vec::with_capacity(per_note * notes.len());

        for &(hz, amplitude) in notes {
            for i in 0..per_note {
                let t = i as f32 / SAMPLE_RATE;
                // Percussive envelope: instant attack, exponential decay. This is what
                // gives the spectrum something to change at.
                let envelope = (-6.0 * t / seconds_each).exp();
                signal.push(amplitude * envelope * (2.0 * PI * hz * t).sin());
            }
        }
        signal
    }

    /// Frame index to the moment of the attack.
    ///
    /// Flux first rises at the frame whose window has just reached the attack, so the
    /// attack sits within one hop of that window's *end*, not its start. Reporting the
    /// frame's start would be a whole frame length early — 23 ms here, which is audible.
    fn onset_seconds(signal: &[f32]) -> Vec<f32> {
        let window = hann(FRAME);
        let track = detect(&frames(signal), &window, OnsetParams::default());
        track
            .onsets
            .iter()
            .map(|&frame| (frame * HOP + FRAME - HOP / 2) as f32 / SAMPLE_RATE)
            .collect()
    }

    #[test]
    fn silence_has_no_onsets() {
        assert!(onset_seconds(&vec![0.0f32; 44100]).is_empty());
    }

    #[test]
    fn finds_one_onset_per_note() {
        let signal = played(&[(440.0, 0.8), (554.0, 0.8), (659.0, 0.8), (880.0, 0.8)], 0.5);
        let onsets = onset_seconds(&signal);

        // Three, not four. Flux is a *change*, so a note beginning at sample zero has
        // nothing to change from and cannot be seen here. That is not a gap: note
        // assembly opens its first note where the signal becomes voiced, and only asks
        // this module about the boundaries in the middle.
        assert_eq!(onsets.len(), 3, "found {onsets:?}");

        for (index, at) in onsets.iter().enumerate() {
            let expected = (index + 1) as f32 * 0.5;
            assert!(
                (at - expected).abs() < 0.02,
                "onset {index} at {at:.3}s, expected {expected:.3}s"
            );
        }
    }

    #[test]
    fn separates_repetitions_of_the_same_pitch() {
        // The case pitch tracking cannot see: four identical notes in a row. Three
        // boundaries, for the same reason as above.
        let signal = played(&[(440.0, 0.8); 4], 0.4);
        assert_eq!(onset_seconds(&signal).len(), 3);
    }

    #[test]
    fn a_single_sustained_tone_is_one_onset_at_most() {
        let seconds = 2.0;
        let signal: Vec<f32> = (0..(SAMPLE_RATE * seconds) as usize)
            .map(|i| 0.8 * (2.0 * PI * 440.0 * i as f32 / SAMPLE_RATE).sin())
            .collect();

        // A steady tone has no flux after the first frame, so nothing should retrigger
        // partway through.
        assert!(onset_seconds(&signal).len() <= 1);
    }

    #[test]
    fn the_minimum_gap_suppresses_a_double_trigger() {
        let flux = vec![0.0, 0.1, 5.0, 4.0, 0.1, 0.0, 0.1, 6.0, 0.1, 0.0];
        let close = pick_peaks(&flux, OnsetParams { min_gap_frames: 8, ..Default::default() });
        let apart = pick_peaks(&flux, OnsetParams { min_gap_frames: 2, ..Default::default() });

        assert_eq!(apart.len(), 2, "two clear peaks");
        assert_eq!(close.len(), 1, "the gap collapses them");
        assert_eq!(close[0], 7, "and the stronger one wins");
    }

    #[test]
    fn a_flat_envelope_has_no_peaks() {
        assert!(pick_peaks(&[1.0; 50], OnsetParams::default()).is_empty());
        assert!(pick_peaks(&[], OnsetParams::default()).is_empty());
        assert!(pick_peaks(&[1.0, 2.0], OnsetParams::default()).is_empty());
    }
}
