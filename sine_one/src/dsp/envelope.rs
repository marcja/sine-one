/// The current phase of the AR envelope.
///
/// State machine:
///   note_on() → Attack → (level reaches 1.0, holds) → note_off() → Release → (level reaches 0) → Idle
///   note_on() from any state → enters Attack from current level (no reset to 0.0)
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvState {
    /// Silent — outputs 0.0 continuously.
    #[default]
    Idle,
    /// Level is ramping up toward 1.0 (or holding at 1.0 after attack completes).
    Attack,
    /// Level is ramping down toward 0.0 from wherever it was when note_off() was called.
    Release,
}

/// A linear Attack/Release envelope generator.
///
/// - `note_on()` enters Attack from the current level (preserves level on retrigger).
/// - During Attack, level increments by `attack_increment` (= (1.0 - level) / attack_samples)
///   each sample, clamping at 1.0 and holding there until `note_off()`. The increment is
///   scaled to the remaining distance so retrigger from a non-zero level still takes the
///   full attack duration.
/// - `note_off()` enters Release from any non-Idle state. The release decrement is computed
///   from the level at the moment note_off() is called, so a mid-attack release ramps from
///   the current level (not from 1.0).
/// - During Release, level decrements by `release_decrement` each sample. When level ≤ 0 → Idle.
#[derive(Default)]
pub struct ArEnvelope {
    state: EnvState,
    /// Current envelope amplitude in [0.0, 1.0].
    level: f32,
    /// Per-sample increment during Attack: (1.0 - level) / attack_samples.
    /// Recomputed on each note_on() so retrigger from a non-zero level
    /// still takes the full attack duration to reach 1.0.
    attack_increment: f32,
    /// Duration of the attack phase in samples. Stored so note_on() can
    /// recompute attack_increment from the current level.
    attack_samples: f32,
    /// Per-sample decrement during Release: level_at_release_start / release_samples.
    release_decrement: f32,
    /// Number of samples in the release phase (stored so we can compute decrement on note_off).
    release_samples: f32,
}

impl ArEnvelope {
    /// Configure the attack time. Call when sample rate changes or when the attack parameter
    /// is read at a note-on boundary.
    ///
    /// `attack_ms` is clamped to a minimum of 1.0 to avoid division by zero.
    pub fn set_attack(&mut self, attack_ms: f32, sample_rate: f32) {
        // Convert milliseconds to samples: ms * (samples/sec) / (1000 ms/sec).
        self.attack_samples = (attack_ms * sample_rate / 1000.0).max(1.0);
    }

    /// Configure the release time. Call when sample rate changes or when the release parameter
    /// is read at a note-off boundary.
    ///
    /// `release_ms` is clamped to a minimum of 1.0 to avoid division by zero.
    pub fn set_release(&mut self, release_ms: f32, sample_rate: f32) {
        // Store release duration in samples for computing the decrement at note_off().
        self.release_samples = (release_ms * sample_rate / 1000.0).max(1.0);
    }

    /// Trigger the attack phase. Preserves the current level so retrigger
    /// during release produces a smooth ramp rather than an audible dip to zero.
    pub fn note_on(&mut self) {
        // Compute increment from the remaining distance to 1.0 so that
        // retrigger from a non-zero level still takes the full attack
        // duration, producing a gentler slope instead of a truncated ramp.
        // When level is already at 1.0, remaining is 0.0 and the increment
        // becomes 0.0 — next_sample() will clamp and hold at 1.0.
        let remaining = (1.0 - self.level).max(0.0);
        self.attack_increment = remaining / self.attack_samples;
        self.state = EnvState::Attack;
    }

    /// Trigger the release phase from any non-Idle state.
    ///
    /// Computes the release decrement from the current level so the ramp always
    /// reaches zero in exactly `release_samples`, regardless of where in the
    /// attack/hold the note was released.
    pub fn note_off(&mut self) {
        if self.state == EnvState::Idle {
            return;
        }
        // Compute decrement so level reaches 0 in release_samples from current level.
        // If level is 0 (e.g., note_off immediately after note_on), go straight to Idle.
        if self.level <= 0.0 {
            self.state = EnvState::Idle;
            self.level = 0.0;
            return;
        }
        self.release_decrement = self.level / self.release_samples;
        self.state = EnvState::Release;
    }

