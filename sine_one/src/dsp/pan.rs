/// Apply constant-power panning to a mono signal.
///
/// `pan` is in [-1.0, 1.0]: -1.0 = hard left, 0.0 = center, 1.0 = hard right.
/// Returns `(left, right)` gain-adjusted samples.
///
/// Uses the sine/cosine pan law:
///   theta = (pan + 1) * PI/4     — maps [-1, 1] to [0, PI/2]
///   left  = sample * cos(theta)
///   right = sample * sin(theta)
///
/// This preserves constant power: left² + right² = sample² for all pan values.
pub fn apply_constant_power_pan(sample: f32, pan: f32) -> (f32, f32) {
    const QUARTER_PI: f32 = std::f32::consts::FRAC_PI_4;
    // Map pan from [-1, 1] to angle [0, PI/2].
    let theta = (pan + 1.0) * QUARTER_PI;
    // cos(0) = 1 → full left at pan = -1; sin(PI/2) = 1 → full right at pan = 1.
    let left = sample * theta.cos();
    let right = sample * theta.sin();
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn center_pan_equal_channels() {
        let (left, right) = apply_constant_power_pan(1.0, 0.0);
        // At center, both channels should equal 1/sqrt(2) ≈ 0.7071.
        let expected = std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (left - expected).abs() < 1e-6,
            "left {left} should be ~{expected}"
        );
        assert!(
            (right - expected).abs() < 1e-6,
            "right {right} should be ~{expected}"
        );
    }

    #[test]
    fn hard_left_silences_right() {
        let (left, right) = apply_constant_power_pan(1.0, -1.0);
        assert!(
            (left - 1.0).abs() < 1e-6,
            "hard-left should produce left=1.0, got {left}"
        );
        assert!(
            right.abs() < 1e-6,
            "hard-left should silence right, got {right}"
        );
    }

    #[test]
    fn hard_right_silences_left() {
        let (left, right) = apply_constant_power_pan(1.0, 1.0);
        assert!(
            left.abs() < 1e-6,
            "hard-right should silence left, got {left}"
        );
        assert!(
            (right - 1.0).abs() < 1e-6,
            "hard-right should produce right=1.0, got {right}"
        );
    }

    #[test]
    fn constant_power_at_center() {
        let sample = 0.8;
        let (left, right) = apply_constant_power_pan(sample, 0.0);
        // Constant power: left² + right² should equal sample².
        let power = left * left + right * right;
        let expected = sample * sample;
        assert!(
            (power - expected).abs() < 1e-6,
            "power {power} should equal {expected}"
        );
    }

    #[test]
    fn pan_is_monotonic() {
        let sample = 1.0;
        let steps: Vec<f32> = (-10..=10).map(|i| i as f32 / 10.0).collect();

        for window in steps.windows(2) {
            let (left_a, right_a) = apply_constant_power_pan(sample, window[0]);
            let (left_b, right_b) = apply_constant_power_pan(sample, window[1]);
            // As pan increases, left decreases and right increases.
            assert!(
                left_b <= left_a + 1e-6,
                "left should decrease: pan {} -> {}, left {} -> {}",
                window[0],
                window[1],
                left_a,
                left_b
            );
            assert!(
                right_b >= right_a - 1e-6,
                "right should increase: pan {} -> {}, right {} -> {}",
                window[0],
                window[1],
                right_a,
                right_b
            );
        }
    }

    proptest! {
        #[test]
        fn constant_power_across_range(pan in -1.0f32..=1.0, sample in -1.0f32..=1.0) {
            let (left, right) = apply_constant_power_pan(sample, pan);
            // Both outputs must be finite.
            prop_assert!(left.is_finite(), "left is not finite for pan={pan}, sample={sample}");
            prop_assert!(right.is_finite(), "right is not finite for pan={pan}, sample={sample}");
            // Constant power: left² + right² ≈ sample².
            let power = left * left + right * right;
            let expected = sample * sample;
            prop_assert!(
                (power - expected).abs() < 1e-4,
                "power {power} != expected {expected} for pan={pan}, sample={sample}"
            );
        }
    }
}
