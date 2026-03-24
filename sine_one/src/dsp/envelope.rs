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

/// Threshold below which the envelope snaps to its target (0.0 or 1.0).
/// 1e-4 is approximately −80 dB — inaudible.
const IDLE_THRESHOLD: f32 = 1e-4;

/// Number of time constants that fit in the user-specified duration.
/// Derived from: exp(−NUM_TC) = IDLE_THRESHOLD, so NUM_TC = −ln(IDLE_THRESHOLD).
/// With IDLE_THRESHOLD = 1e-4: NUM_TC = ln(10000) ≈ 9.2103.
/// This means "attack_ms" = time to reach within −80 dB of peak,
/// and "release_ms" = time to decay to −80 dB.
const NUM_TC: f32 = 9.2103;

/// An exponential Attack/Release envelope generator.
///
/// Uses one-pole exponential curves for both phases, producing the convex attack
/// (fast rise, settling toward peak) and concave release (fast initial drop, long tail)
/// characteristic of vactrol-based envelopes.
///
/// - `note_on()` enters Attack from the current level (preserves level on retrigger).
/// - During Attack, the remaining distance to 1.0 decays exponentially:
///   `level = 1.0 − (1.0 − level) × attack_coeff`. When level reaches within
///   IDLE_THRESHOLD of 1.0, it snaps to 1.0 and holds until `note_off()`.
/// - `note_off()` enters Release from any non-Idle state.
/// - During Release, level decays exponentially: `level *= release_coeff`.
///   When level falls below IDLE_THRESHOLD, it snaps to 0.0 → Idle.
pub struct ArEnvelope {
    state: EnvState,
    /// Current envelope amplitude in [0.0, 1.0].
    level: f32,
    /// Per-sample coefficient for exponential approach to 1.0 during Attack.
    /// In (0, 1): closer to 1.0 = slower attack. Computed from attack_ms in set_attack().
    attack_coeff: f32,
    /// Per-sample coefficient for exponential decay toward 0.0 during Release.
    /// In (0, 1): closer to 1.0 = slower release. Computed from release_ms in set_release().
    release_coeff: f32,
}

impl Default for ArEnvelope {
    fn default() -> Self {
        Self {
            state: EnvState::default(),
            level: 0.0,
            // Coefficient of 1.0 = infinitely slow (no change per sample).
            // Safe default: note_on() without set_attack() holds at current level.
            attack_coeff: 1.0,
            release_coeff: 1.0,
        }
    }
}

impl ArEnvelope {
    /// Configure the attack time. Call when sample rate changes or when the attack parameter
    /// is read at a note-on boundary.
    ///
    /// Computes the exponential coefficient: `attack_coeff = exp(−NUM_TC / attack_samples)`.
    /// `attack_ms` is clamped to a minimum of 1.0 to avoid division by zero.
    pub fn set_attack(&mut self, attack_ms: f32, sample_rate: f32) {
        // Convert milliseconds to samples: ms × (samples/sec) / (1000 ms/sec).
        let attack_samples = (attack_ms * sample_rate / 1000.0).max(1.0);
        // exp(−NUM_TC / N): the remaining distance to 1.0 shrinks by this factor each sample.
        self.attack_coeff = (-NUM_TC / attack_samples).exp();
    }

    /// Configure the release time. Call when sample rate changes or when the release parameter
    /// is read at a note-off boundary.
    ///
    /// Computes the exponential coefficient: `release_coeff = exp(−NUM_TC / release_samples)`.
    /// `release_ms` is clamped to a minimum of 1.0 to avoid division by zero.
    pub fn set_release(&mut self, release_ms: f32, sample_rate: f32) {
        // Convert milliseconds to samples: ms × (samples/sec) / (1000 ms/sec).
        let release_samples = (release_ms * sample_rate / 1000.0).max(1.0);
        // exp(−NUM_TC / N): level multiplied by this factor each sample during release.
        self.release_coeff = (-NUM_TC / release_samples).exp();
    }

