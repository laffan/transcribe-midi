//! The small amount of signal processing everything else is built on.
//!
//! Hand-written rather than pulled from a crate: this is one radix-2 FFT and two window
//! functions, it has to compile for iOS, and a dependency here would be more surface
//! than substance. It is tested against a hand-computed DFT, which is the only way to
//! be sure of an FFT without another FFT to compare against.

use std::f32::consts::PI;

/// A complex sample. Two fields and three operations — a crate would be overkill.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    pub const fn new(re: f32, im: f32) -> Self {
        Complex { re, im }
    }

    pub fn magnitude(self) -> f32 {
        (self.re * self.re + self.im * self.im).sqrt()
    }
}

/// In-place iterative radix-2 FFT. `data.len()` must be a power of two.
///
/// Decimation-in-time with a bit-reversal permutation first, so no scratch buffer is
/// needed — this runs once per frame over a whole recording and allocation here would
/// show up as a stall on long takes.
pub fn fft(data: &mut [Complex]) {
    let n = data.len();
    if n <= 1 {
        return;
    }
    debug_assert!(n.is_power_of_two(), "fft needs a power-of-two length");

    // Bit-reversal permutation.
    let mut target = 0usize;
    for position in 1..n {
        let mut mask = n >> 1;
        while target & mask != 0 {
            target &= !mask;
            mask >>= 1;
        }
        target |= mask;
        if target > position {
            data.swap(position, target);
        }
    }

    // Butterflies, doubling the transform size each pass.
    let mut size = 2;
    while size <= n {
        let angle = -2.0 * PI / size as f32;
        let half = size / 2;

        for start in (0..n).step_by(size) {
            for offset in 0..half {
                let theta = angle * offset as f32;
                let (sin, cos) = theta.sin_cos();

                let upper = data[start + offset + half];
                let twiddle = Complex::new(
                    upper.re * cos - upper.im * sin,
                    upper.re * sin + upper.im * cos,
                );
                let lower = data[start + offset];

                data[start + offset] = Complex::new(lower.re + twiddle.re, lower.im + twiddle.im);
                data[start + offset + half] =
                    Complex::new(lower.re - twiddle.re, lower.im - twiddle.im);
            }
        }
        size <<= 1;
    }
}

/// Magnitude spectrum of a real frame, up to Nyquist.
///
/// The frame is copied into `scratch` so the caller can reuse one buffer for a whole
/// recording rather than allocating per frame.
pub fn magnitude_spectrum(frame: &[f32], window: &[f32], scratch: &mut Vec<Complex>) -> Vec<f32> {
    scratch.clear();
    scratch.extend(
        frame
            .iter()
            .zip(window)
            .map(|(sample, weight)| Complex::new(sample * weight, 0.0)),
    );
    fft(scratch);
    scratch[..frame.len() / 2].iter().map(|c| c.magnitude()).collect()
}

/// Periodic Hann window, the right variant for spectral analysis.
///
/// The symmetric variant (dividing by `n - 1`) is for filter design; using it here would
/// leave a small discontinuity between overlapping frames.
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / n as f32).cos())
        .collect()
}

pub fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f32 = frame.iter().map(|s| s * s).sum();
    (sum / frame.len() as f32).sqrt()
}

/// Median of a slice. Copies, because every caller has a short window and none of them
/// want their data reordered.
pub fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The direct O(n²) transform. Slow and obviously correct — the point of comparison.
    fn naive_dft(input: &[Complex]) -> Vec<Complex> {
        let n = input.len();
        (0..n)
            .map(|k| {
                let mut acc = Complex::default();
                for (j, sample) in input.iter().enumerate() {
                    let angle = -2.0 * PI * (k * j) as f32 / n as f32;
                    let (sin, cos) = angle.sin_cos();
                    acc.re += sample.re * cos - sample.im * sin;
                    acc.im += sample.re * sin + sample.im * cos;
                }
                acc
            })
            .collect()
    }

    #[test]
    fn fft_matches_the_direct_transform() {
        for size in [2usize, 4, 8, 64, 256] {
            // A deterministic but non-trivial signal — a constant or a pure sine would
            // pass with a broken twiddle factor.
            let input: Vec<Complex> = (0..size)
                .map(|i| {
                    let t = i as f32;
                    Complex::new((t * 0.37).sin() + 0.5 * (t * 1.9).cos(), (t * 0.11).sin())
                })
                .collect();

            let expected = naive_dft(&input);
            let mut actual = input.clone();
            fft(&mut actual);

            for (k, (a, e)) in actual.iter().zip(&expected).enumerate() {
                let tolerance = 1e-2 * (size as f32).sqrt();
                assert!(
                    (a.re - e.re).abs() < tolerance && (a.im - e.im).abs() < tolerance,
                    "size {size}, bin {k}: {a:?} vs {e:?}"
                );
            }
        }
    }

    #[test]
    fn a_sine_puts_its_energy_in_one_bin() {
        let size = 1024;
        let bin = 40;
        let signal: Vec<f32> = (0..size)
            .map(|i| (2.0 * PI * bin as f32 * i as f32 / size as f32).sin())
            .collect();

        // Rectangular window, so the tone lands exactly on a bin with no leakage.
        let window = vec![1.0f32; size];
        let mut scratch = Vec::new();
        let spectrum = magnitude_spectrum(&signal, &window, &mut scratch);

        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(peak, bin);
    }

    #[test]
    fn the_hann_window_is_periodic() {
        let window = hann(8);
        assert!(window[0].abs() < 1e-6, "starts at zero");
        // The periodic variant does not return to zero at the end — that sample belongs
        // to the next period. Getting this wrong is silent and costs a little resolution.
        assert!(window[7] > 0.0);
        assert!((window[4] - 1.0).abs() < 1e-6, "peaks at the centre");
    }

    #[test]
    fn rms_of_a_full_scale_sine_is_root_two_over_two() {
        let signal: Vec<f32> = (0..1000)
            .map(|i| (2.0 * PI * 10.0 * i as f32 / 1000.0).sin())
            .collect();
        assert!((rms(&signal) - 0.707).abs() < 0.01);
        assert_eq!(rms(&[]), 0.0);
    }

    #[test]
    fn median_handles_both_parities_and_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }
}