    /// Advance the envelope by one sample and return the current level in [0.0, 1.0].
    pub fn next_sample(&mut self) -> f32 {
        match self.state {
            EnvState::Idle => 0.0,
            EnvState::Attack => {
                self.level += self.attack_increment;
                // Clamp at 1.0 and hold there until note_off().
                if self.level >= 1.0 {
                    self.level = 1.0;
                }
                self.level
            }
            EnvState::Release => {
                self.level -= self.release_decrement;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.state = EnvState::Idle;
                }
                self.level
            }
        }
    }

    /// Returns `true` when the envelope is in the Idle state (silent).
    /// Used by the plugin to distinguish "starting from silence" (phase reset
    /// desired) from "retrigger while sounding" (phase reset causes click).
    pub fn is_idle(&self) -> bool {
        self.state == EnvState::Idle
    }

    /// Returns `true` when the envelope is in the Release state (fading out).
    /// Used by the voice allocator to prefer stealing releasing voices over
    /// voices that are still in the attack/hold phase.
    pub fn is_releasing(&self) -> bool {
        self.state == EnvState::Release
    }

    /// Zero all state. Called by `Plugin::reset()`.
    pub fn reset(&mut self) {
        self.state = EnvState::Idle;
        self.level = 0.0;
        self.attack_increment = 0.0;
        self.release_decrement = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Default sample rate used by all test helpers.
    const TEST_SAMPLE_RATE: f32 = 44100.0;

    /// Convert a duration in milliseconds to a whole number of samples.
    fn ms_to_samples(ms: f32) -> usize {
        (ms * TEST_SAMPLE_RATE / 1000.0) as usize
    }

    /// Helper: create an envelope with given attack/release in ms at the test sample rate.
    fn make_envelope(attack_ms: f32, release_ms: f32) -> ArEnvelope {
        let mut env = ArEnvelope::default();
        env.set_attack(attack_ms, TEST_SAMPLE_RATE);
        env.set_release(release_ms, TEST_SAMPLE_RATE);
        env
    }

    #[test]
    fn idle_outputs_zero() {
        let mut env = ArEnvelope::default();
        for _ in 0..100 {
            assert_eq!(env.next_sample(), 0.0);
        }
    }

    #[test]
    fn attack_ramps_up() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        let mut prev = 0.0_f32;
        // Attack at 10ms / 44100 Hz ≈ 441 samples. Check first 400 are monotonically increasing.
        for i in 0..400 {
            let sample = env.next_sample();
            assert!(
                sample >= prev,
                "sample {i}: {sample} should be >= previous {prev}"
            );
            prev = sample;
        }
        // Should have increased above zero.
        assert!(prev > 0.0, "attack should produce nonzero output");
    }

    #[test]
    fn attack_reaches_one() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // 10ms at 44100 Hz = 441 samples. Run a few extra to be safe.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        let sample = env.next_sample();
        assert!(
            (sample - 1.0).abs() < 1e-6,
            "level should be 1.0 after attack completes, got {sample}"
        );
    }

    #[test]
    fn hold_stays_at_one() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // Complete the attack phase.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        // Hold — level should stay at 1.0.
        for i in 0..1000 {
            let sample = env.next_sample();
            assert!(
                (sample - 1.0).abs() < 1e-6,
                "sample {i}: hold level should be 1.0, got {sample}"
            );
        }
    }

    #[test]
    fn release_ramps_down() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        env.note_off();

        let mut prev = 1.0_f32;
        // Release at 100ms = 4410 samples. Check first 4000 are monotonically decreasing.
        for i in 0..4000 {
            let sample = env.next_sample();
            assert!(
                sample <= prev,
                "sample {i}: {sample} should be <= previous {prev}"
            );
            prev = sample;
        }
        assert!(prev < 1.0, "release should decrease from 1.0");
    }

    #[test]
    fn release_reaches_idle() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        env.note_off();

        // Release at 100ms = 4410 samples. Run extra to ensure idle.
        let release_samples = ms_to_samples(100.0);
        for _ in 0..release_samples + 10 {
            env.next_sample();
        }

        assert_eq!(env.state, EnvState::Idle, "should be Idle after release");
        assert_eq!(env.next_sample(), 0.0, "Idle should output 0.0");
    }

    #[test]
    fn retrigger_preserves_level() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // Advance partway through attack to get a nonzero level.
        for _ in 0..200 {
            env.next_sample();
        }
        let level_before = env.level;
        assert!(level_before > 0.0, "level should be nonzero mid-attack");

        // Retrigger — level should be preserved and state should be Attack.
        env.note_on();
        assert_eq!(
            env.state,
            EnvState::Attack,
            "state should be Attack after retrigger"
        );

        let sample = env.next_sample();
        assert!(
            sample >= level_before,
            "retrigger should preserve level (expected >= {level_before}, got {sample})"
        );
    }

    #[test]
    fn retrigger_during_release_preserves_level() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();

        // Complete attack phase.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        // Enter release and advance partway.
        env.note_off();
        for _ in 0..500 {
            env.next_sample();
        }

        let level_before = env.level;
        assert!(
            level_before > 0.0 && level_before < 1.0,
            "level should be mid-release, got {level_before}"
        );

        // Retrigger mid-release — level should be preserved and state should be Attack.
        env.note_on();
        assert_eq!(
            env.state,
            EnvState::Attack,
            "state should be Attack after retrigger"
        );

        let sample = env.next_sample();
        assert!(
            sample >= level_before,
            "retrigger mid-release should preserve level (expected >= {level_before}, got {sample})"
        );
    }

    proptest! {
        #[test]
        fn envelope_output_bounded(attack_ms in 1.0f32..5000.0, release_ms in 1.0f32..10000.0) {
            let mut env = ArEnvelope::default();
            env.set_attack(attack_ms, TEST_SAMPLE_RATE);
            env.set_release(release_ms, TEST_SAMPLE_RATE);
            env.note_on();
            for _ in 0..ms_to_samples(attack_ms) + 10 {
                let v = env.next_sample();
                prop_assert!(v.is_finite() && v >= 0.0 && v <= 1.0,
                    "attack phase: value {v} out of [0, 1]");
            }
            env.note_off();
            for _ in 0..ms_to_samples(release_ms) + 10 {
                let v = env.next_sample();
                prop_assert!(v.is_finite() && v >= 0.0 && v <= 1.0,
                    "release phase: value {v} out of [0, 1]");
            }
        }
    }

    #[test]
    fn is_idle_when_default() {
        let env = ArEnvelope::default();
        assert!(env.is_idle(), "fresh envelope should be idle");
    }

    #[test]
    fn is_not_idle_during_attack() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        assert!(!env.is_idle(), "envelope should not be idle during attack");
    }

    #[test]
    fn is_not_idle_during_release() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }
        env.note_off();
        // Advance partway through release.
        for _ in 0..500 {
            env.next_sample();
        }
        assert!(!env.is_idle(), "envelope should not be idle during release");
    }

    #[test]
    fn is_releasing_during_release() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }
        env.note_off();
        // Advance partway through release.
        for _ in 0..500 {
            env.next_sample();
        }
        assert!(
            env.is_releasing(),
            "envelope should be releasing during release phase"
        );
    }

    #[test]
    fn is_not_releasing_during_attack() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        assert!(
            !env.is_releasing(),
            "envelope should not be releasing during attack"
        );
    }

    #[test]
    fn is_not_releasing_when_idle() {
        let env = ArEnvelope::default();
        assert!(
            !env.is_releasing(),
            "fresh envelope should not be releasing"
        );
    }

    #[test]
    fn retrigger_attack_duration_matches_full_attack() {
        // When retriggering mid-release, the attack phase should still take
        // the full attack_ms to reach 1.0 (scaled increment), not a fraction
        // of it (fixed increment).
        let attack_ms = 10.0;
        let release_ms = 100.0;
        let attack_samples = ms_to_samples(attack_ms); // 441

        let mut env = make_envelope(attack_ms, release_ms);
        env.note_on();

        // Complete attack + hold briefly.
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        // Release partway — level drops to ~0.5.
        env.note_off();
        let half_release = ms_to_samples(release_ms) / 2;
        for _ in 0..half_release {
            env.next_sample();
        }
        let level_at_retrigger = env.level;
        assert!(
            level_at_retrigger > 0.3 && level_at_retrigger < 0.7,
            "expected mid-release level, got {level_at_retrigger}"
        );

        // Retrigger — attack should take full attack_samples to reach 1.0.
        env.set_attack(attack_ms, TEST_SAMPLE_RATE);
        env.note_on();

        // After 75% of the attack time, level should NOT yet be 1.0.
        // With the bug (fixed increment 1/441), starting from ~0.5, level
        // would reach 1.0 in ~50% of attack time and be clamped here.
        // With the fix (scaled increment 0.5/441), level would be ~0.875.
        let check_samples = attack_samples * 3 / 4;
        for _ in 0..check_samples {
            env.next_sample();
        }
        assert!(
            env.level < 1.0,
            "level should not reach 1.0 after 75% of attack time on retrigger, got {}",
            env.level
        );

        // After the full attack time (plus margin), level should be 1.0.
        let remaining_samples = attack_samples - check_samples + 10;
        for _ in 0..remaining_samples {
            env.next_sample();
        }
        assert!(
            (env.level - 1.0).abs() < 1e-6,
            "level should reach 1.0 after full attack time on retrigger, got {}",
            env.level
        );
    }

    #[test]
    fn is_idle_after_release_completes() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }
        env.note_off();
        // Complete release.
        let release_samples = ms_to_samples(100.0);
        for _ in 0..release_samples + 10 {
            env.next_sample();
        }
        assert!(
            env.is_idle(),
            "envelope should be idle after release completes"
        );
    }

    #[test]
    fn reset_clears_derived_increments() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        // Advance partway so attack_increment is non-zero.
        for _ in 0..200 {
            env.next_sample();
        }
        env.note_off();
        // release_decrement is now non-zero.

        env.reset();

        // After reset, derived increments should be zero so that a note_on()
        // without a preceding set_attack() produces no ramp (stays at 0.0).
        assert_eq!(
            env.attack_increment, 0.0,
            "attack_increment should be zero after reset"
        );
        assert_eq!(
            env.release_decrement, 0.0,
            "release_decrement should be zero after reset"
        );
    }
}