    /// Trigger the attack phase. Preserves the current level so retrigger
    /// during release produces a smooth ramp rather than an audible dip to zero.
    pub fn note_on(&mut self) {
        self.note_on_at_level(self.level);
    }

    /// Trigger the attack phase with an explicit initial level.
    ///
    /// Sets the envelope level to `initial_level` (clamped to [0.0, 1.0]) and
    /// enters Attack, approaching 1.0 exponentially. With exponential curves,
    /// the coefficient is level-independent — no per-trigger recomputation needed.
    /// Contrast with `note_on()`, which preserves the current level.
    pub fn note_on_at_level(&mut self, initial_level: f32) {
        self.level = initial_level.clamp(0.0, 1.0);
        self.state = EnvState::Attack;
    }

    /// Trigger the release phase from any non-Idle state.
    ///
    /// With exponential release, the coefficient is level-independent — no
    /// per-trigger recomputation needed. The level decays toward 0.0 at the
    /// same rate regardless of where in the attack/hold the note was released.
    pub fn note_off(&mut self) {
        if self.state == EnvState::Idle {
            return;
        }
        // If level is effectively zero, go straight to Idle.
        if self.level <= IDLE_THRESHOLD {
            self.state = EnvState::Idle;
            self.level = 0.0;
            return;
        }
        self.state = EnvState::Release;
    }

