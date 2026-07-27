//! What a waveform view draws.
//!
//! Nothing to do with transcription — this is the evidence the user fine-tunes against,
//! and it lives here because it is the same pure-DSP-no-I/O shape as everything else in
//! the crate.

/// Min/max pairs over `buckets` equal spans of `samples`.
///
/// What a waveform view actually draws. A bucket's *extremes* rather than its mean or its
/// RMS: at any zoom where one pixel covers hundreds of samples, averaging turns a
/// percussive attack into a low bump and the eye loses exactly the feature it is looking
/// for. Min and max keep the transient.
///
/// The range is given in seconds so the caller can ask for what is on screen rather than
/// receiving the whole take at every zoom level.
pub fn peaks(
    samples: &[f32],
    sample_rate: f64,
    from_seconds: f64,
    to_seconds: f64,
    buckets: usize,
) -> Vec<(f32, f32)> {
    if samples.is_empty() || buckets == 0 || sample_rate <= 0.0 || to_seconds <= from_seconds {
        return Vec::new();
    }

    let start = ((from_seconds.max(0.0) * sample_rate) as usize).min(samples.len());
    let end = ((to_seconds.max(0.0) * sample_rate).ceil() as usize).min(samples.len());
    if end <= start {
        return Vec::new();
    }

    let span = end - start;
    (0..buckets)
        .map(|bucket| {
            let low = start + span * bucket / buckets;
            // At least one sample per bucket: asking for more buckets than there are
            // samples is a legitimate thing for a zoomed-in view to do, and it should
            // repeat samples rather than return empty columns.
            let high = (start + span * (bucket + 1) / buckets).max(low + 1).min(end);

            let slice = &samples[low..high];
            slice.iter().fold((f32::MAX, f32::MIN), |(min, max), &s| {
                (min.min(s), max.max(s))
            })
        })
        .collect()
}
