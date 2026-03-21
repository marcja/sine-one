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
}
