/// Maximum drive factor for the sine wavefolder.
///
/// At `fold_amount = 1.0`, the drive reaches `4π ≈ 12.57`. A unit-amplitude
/// sine input folds back on itself roughly 4 times per half-cycle, producing
/// rich harmonic content without excessive harshness.
const MAX_DRIVE: f32 = 4.0 * std::f32::consts::PI;

/// Apply a sine wavefolder to a signal.
///
/// Passes the input through `sin(input * drive)`, where `drive` maps linearly
/// from `1.0` at `fold = 0+` to `MAX_DRIVE` (4π) at `fold = 1.0`. This creates
/// harmonics at integer multiples of the fundamental (Bessel-weighted odd
/// harmonics) — the classic Buchla-style wavefolder.
///
/// - `fold = 0.0`: bypass — returns `input` unchanged (identity).
/// - `fold = 1.0`: maximum folding — `drive = 4π`.
///
/// Output is always in `[-1, 1]` because `sin()` is bounded.
pub fn wavefold(input: f32, fold: f32) -> f32 {
    // Bypass returns input directly — sin(input * 1.0) is NOT identity for sine input.
    if fold <= 0.0 {
        return input;
    }
    // Clamp so drive never exceeds MAX_DRIVE (e.g. from smoother overshoot).
    let fold = fold.min(1.0);
    let drive = 1.0 + fold * (MAX_DRIVE - 1.0);
    (input * drive).sin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn zero_fold_is_identity() {
        // fold_amount = 0 must return the input unchanged.
        for &input in &[0.0, 0.5, -0.5, 1.0, -1.0, 0.123, -0.999] {
            let result = wavefold(input, 0.0);
            assert_eq!(
                result, input,
                "fold=0 should be identity: wavefold({input}, 0.0) = {result}"
            );
        }
    }

    #[test]
    fn output_bounded() {
        // Output must stay in [-1, 1] for all valid inputs and fold amounts.
        let inputs = [0.0, 0.1, 0.5, 0.9, 1.0, -0.1, -0.5, -0.9, -1.0];
        let folds = [0.0, 0.1, 0.25, 0.5, 0.75, 1.0];
        for &input in &inputs {
            for &fold in &folds {
                let result = wavefold(input, fold);
                assert!(
                    (-1.0..=1.0).contains(&result),
                    "out of bounds: wavefold({input}, {fold}) = {result}"
                );
            }
        }
    }

    #[test]
    fn max_fold_changes_signal() {
        // At full fold, the output must differ from the input.
        let input = 0.5;
        let result = wavefold(input, 1.0);
        assert_ne!(
            result, input,
            "full fold should change the signal: wavefold({input}, 1.0) = {result}"
        );
    }

    #[test]
    fn fold_is_odd_symmetric() {
        // sin(-x) = -sin(x), so wavefold(-x, a) should equal -wavefold(x, a).
        let inputs = [0.1, 0.5, 0.9, 1.0];
        let folds = [0.25, 0.5, 0.75, 1.0];
        for &input in &inputs {
            for &fold in &folds {
                let pos = wavefold(input, fold);
                let neg = wavefold(-input, fold);
                assert!(
                    (pos + neg).abs() < 1e-6,
                    "odd symmetry violated: wavefold({input}, {fold}) = {pos}, wavefold({}, {fold}) = {neg}",
                    -input
                );
            }
        }
    }

    #[test]
    fn negative_fold_treated_as_zero() {
        // Edge case: fold_amount < 0 should bypass, same as fold = 0.
        let input = 0.7;
        let result = wavefold(input, -0.5);
        assert_eq!(
            result, input,
            "negative fold should bypass: wavefold({input}, -0.5) = {result}"
        );
    }

    proptest! {
        #[test]
        fn wavefold_always_bounded(input in -1.0f32..=1.0, fold in 0.0f32..=1.0) {
            let result = wavefold(input, fold);
            prop_assert!(result.is_finite(), "not finite: wavefold({input}, {fold}) = {result}");
            prop_assert!(
                (-1.0..=1.0).contains(&result),
                "out of bounds: wavefold({input}, {fold}) = {result}"
            );
        }

        #[test]
        fn zero_fold_always_identity(input in -1.0f32..=1.0) {
            let result = wavefold(input, 0.0);
            prop_assert_eq!(result, input, "fold=0 should be identity");
        }
    }
}
