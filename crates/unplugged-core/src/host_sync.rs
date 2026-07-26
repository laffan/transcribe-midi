//! Following a host's transport instead of owning one.
//!
//! Standalone, Unplugged owns the clock: the audio thread advances a sample cursor and
//! everything else observes it. As a plugin it owns nothing — the host says where the
//! playhead is at the start of every render block, and the sequencer's job is to agree.
//!
//! The naive version, re-seating the sequencer from the host every block, does not work:
//! seeking flushes held notes, so a sustained note would be re-triggered at the audio
//! block rate. The opposite, trusting our own cursor once started, drifts and misses
//! every locate and loop jump.
//!
//! So: advance normally, and re-seat **only when the disagreement is larger than normal
//! playback could produce**. Ordinary advance keeps the two within a fraction of a block
//! of each other; a locate, a cycle jump, or the user dragging the playhead moves them
//! much further apart than that. One threshold separates the two cases cleanly.
//!
//! Pure decision logic, with no audio and no platform, so the part most likely to be
//! subtly wrong is the part that can be tested exhaustively on any machine.

use crate::model::Ticks;

/// What the host says at the top of a render block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostTransport {
    /// Musical position in beats — quarter notes, which is what every host means by
    /// "beat" in this context.
    pub beats: f64,
    pub tempo_bpm: f64,
    pub playing: bool,
}

impl HostTransport {
    pub fn tick(&self, ppq: u16) -> Ticks {
        if !self.beats.is_finite() || self.beats <= 0.0 {
            return 0;
        }
        (self.beats * ppq as f64).round().clamp(0.0, Ticks::MAX as f64) as Ticks
    }
}

/// What the render path should do before rendering this block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSync {
    /// Re-seat the sequencer here first. Flushes held notes, so it is only issued when
    /// the position genuinely jumped.
    pub seek_to: Option<Ticks>,
    /// Whether the sequencer should be running after this block's adjustments.
    pub playing: bool,
}

/// Decides when to re-seat. Holds only what it needs to spot a transition.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostFollower {
    was_playing: bool,
    /// False until the first block, so the very first render always locates.
    started: bool,
}

impl HostFollower {
    pub fn new() -> Self {
        HostFollower::default()
    }

    /// How far apart the two clocks may be before it counts as a jump.
    ///
    /// Two things bound this. It must exceed what one block of ordinary playback can
    /// produce, or every block would seek and every sustained note would stutter — hence
    /// the block-length term, doubled for margin. And it must stay small enough that a
    /// real locate is never mistaken for drift, which a 32nd note comfortably satisfies:
    /// nobody locates by less than that, and nothing drifts by more.
    fn tolerance_ticks(ppq: u16, frames: u32, samples_per_tick: f64) -> f64 {
        let block_ticks = if samples_per_tick > 0.0 {
            frames as f64 / samples_per_tick
        } else {
            0.0
        };
        (block_ticks * 2.0).max(ppq as f64 / 8.0)
    }

    /// Decide what this block needs.
    ///
    /// `current_tick` is where the sequencer thinks it is; `samples_per_tick` and
    /// `frames` describe the block, and only feed the tolerance.
    pub fn follow(
        &mut self,
        host: HostTransport,
        current_tick: Ticks,
        ppq: u16,
        frames: u32,
        samples_per_tick: f64,
    ) -> HostSync {
        let target = host.tick(ppq);
        let tolerance = Self::tolerance_ticks(ppq, frames, samples_per_tick);
        let drift = (target as f64) - (current_tick as f64);

        // Always locate on the first block and whenever the host starts. A host that
        // starts from a new position — which is the usual case, since you locate and then
        // press play — would otherwise resume from wherever we happened to stop.
        let starting = !self.started || (host.playing && !self.was_playing);

        // While stopped, follow the host's playhead exactly. Logic lets you drag it while
        // stopped, and a plugin whose position quietly disagrees will play the wrong bar
        // the moment you hit play.
        let stopped_and_moved = !host.playing && target != current_tick;

        let seek_to = if starting || stopped_and_moved || drift.abs() > tolerance {
            Some(target)
        } else {
            None
        };

        self.started = true;
        self.was_playing = host.playing;

        HostSync { seek_to, playing: host.playing }
    }

