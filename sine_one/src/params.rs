use crate::dsp::svf::{MAX_CUTOFF_HZ, MIN_CUTOFF_HZ};
use nih_plug::prelude::*;

/// Plugin parameters — user/host-controllable values.
///
/// Nine FloatParams and one IntParam that the host (or automation) can control:
/// - `fine_tune`: pitch offset in cents (±100 = ±1 semitone)
/// - `attack`: AR envelope attack time in milliseconds
/// - `release`: AR envelope release time in milliseconds
/// - `start_phase`: oscillator phase on NoteOn (0–360°)
/// - `fold`: wavefolder amount (0–1, 0 = bypass)
/// - `lpg`: LPG envelope-to-cutoff depth (0–1, 0 = bypass)
/// - `lpg_cutoff`: LPG maximum cutoff frequency in Hz
/// - `lpg_resonance`: LPG SVF resonance (0–1)
/// - `voices`: polyphonic voice count (1–8)
/// - `output_gain`: output level in dB (-24 to +12)
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

    /// Oscillator start phase in degrees (0–360).
    /// Read at note-on boundaries to set the oscillator phase — no smoothing needed.
    /// 0° = sin(0) = 0.0 (cleanest sine start); 90° = sin(π/2) = 1.0 (peak).
    #[id = "start_phase"]
    pub start_phase: FloatParam,

    /// Wavefolder amount (0–1). 0 = bypass (pure sine), 1 = maximum folding.
    /// Smoothed at 20 ms to avoid zipper noise when automated.
    #[id = "fold"]
    pub fold: FloatParam,

    /// LPG envelope-to-cutoff depth (0–1). At 0 the SVF is bypassed entirely.
    /// At 1 the filter cutoff fully tracks the AR envelope level.
    /// Smoothed at 20 ms — read per-sample in `process()`.
    #[id = "lpg"]
    pub lpg: FloatParam,

    /// LPG maximum cutoff frequency (Hz). The SVF opens up to this frequency
    /// when the envelope is at 1.0. Default 20 kHz (fully open).
    /// Smoothed at 20 ms — read per-sample in `process()`.
    #[id = "lpg_cutoff"]
    pub lpg_cutoff: FloatParam,

    /// LPG SVF resonance (0–1). Controls the resonant peak at cutoff.
    /// 0 = Butterworth (flat passband), 1 = strong resonant peak.
    /// Smoothed at 20 ms — read per-sample in `process()`.
    #[id = "lpg_resonance"]
    pub lpg_resonance: FloatParam,

    /// Number of polyphonic voices (1–8).
    /// At 1, the plugin behaves identically to monophonic mode.
    /// Read at the start of each process block — no smoothing needed.
    #[id = "voices"]
    pub voices: IntParam,

    /// Output gain in dB (-24 to +12). Applied after voice gain compensation
    /// as a final scaling factor. Default 0 dB (unity gain).
    #[id = "output_gain"]
    pub output_gain: FloatParam,
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

            start_phase: Self::build_start_phase(0.0),

            fold: Self::build_fold(0.0),

            lpg: Self::build_lpg(0.0),
            lpg_cutoff: Self::build_lpg_cutoff(MAX_CUTOFF_HZ),
            lpg_resonance: Self::build_lpg_resonance(0.0),

            voices: Self::build_voices(1),

            output_gain: Self::build_output_gain(0.0),
        }
    }
}

impl SineOneParams {
    /// Build the voices IntParam with the given default value.
    /// Shared between `Default` and the test helper to avoid duplicating
    /// range definition.
    fn build_voices(default_count: i32) -> IntParam {
        IntParam::new("Voices", default_count, IntRange::Linear { min: 1, max: 8 })
    }

    /// Build the output_gain FloatParam with the given default value.
    /// Shared between `Default` and the test helper to avoid duplicating
    /// range, unit, and step_size definitions.
    fn build_output_gain(default_db: f32) -> FloatParam {
        FloatParam::new(
            "Output Gain",
            default_db,
            FloatRange::Linear {
                min: -24.0,
                max: 12.0,
            },
        )
        .with_unit(" dB")
        .with_step_size(0.1)
    }