    /// Advance the envelope by one sample and return the current level in [0.0, 1.0].
    pub fn next_sample(&mut self) -> f32 {
        match self.state {
            EnvState::Idle => 0.0,
            EnvState::Attack => {
                // Exponential approach to 1.0: remaining distance shrinks by attack_coeff.
                self.level = 1.0 - (1.0 - self.level) * self.attack_coeff;
                // Snap to 1.0 when within threshold (exponential never reaches target exactly).
                if self.level >= 1.0 - IDLE_THRESHOLD {
                    self.level = 1.0;
                }
                self.level
            }
            EnvState::Release => {
                // Exponential decay toward 0.0: level shrinks by release_coeff each sample.
                self.level *= self.release_coeff;
                // Snap to 0.0 when below threshold to prevent denormals and reach Idle.
                if self.level <= IDLE_THRESHOLD {
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
        // Coefficient of 1.0 = infinitely slow (safe default, same as Default::default()).
        self.attack_coeff = 1.0;
        self.release_coeff = 1.0;
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
    fn num_tc_matches_idle_threshold() {
        let expected = -(IDLE_THRESHOLD.ln());
        assert!(
            (NUM_TC - expected).abs() < 1e-3,
            "NUM_TC ({NUM_TC}) != -ln(IDLE_THRESHOLD) ({expected})"
        );
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
    fn attack_is_convex() {
        // Exponential attack: increments should decrease over time
        // (fast initial rise, settling toward peak).
        let attack_ms = 50.0;
        let mut env = make_envelope(attack_ms, 100.0);
        env.note_on();

        let attack_samples = ms_to_samples(attack_ms);
        let quarter = attack_samples / 4;

        // Measure level at 25% and 50% of attack time.
        for _ in 0..quarter {
            env.next_sample();
        }
        let level_25 = env.level;

        for _ in 0..quarter {
            env.next_sample();
        }
        let level_50 = env.level;

        for _ in 0..quarter {
            env.next_sample();
        }
        let level_75 = env.level;

        // Early increment (0→25%) should be larger than late increment (50→75%).
        let early_increment = level_25; // from 0.0 to level_25
        let late_increment = level_75 - level_50;
        assert!(
            early_increment > late_increment,
            "attack should be convex: early increment ({early_increment}) > late increment ({late_increment})"
        );
    }

    #[test]
    fn release_is_concave() {
        // Exponential release: early drop should be larger than late drop
        // (fast initial decay, long tail).
        let release_ms = 100.0;
        let mut env = make_envelope(10.0, release_ms);
        env.note_on();

        // Complete attack.
        let attack_samples = ms_to_samples(10.0);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        env.note_off();

        let release_samples = ms_to_samples(release_ms);
        let quarter = release_samples / 4;

        // Measure level at 25% and 50% of release time.
        for _ in 0..quarter {
            env.next_sample();
        }
        let level_25 = env.level;

        for _ in 0..quarter {
            env.next_sample();
        }
        let level_50 = env.level;

        for _ in 0..quarter {
            env.next_sample();
        }
        let level_75 = env.level;

        // Early drop (100%→25%) should be larger than late drop (50%→75%).
        let early_drop = 1.0 - level_25;
        let late_drop = level_50 - level_75;
        assert!(
            early_drop > late_drop,
            "release should be concave: early drop ({early_drop}) > late drop ({late_drop})"
        );
    }

    /// Count samples until the envelope reaches 1.0 (within tolerance), or `max` if it never does.
    fn samples_to_reach_one(env: &mut ArEnvelope, max: usize) -> usize {
        for i in 0..max {
            if (env.next_sample() - 1.0).abs() < 1e-6 {
                return i;
            }
        }
        max
    }

    #[test]
    fn retrigger_from_higher_level_reaches_one_sooner() {
        // With exponential attack, retrigger from 0.5 should reach 1.0
        // in fewer samples than from 0.0 (less remaining distance).
        let attack_ms = 10.0;
        let max_samples = ms_to_samples(attack_ms) + 100;

        let mut env_from_zero = make_envelope(attack_ms, 100.0);
        env_from_zero.note_on();
        let samples_from_zero = samples_to_reach_one(&mut env_from_zero, max_samples);

        let mut env_from_half = make_envelope(attack_ms, 100.0);
        env_from_half.note_on_at_level(0.5);
        let samples_from_half = samples_to_reach_one(&mut env_from_half, max_samples);

        assert!(
            samples_from_half < samples_from_zero,
            "retrigger from 0.5 should reach 1.0 sooner ({samples_from_half}) than from 0.0 ({samples_from_zero})"
        );
    }

    #[test]
    fn retrigger_attack_completes_within_attack_time() {
        // After retrigger from mid-release, the attack should still complete
        // within the configured attack_ms (plus margin for threshold snap).
        let attack_ms = 10.0;
        let release_ms = 100.0;

        let mut env = make_envelope(attack_ms, release_ms);
        env.note_on();

        // Complete attack + hold.
        let attack_samples = ms_to_samples(attack_ms);
        for _ in 0..attack_samples + 10 {
            env.next_sample();
        }

        // Release partway.
        env.note_off();
        let half_release = ms_to_samples(release_ms) / 2;
        for _ in 0..half_release {
            env.next_sample();
        }
        let level_at_retrigger = env.level;
        assert!(
            level_at_retrigger > 0.0 && level_at_retrigger < 1.0,
            "expected mid-release level, got {level_at_retrigger}"
        );

        // Retrigger — should reach 1.0 within attack_samples (faster than
        // from 0.0 because less remaining distance with exponential).
        env.set_attack(attack_ms, TEST_SAMPLE_RATE);
        env.note_on();
        for _ in 0..attack_samples + 10 {
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

    // --- note_on_at_level tests ---

    #[test]
    fn note_on_at_level_sets_initial_level() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on_at_level(0.5);

        // First next_sample() applies exponential approach:
        // level = 1.0 - (1.0 - 0.5) * attack_coeff
        let attack_samples = 10.0 * TEST_SAMPLE_RATE / 1000.0;
        let attack_coeff = (-NUM_TC / attack_samples).exp();
        let expected = 1.0 - (1.0 - 0.5) * attack_coeff;
        let sample = env.next_sample();
        assert!(
            (sample - expected).abs() < 1e-6,
            "first sample should be ~{expected}, got {sample}"
        );
    }

    #[test]
    fn note_on_at_level_zero_matches_note_on() {
        let mut env_a = make_envelope(10.0, 100.0);
        let mut env_b = make_envelope(10.0, 100.0);

        env_a.note_on();
        env_b.note_on_at_level(0.0);

        // Both should produce identical output for the full attack phase.
        let attack_samples = ms_to_samples(10.0);
        for i in 0..attack_samples + 10 {
            let a = env_a.next_sample();
            let b = env_b.next_sample();
            assert!(
                (a - b).abs() < 1e-6,
                "sample {i}: note_on ({a}) and note_on_at_level(0.0) ({b}) should match"
            );
        }
    }

    #[test]
    fn note_on_at_level_one_holds_at_one() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on_at_level(1.0);

        // Already at target — increment should be 0, level holds at 1.0.
        for i in 0..100 {
            let sample = env.next_sample();
            assert!(
                (sample - 1.0).abs() < 1e-6,
                "sample {i}: should hold at 1.0, got {sample}"
            );
        }
    }

    #[test]
    fn note_on_at_level_clamps_input() {
        let mut env = make_envelope(10.0, 100.0);

        // Value > 1.0 should be clamped to 1.0.
        env.note_on_at_level(2.0);
        let sample = env.next_sample();
        assert!(
            (sample - 1.0).abs() < 1e-6,
            "level > 1.0 should be clamped, got {sample}"
        );

        // Reset and test value < 0.0 — should be clamped to 0.0.
        env.reset();
        env.set_attack(10.0, TEST_SAMPLE_RATE);
        env.note_on_at_level(-1.0);
        let sample = env.next_sample();
        // Should behave like note_on_at_level(0.0): first sample uses exponential
        // approach from 0.0: level = 1.0 - (1.0 - 0.0) * attack_coeff.
        let attack_samples = 10.0 * TEST_SAMPLE_RATE / 1000.0;
        let attack_coeff = (-NUM_TC / attack_samples).exp();
        let expected = 1.0 - attack_coeff;
        assert!(
            (sample - expected).abs() < 1e-6,
            "level < 0.0 should be clamped to 0.0, got {sample}"
        );
    }

    #[test]
    fn note_off_while_idle_stays_idle() {
        let mut env = make_envelope(10.0, 100.0);
        assert!(env.is_idle(), "fresh envelope should be idle");

        env.note_off();

        assert!(
            env.is_idle(),
            "note_off on idle envelope should remain idle"
        );
        assert_eq!(env.next_sample(), 0.0, "idle envelope should output 0.0");
    }

    #[test]
    fn note_off_at_zero_level_goes_idle() {
        let mut env = make_envelope(10.0, 100.0);
        // Enter attack but don't advance — level is still 0.0.
        env.note_on();
        assert_eq!(
            env.level, 0.0,
            "level should be 0.0 immediately after note_on"
        );

        // Immediately release before any next_sample() call.
        env.note_off();

        assert!(
            env.is_idle(),
            "note_off at zero level should go straight to Idle"
        );
        assert_eq!(env.next_sample(), 0.0, "idle envelope should output 0.0");
    }

    #[test]
    fn reset_clears_derived_coefficients() {
        let mut env = make_envelope(10.0, 100.0);
        env.note_on();
        // Advance partway so coefficients have been computed.
        for _ in 0..200 {
            env.next_sample();
        }
        env.note_off();

        env.reset();

        // After reset, coefficients should be 1.0 (infinitely slow = no change),
        // matching Default::default(). A note_on() without set_attack() produces
        // no ramp (level stays at its current value).
        assert_eq!(
            env.attack_coeff, 1.0,
            "attack_coeff should be 1.0 after reset"
        );
        assert_eq!(
            env.release_coeff, 1.0,
            "release_coeff should be 1.0 after reset"
        );
    }
}