    /// Forget the transport history — after the host changes sample rate, or the plugin
    /// is reallocated — so the next block locates rather than trusting a stale position.
    pub fn reset(&mut self) {
        *self = HostFollower::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPQ: u16 = 480;
    /// 48 kHz, 120 bpm: one tick is 50 samples.
    const SAMPLES_PER_TICK: f64 = 50.0;
    const FRAMES: u32 = 512;

    fn host(beats: f64, playing: bool) -> HostTransport {
        HostTransport { beats, tempo_bpm: 120.0, playing }
    }

    #[test]
    fn beats_convert_to_ticks() {
        assert_eq!(host(0.0, true).tick(PPQ), 0);
        assert_eq!(host(1.0, true).tick(PPQ), 480);
        assert_eq!(host(4.0, true).tick(PPQ), 1920);
        // Hosts have been known to report a small negative position just before zero.
        assert_eq!(host(-1.0, true).tick(PPQ), 0);
        assert_eq!(host(f64::NAN, true).tick(PPQ), 0);
    }

    #[test]
    fn the_first_block_always_locates() {
        let mut follower = HostFollower::new();
        // Even though the sequencer already claims to be at the host's position: it has
        // not rendered yet, and agreeing by accident is not the same as being seated.
        let sync = follower.follow(host(8.0, true), 3840, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(sync.seek_to, Some(3840));
        assert!(sync.playing);
    }

    #[test]
    fn steady_playback_does_not_reseek() {
        let mut follower = HostFollower::new();
        follower.follow(host(0.0, true), 0, PPQ, FRAMES, SAMPLES_PER_TICK);

        // Walk forward a block at a time with both clocks advancing together. Not one of
        // these may seek — a seek flushes held notes, so a stray one is an audible
        // re-trigger in the middle of a sustained chord.
        let ticks_per_block = FRAMES as f64 / SAMPLES_PER_TICK;
        for block in 1..200 {
            let tick = (block as f64 * ticks_per_block).round() as Ticks;
            let beats = tick as f64 / PPQ as f64;
            let sync = follower.follow(host(beats, true), tick, PPQ, FRAMES, SAMPLES_PER_TICK);
            assert_eq!(sync.seek_to, None, "block {block} re-seeked");
        }
    }

    #[test]
    fn small_jitter_is_tolerated() {
        let mut follower = HostFollower::new();
        follower.follow(host(0.0, true), 0, PPQ, FRAMES, SAMPLES_PER_TICK);

        // A block's worth of disagreement is normal — the host reports the position at
        // the block boundary and we are somewhere inside it.
        let sync = follower.follow(host(1.0, true), 480 - 10, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(sync.seek_to, None);
    }

    #[test]
    fn a_locate_is_followed() {
        let mut follower = HostFollower::new();
        follower.follow(host(0.0, true), 0, PPQ, FRAMES, SAMPLES_PER_TICK);

        // The user drags the playhead to bar 9 mid-playback.
        let sync = follower.follow(host(32.0, true), 240, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(sync.seek_to, Some(15_360));
    }

    #[test]
    fn a_cycle_jump_backwards_is_followed() {
        let mut follower = HostFollower::new();
        follower.follow(host(8.0, true), 3840, PPQ, FRAMES, SAMPLES_PER_TICK);

        // Logic's cycle wraps from bar 3 back to bar 1 between two blocks.
        let sync = follower.follow(host(0.0, true), 3850, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(sync.seek_to, Some(0));
    }

    #[test]
    fn pressing_play_after_locating_starts_from_the_new_position() {
        let mut follower = HostFollower::new();
        follower.follow(host(0.0, true), 0, PPQ, FRAMES, SAMPLES_PER_TICK);

        // Stop, drag the playhead, press play. The stopped blocks follow the playhead,
        // and the start locates — without that, playback would resume from bar 1.
        let stopped = follower.follow(host(16.0, false), 0, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(stopped.seek_to, Some(7680));
        assert!(!stopped.playing);

        let started = follower.follow(host(16.0, true), 7680, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(started.seek_to, Some(7680), "starting always locates");
        assert!(started.playing);
    }

    #[test]
    fn a_stopped_host_that_has_not_moved_does_not_seek() {
        let mut follower = HostFollower::new();
        follower.follow(host(4.0, false), 1920, PPQ, FRAMES, SAMPLES_PER_TICK);

        // Idle blocks while the host sits stopped must be free — a host calls the render
        // block continuously whether or not it is playing.
        for _ in 0..50 {
            let sync = follower.follow(host(4.0, false), 1920, PPQ, FRAMES, SAMPLES_PER_TICK);
            assert_eq!(sync.seek_to, None);
            assert!(!sync.playing);
        }
    }

    #[test]
    fn the_tolerance_grows_with_the_block_but_never_shrinks_past_a_32nd() {
        // A large block at a slow tempo: the block term dominates.
        let large = HostFollower::tolerance_ticks(PPQ, 4096, SAMPLES_PER_TICK);
        assert!(large > 4096.0 / SAMPLES_PER_TICK, "must exceed one block");

        // A tiny block: the musical floor takes over, so the threshold never collapses to
        // something ordinary jitter would cross.
        let small = HostFollower::tolerance_ticks(PPQ, 32, SAMPLES_PER_TICK);
        assert_eq!(small, PPQ as f64 / 8.0);

        // Degenerate input must not produce a zero or NaN threshold.
        let broken = HostFollower::tolerance_ticks(PPQ, 512, 0.0);
        assert!(broken.is_finite() && broken > 0.0);
    }

    #[test]
    fn a_reset_makes_the_next_block_locate() {
        let mut follower = HostFollower::new();
        follower.follow(host(0.0, true), 0, PPQ, FRAMES, SAMPLES_PER_TICK);
        assert_eq!(
            follower.follow(host(0.2, true), 96, PPQ, FRAMES, SAMPLES_PER_TICK).seek_to,
            None
        );

        follower.reset();
        assert!(
            follower.follow(host(0.2, true), 96, PPQ, FRAMES, SAMPLES_PER_TICK).seek_to.is_some(),
            "after a reset the position is not to be trusted"
        );
    }
}
