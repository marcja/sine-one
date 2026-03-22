use nih_plug::prelude::*;

/// Plugin parameters — user/host-controllable values.
///
/// Three FloatParams that the host (or automation) can control:
/// - `fine_tune`: pitch offset in cents (±100 = ±1 semitone)
/// - `attack`: AR envelope attack time in milliseconds
/// - `release`: AR envelope release time in milliseconds
///
/// DSP state (oscillator phase, envelope level) does NOT belong here —
/// it lives on the plugin struct. Params holds only what the user controls.
///
/// FIXME(bitwig_defaults): Bitwig's "Set to Default" resets CLAP params to
///   min_value (0.0 normalized) instead of the reported default_value. nih-plug
///   correctly reports default_value (verified by unit tests and clap-validator),
///   so this appears to be a Bitwig host behavior. All three params are affected.
#[derive(Params)]
pub struct SineOneParams {
    /// Pitch offset in cents. ±100 cents = ±1 semitone.
    /// Smoothed at 20 ms to avoid zipper noise when automated.
    #[id = "fine_tune"]
    pub fine_tune: FloatParam,

    /// Envelope attack time in milliseconds.
    /// Value is read at note-on boundaries, not per-sample — no smoothing needed.
    #[id = "attack"]
    pub attack: FloatParam,

    /// Envelope release time in milliseconds.
    /// Value is read at note-off boundaries, not per-sample — no smoothing needed.
    #[id = "release"]
    pub release: FloatParam,
}

impl Default for SineOneParams {
    fn default() -> Self {
        Self {
            fine_tune: FloatParam::new(
                "Fine Tune",
                0.0,
                // ±100 cents = ±1 semitone
                FloatRange::Linear {
                    min: -100.0,
                    max: 100.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" ct")
            .with_step_size(1.0),

            attack: FloatParam::new(
                "Attack",
                10.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 5000.0,
                    // Negative factor spreads the low end of the range,
                    // where perceptual differences are largest (1–50 ms).
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.1),

            release: FloatParam::new(
                "Release",
                300.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 10000.0,
                    // Same skew rationale as attack.
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" ms")
            .with_step_size(0.1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify each param's default value is within its declared min/max.
    /// Catches common copy-paste errors in FloatRange definitions.
    #[test]
    fn param_defaults_in_range() {
        let params = SineOneParams::default();

        // Fine Tune: range −100..+100 cents, default 0.0
        let fine_tune_val = params.fine_tune.value();
        assert!(
            (-100.0..=100.0).contains(&fine_tune_val),
            "fine_tune default {fine_tune_val} out of range"
        );
        assert_eq!(fine_tune_val, 0.0, "fine_tune default should be 0.0 cents");

        // Attack: range 1..5000 ms, default 10.0
        let attack_val = params.attack.value();
        assert!(
            (1.0..=5000.0).contains(&attack_val),
            "attack default {attack_val} out of range"
        );
        assert_eq!(attack_val, 10.0, "attack default should be 10.0 ms");

        // Release: range 1..10000 ms, default 300.0
        let release_val = params.release.value();
        assert!(
            (1.0..=10000.0).contains(&release_val),
            "release default {release_val} out of range"
        );
        assert_eq!(release_val, 300.0, "release default should be 300.0 ms");
    }

    /// Verify that default values survive a normalize→unnormalize round-trip.
    /// This catches any floating-point drift that could cause the host's
    /// "reset to default" to produce a different value than the initial state.
    #[test]
    fn param_defaults_survive_normalize_round_trip() {
        let params = SineOneParams::default();

        // Fine Tune: Linear range, should round-trip exactly.
        let ft_norm = params.fine_tune.preview_normalized(0.0);
        let ft_plain = params.fine_tune.preview_plain(ft_norm);
        assert!(
            (ft_plain - 0.0).abs() < 0.01,
            "fine_tune round-trip: expected 0.0, got {ft_plain}"
        );

        // Attack: Skewed range, verify round-trip within step_size tolerance.
        let atk_norm = params.attack.preview_normalized(10.0);
        let atk_plain = params.attack.preview_plain(atk_norm);
        assert!(
            (atk_plain - 10.0).abs() < 0.1,
            "attack round-trip: expected 10.0, got {atk_plain}"
        );

        // Release: Skewed range, verify round-trip within step_size tolerance.
        let rel_norm = params.release.preview_normalized(300.0);
        let rel_plain = params.release.preview_plain(rel_norm);
        assert!(
            (rel_plain - 300.0).abs() < 0.1,
            "release round-trip: expected 300.0, got {rel_plain}"
        );
    }

    /// Verify that the normalized default values reported to the CLAP host
    /// are correct. The host uses these for "Reset to Default".
    #[test]
    fn param_normalized_defaults_correct() {
        let params = SineOneParams::default();

        // Fine Tune: Linear -100..100, default 0.0 → normalized 0.5.
        let ft_norm = params.fine_tune.default_normalized_value();
        assert!(
            (ft_norm - 0.5).abs() < 1e-6,
            "fine_tune normalized default: expected 0.5, got {ft_norm}"
        );

        // Attack: Skewed 1..5000, default 10.0 → normalized ~0.206.
        let atk_norm = params.attack.default_normalized_value();
        assert!(
            atk_norm > 0.1 && atk_norm < 0.3,
            "attack normalized default: expected ~0.206, got {atk_norm}"
        );

        // Release: Skewed 1..10000, default 300.0 → normalized ~0.416.
        let rel_norm = params.release.default_normalized_value();
        assert!(
            rel_norm > 0.3 && rel_norm < 0.5,
            "release normalized default: expected ~0.416, got {rel_norm}"
        );
    }
}