    /// Build the fold FloatParam with the given default value.
    /// Shared between `Default` and the test helper to avoid duplicating
    /// range, unit, smoother, and step_size definitions.
    fn build_fold(default: f32) -> FloatParam {
        FloatParam::new("Fold", default, FloatRange::Linear { min: 0.0, max: 1.0 })
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" %")
            .with_step_size(0.01)
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
    }

    /// Build the lpg FloatParam with the given default value.
    fn build_lpg(default: f32) -> FloatParam {
        FloatParam::new("LPG", default, FloatRange::Linear { min: 0.0, max: 1.0 })
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" %")
            .with_step_size(0.01)
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
    }

    /// Build the lpg_cutoff FloatParam with the given default value.
    fn build_lpg_cutoff(default_hz: f32) -> FloatParam {
        FloatParam::new(
            "LPG Cutoff",
            default_hz,
            FloatRange::Skewed {
                min: MIN_CUTOFF_HZ,
                max: MAX_CUTOFF_HZ,
                // Negative factor spreads the low end of the frequency range,
                // where perceptual differences are largest (20–500 Hz).
                factor: FloatRange::skew_factor(-2.0),
            },
        )
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit(" Hz")
        .with_step_size(1.0)
    }

    /// Build the lpg_resonance FloatParam with the given default value.
    fn build_lpg_resonance(default: f32) -> FloatParam {
        FloatParam::new(
            "LPG Resonance",
            default,
            FloatRange::Linear { min: 0.0, max: 1.0 },
        )
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit(" %")
        .with_step_size(0.01)
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
    }

    /// Build the start_phase FloatParam with the given default value.
    /// Shared between `Default` and the test helper to avoid duplicating
    /// range, unit, and step_size definitions.
    fn build_start_phase(default_degrees: f32) -> FloatParam {
        FloatParam::new(
            "Start Phase",
            default_degrees,
            FloatRange::Linear {
                min: 0.0,
                max: 360.0,
            },
        )
        .with_unit(" °")
        .with_step_size(1.0)
    }
}

#[cfg(test)]
impl SineOneParams {
    /// Create params with a custom start_phase default for testing.
    /// Works around nih-plug's `ParamMut` being `pub(crate)` by constructing
    /// the `FloatParam` with the desired value baked in as the default.
    pub fn with_start_phase(degrees: f32) -> Self {
        let mut params = Self::default();
        params.start_phase = Self::build_start_phase(degrees);
        params
    }

    /// Create params with a custom voice count for testing.
    pub fn with_voices(voice_count: i32) -> Self {
        let mut params = Self::default();
        params.voices = Self::build_voices(voice_count);
        params
    }

    /// Create params with a custom output gain (dB) for testing.
    pub fn with_output_gain(db: f32) -> Self {
        let mut params = Self::default();
        params.output_gain = Self::build_output_gain(db);
        params
    }

    /// Create params with a custom LPG depth for testing.
    pub fn with_lpg(depth: f32) -> Self {
        let mut params = Self::default();
        params.lpg = Self::build_lpg(depth);
        params
    }

    /// Create params with a custom LPG cutoff (Hz) for testing.
    pub fn with_lpg_cutoff(hz: f32) -> Self {
        let mut params = Self::default();
        params.lpg_cutoff = Self::build_lpg_cutoff(hz);
        params
    }

