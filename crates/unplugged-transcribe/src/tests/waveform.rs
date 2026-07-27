//! Waveform buckets, including the degenerate requests a zoomed-out view makes.

use super::*;

#[test]
fn peaks_keep_the_transient() {
    // A single spike in an otherwise quiet buffer. Averaging would bury it; min/max
    // must not, because that spike is the attack a user is looking for.
    let mut samples = vec![0.01f32; 10_000];
    samples[5_000] = 1.0;
    samples[5_001] = -1.0;

    let buckets = peaks(&samples, 10_000.0, 0.0, 1.0, 100);
    assert_eq!(buckets.len(), 100);

    let (min, max) = buckets[50];
    assert!((max - 1.0).abs() < 1e-6, "the peak survives: {max}");
    assert!((min + 1.0).abs() < 1e-6, "and so does the trough: {min}");

    // Its neighbours stay quiet.
    assert!(buckets[49].1 < 0.02 && buckets[51].1 < 0.02);
}

#[test]
fn peaks_respect_the_requested_window() {
    let samples: Vec<f32> = (0..1000).map(|i| if i < 500 { 0.5 } else { -0.5 }).collect();

    let first = peaks(&samples, 1000.0, 0.0, 0.5, 10);
    assert!(first.iter().all(|&(min, max)| min == 0.5 && max == 0.5));

    let second = peaks(&samples, 1000.0, 0.5, 1.0, 10);
    assert!(second.iter().all(|&(min, max)| min == -0.5 && max == -0.5));
}

#[test]
fn peaks_survive_degenerate_requests() {
    let samples = vec![0.5f32; 100];
    assert!(peaks(&[], 1000.0, 0.0, 1.0, 10).is_empty());
    assert!(peaks(&samples, 1000.0, 0.0, 1.0, 0).is_empty());
    assert!(peaks(&samples, 0.0, 0.0, 1.0, 10).is_empty());
    assert!(peaks(&samples, 1000.0, 1.0, 0.0, 10).is_empty(), "reversed range");
    assert!(peaks(&samples, 1000.0, 5.0, 6.0, 10).is_empty(), "past the end");

    // More buckets than samples: a zoomed-in view is entitled to ask, and every
    // bucket must still carry a value rather than the fold's sentinel.
    let dense = peaks(&samples, 1000.0, 0.0, 0.1, 500);
    assert_eq!(dense.len(), 500);
    assert!(dense.iter().all(|&(min, max)| min == 0.5 && max == 0.5));
}

