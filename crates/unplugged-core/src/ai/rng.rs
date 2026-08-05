//! Reproducible randomness for the tools that need jitter.

/// xorshift64*. Not cryptographic and not trying to be — it exists so `humanize` gives
/// the same answer twice, which matters far more here than statistical quality.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift, so it is never allowed.
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[low, high]`.
    pub(super) fn range(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        let span = (high - low + 1) as u64;
        low + (self.next_u64() % span) as i64
    }
}