    /// Create params with a custom LPG resonance for testing.
    pub fn with_lpg_resonance(res: f32) -> Self {
        let mut params = Self::default();
        params.lpg_resonance = Self::build_lpg_resonance(res);
        params
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

        // Start Phase: range 0..360 degrees, default 0.0
        let sp_val = params.start_phase.value();
        assert!(
            (0.0..=360.0).contains(&sp_val),
            "start_phase default {sp_val} out of range"
        );
        assert_eq!(sp_val, 0.0, "start_phase default should be 0.0 degrees");

        // Voices: range 1..8, default 1
        let voices_val = params.voices.value();
        assert!(
            (1..=8).contains(&voices_val),
            "voices default {voices_val} out of range"
        );
        assert_eq!(voices_val, 1, "voices default should be 1");

        // Fold: range 0..1, default 0.0
        let fold_val = params.fold.value();
        assert!(
            (0.0..=1.0).contains(&fold_val),
            "fold default {fold_val} out of range"
        );
        assert_eq!(fold_val, 0.0, "fold default should be 0.0");

        // Output Gain: range -24..+12 dB, default 0.0
        let og_val = params.output_gain.value();
        assert!(
            (-24.0..=12.0).contains(&og_val),
            "output_gain default {og_val} out of range"
        );
        assert_eq!(og_val, 0.0, "output_gain default should be 0.0 dB");

        // LPG: range 0..1, default 0.0
        let lpg_val = params.lpg.value();
        assert!(
            (0.0..=1.0).contains(&lpg_val),
            "lpg default {lpg_val} out of range"
        );
        assert_eq!(lpg_val, 0.0, "lpg default should be 0.0");

        // LPG Cutoff: range 20..20000 Hz, default 20000.0
        let lc_val = params.lpg_cutoff.value();
        assert!(
            (20.0..=20000.0).contains(&lc_val),
            "lpg_cutoff default {lc_val} out of range"
        );
        assert_eq!(lc_val, 20000.0, "lpg_cutoff default should be 20000.0 Hz");

        // LPG Resonance: range 0..1, default 0.0
        let lr_val = params.lpg_resonance.value();
        assert!(
            (0.0..=1.0).contains(&lr_val),
            "lpg_resonance default {lr_val} out of range"
        );
        assert_eq!(lr_val, 0.0, "lpg_resonance default should be 0.0");
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

        // Start Phase: Linear range, should round-trip exactly.
        let sp_norm = params.start_phase.preview_normalized(0.0);
        let sp_plain = params.start_phase.preview_plain(sp_norm);
        assert!(
            (sp_plain - 0.0).abs() < 0.01,
            "start_phase round-trip: expected 0.0, got {sp_plain}"
        );

        // Voices: Linear integer range 1..8, default 1.
        let v_norm = params.voices.preview_normalized(1);
        let v_plain = params.voices.preview_plain(v_norm);
        assert_eq!(v_plain, 1, "voices round-trip: expected 1, got {v_plain}");

        // Fold: Linear 0..1, default 0.0.
        let fold_norm = params.fold.preview_normalized(0.0);
        let fold_plain = params.fold.preview_plain(fold_norm);
        assert!(
            (fold_plain - 0.0).abs() < 0.01,
            "fold round-trip: expected 0.0, got {fold_plain}"
        );

        // Output Gain: Linear -24..+12, default 0.0.
        let og_norm = params.output_gain.preview_normalized(0.0);
        let og_plain = params.output_gain.preview_plain(og_norm);
        assert!(
            (og_plain - 0.0).abs() < 0.01,
            "output_gain round-trip: expected 0.0, got {og_plain}"
        );

        // LPG: Linear 0..1, default 0.0.
        let lpg_norm = params.lpg.preview_normalized(0.0);
        let lpg_plain = params.lpg.preview_plain(lpg_norm);
        assert!(
            (lpg_plain - 0.0).abs() < 0.01,
            "lpg round-trip: expected 0.0, got {lpg_plain}"
        );

        // LPG Cutoff: Skewed 20..20000, default 20000.0.
        let lc_norm = params.lpg_cutoff.preview_normalized(20000.0);
        let lc_plain = params.lpg_cutoff.preview_plain(lc_norm);
        assert!(
            (lc_plain - 20000.0).abs() < 1.0,
            "lpg_cutoff round-trip: expected 20000.0, got {lc_plain}"
        );

        // LPG Resonance: Linear 0..1, default 0.0.
        let lr_norm = params.lpg_resonance.preview_normalized(0.0);
        let lr_plain = params.lpg_resonance.preview_plain(lr_norm);
        assert!(
            (lr_plain - 0.0).abs() < 0.01,
            "lpg_resonance round-trip: expected 0.0, got {lr_plain}"
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

        // Start Phase: Linear 0..360, default 0.0 → normalized 0.0.
        let sp_norm = params.start_phase.default_normalized_value();
        assert!(
            sp_norm.abs() < 1e-6,
            "start_phase normalized default: expected 0.0, got {sp_norm}"
        );

        // Voices: Linear 1..8, default 1 → normalized 0.0.
        let v_norm = params.voices.default_normalized_value();
        assert!(
            v_norm.abs() < 1e-6,
            "voices normalized default: expected 0.0, got {v_norm}"
        );

        // Fold: Linear 0..1, default 0.0 → normalized 0.0.
        let fold_norm = params.fold.default_normalized_value();
        assert!(
            fold_norm.abs() < 1e-6,
            "fold normalized default: expected 0.0, got {fold_norm}"
        );

        // Output Gain: Linear -24..+12, default 0.0 → normalized 24/36 ≈ 0.667.
        let og_norm = params.output_gain.default_normalized_value();
        assert!(
            (og_norm - 24.0 / 36.0).abs() < 1e-4,
            "output_gain normalized default: expected ~0.667, got {og_norm}"
        );

        // LPG: Linear 0..1, default 0.0 → normalized 0.0.
        let lpg_norm = params.lpg.default_normalized_value();
        assert!(
            lpg_norm.abs() < 1e-6,
            "lpg normalized default: expected 0.0, got {lpg_norm}"
        );

        // LPG Cutoff: Skewed 20..20000, default 20000.0 → normalized ~1.0.
        let lc_norm = params.lpg_cutoff.default_normalized_value();
        assert!(
            (lc_norm - 1.0).abs() < 1e-4,
            "lpg_cutoff normalized default: expected ~1.0, got {lc_norm}"
        );

        // LPG Resonance: Linear 0..1, default 0.0 → normalized 0.0.
        let lr_norm = params.lpg_resonance.default_normalized_value();
        assert!(
            lr_norm.abs() < 1e-6,
            "lpg_resonance normalized default: expected 0.0, got {lr_norm}"
        );
    }
}
