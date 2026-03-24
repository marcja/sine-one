use std::f32::consts::PI;

/// Lowest cutoff frequency the filter will accept (Hz).
/// Prevents near-DC operation that could cause denormalized floats.
pub const MIN_CUTOFF_HZ: f32 = 20.0;

/// `ln(MIN_CUTOFF_HZ)` — precomputed to avoid per-sample `ln()` calls
/// in `compute_lpg_cutoff()`. Value: ln(20) ≈ 2.9957.
const LOG_MIN_CUTOFF: f32 = 2.995_732_3;

/// Highest cutoff frequency the filter will accept (Hz).
/// Used as the default and maximum for the `lpg_cutoff` parameter in `params.rs`.
pub const MAX_CUTOFF_HZ: f32 = 20_000.0;

/// Butterworth Q — flat passband, no resonant peak.
/// Equal to 1/√2 ≈ 0.7071. This is the "no resonance" setting.
const Q_MIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Maximum Q — near self-oscillation. Produces a strong resonant peak
/// at the cutoff frequency.
const Q_MAX: f32 = 20.0;

/// Precomputed ratio Q_MAX / Q_MIN, used by `resonance_to_q()` to avoid
/// a per-sample division. The exponential mapping is Q_MIN × Q_RATIO^resonance.
const Q_RATIO: f32 = Q_MAX / Q_MIN;

/// Fraction of Nyquist used as the upper cutoff clamp.
/// Leaves a small margin below true Nyquist (0.5) to prevent `tan()`
/// from diverging as the argument approaches π/2.
const NYQUIST_LIMIT_RATIO: f32 = 0.498;

/// Cytomic/Simper state variable filter (SVF) configured for lowpass output.
///
/// The SVF is a topology-preserving transform (TPT) filter that remains
/// stable under fast per-sample coefficient modulation — essential for an
/// LPG where the cutoff tracks the envelope every sample.
///
/// Two internal integrator states (`ic1eq`, `ic2eq`) correspond to physical
/// capacitor voltages in the analog prototype, which is why the filter stays
/// well-behaved when coefficients change rapidly.
///
/// Reference: Andrew Simper, "Linear Trapezoidal Integrated SVF",
/// Cytomic Technical Paper (2013).
pub struct SvfFilter {
    /// First integrator state (bandpass path).
    ic1eq: f32,
    /// Second integrator state (lowpass path).
    ic2eq: f32,
}

impl Default for SvfFilter {
    fn default() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
        }
    }
}

impl SvfFilter {
    /// Process one sample through the lowpass output of the SVF.
    ///
    /// Computes coefficients from `cutoff_hz` and `q` each sample, then
    /// advances the two integrator states. This per-sample coefficient
    /// update is what makes the SVF safe for envelope-rate modulation.
    ///
    /// # Arguments
    /// - `input`: audio sample (typically in [-1.0, 1.0])
    /// - `cutoff_hz`: filter cutoff frequency in Hz
    /// - `q`: filter quality factor (Q_MIN=0.707 for Butterworth, up to Q_MAX=20)
    /// - `sample_rate`: audio sample rate in Hz
    pub fn process(&mut self, input: f32, cutoff_hz: f32, q: f32, sample_rate: f32) -> f32 {
        // Clamp cutoff below Nyquist to prevent tan() from diverging.
        let fc = cutoff_hz.clamp(MIN_CUTOFF_HZ, sample_rate * NYQUIST_LIMIT_RATIO);

        // Bilinear transform: maps analog cutoff to digital domain.
        // g = tan(π × fc / fs) is the standard Cytomic/Simper formulation.
        let g = (PI * fc / sample_rate).tan();

        // Damping factor: k = 1/Q. Lower Q = more damping = flatter response.
        let k = 1.0 / q;

        // Coefficient computation (Cytomic SVF closed-form solution).
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        // Process: solve the delay-free loop analytically.
        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3; // bandpass output
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3; // lowpass output

        // Update integrator states for next sample.
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        v2
    }

    /// Zero both integrator states. Called by `Voice::reset()`.
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

/// Compute the LPG filter cutoff frequency from the envelope level.
///
/// Interpolates in log-frequency space between `MIN_CUTOFF_HZ` and
/// `max_cutoff`, modulated by the envelope level and LPG depth:
///
/// ```text
/// log_fc = (1 − depth) × ln(max) + depth × (ln(min) + env × (ln(max) − ln(min)))
/// fc = exp(log_fc)
/// ```
///
/// - `lpg_depth = 0`: cutoff stays at `max_cutoff` (transparent)
/// - `lpg_depth = 1, env = 0`: cutoff = `MIN_CUTOFF_HZ` (nearly closed)
/// - `lpg_depth = 1, env = 1`: cutoff = `max_cutoff` (fully open)
pub fn compute_lpg_cutoff(env_level: f32, lpg_depth: f32, max_cutoff: f32) -> f32 {
    let log_max = max_cutoff.ln();

    // Fully modulated cutoff: sweeps from min to max as envelope opens.
    let log_modulated = LOG_MIN_CUTOFF + env_level * (log_max - LOG_MIN_CUTOFF);

    // Blend between unmodulated (max) and modulated based on depth.
    let log_fc = (1.0 - lpg_depth) * log_max + lpg_depth * log_modulated;

    log_fc.exp()
}

/// Map the user-facing resonance parameter (0–1) to SVF Q.
///
/// Uses exponential mapping for perceptually even spread:
/// `Q = Q_MIN × (Q_MAX / Q_MIN) ^ resonance`
///
/// - `resonance = 0` → Q_MIN (0.707, Butterworth — flat passband)
/// - `resonance = 1` → Q_MAX (20.0 — strong resonant peak)
pub fn resonance_to_q(resonance: f32) -> f32 {
    Q_MIN * Q_RATIO.powf(resonance)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    /// Generate a sine at `freq` Hz, run it through the SVF, and return
    /// `(input_energy, output_energy)` measured over `measure` samples
    /// after a `warmup`-sample settling period.
    fn measure_energy(
        svf: &mut SvfFilter,
        freq: f32,
        cutoff: f32,
        q: f32,
        warmup: usize,
        measure: usize,
    ) -> (f32, f32) {
        for i in 0..warmup {
            let input = (i as f32 * freq / SR * 2.0 * PI).sin();
            svf.process(input, cutoff, q, SR);
        }
        let mut input_energy = 0.0_f32;
        let mut output_energy = 0.0_f32;
        for i in 0..measure {
            let input = ((i + warmup) as f32 * freq / SR * 2.0 * PI).sin();
            let output = svf.process(input, cutoff, q, SR);
            input_energy += input * input;
            output_energy += output * output;
        }
        (input_energy, output_energy)
    }

    // --- SvfFilter tests ---

    #[test]
    fn default_state_is_zero() {
        let svf = SvfFilter::default();
        assert_eq!(svf.ic1eq, 0.0);
        assert_eq!(svf.ic2eq, 0.0);
    }

    #[test]
    fn passthrough_at_max_cutoff() {
        // A 100 Hz sine through a 20 kHz lowpass should pass nearly unchanged.
        let mut svf = SvfFilter::default();
        let (input_e, output_e) = measure_energy(&mut svf, 100.0, MAX_CUTOFF_HZ, Q_MIN, 512, 1024);
        let ratio = output_e / input_e;
        assert!(
            ratio > 0.99,
            "100 Hz through 20 kHz cutoff should pass through, energy ratio = {ratio}"
        );
    }

    #[test]
    fn attenuates_above_cutoff() {
        // A 2 kHz sine through a 200 Hz lowpass should be heavily attenuated.
        let mut svf = SvfFilter::default();
        let (input_e, output_e) = measure_energy(&mut svf, 2000.0, 200.0, Q_MIN, 512, 1024);
        let ratio = output_e / input_e;
        assert!(
            ratio < 0.01,
            "2 kHz through 200 Hz cutoff should be attenuated, energy ratio = {ratio}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut svf = SvfFilter::default();

        // Process some samples to dirty the state.
        for i in 0..100 {
            let input = (i as f32 * 0.1).sin();
            svf.process(input, 1000.0, Q_MIN, SR);
        }
        assert!(
            svf.ic1eq != 0.0 || svf.ic2eq != 0.0,
            "state should be non-zero after processing"
        );

        svf.reset();
        assert_eq!(svf.ic1eq, 0.0, "ic1eq should be zero after reset");
        assert_eq!(svf.ic2eq, 0.0, "ic2eq should be zero after reset");
    }

    #[test]
    fn resonance_boosts_near_cutoff() {
        // At high Q, a sine at the cutoff frequency should have more energy than at Q_MIN.
        let mut svf_flat = SvfFilter::default();
        let (_, energy_flat) = measure_energy(&mut svf_flat, 1000.0, 1000.0, Q_MIN, 512, 2048);

        let mut svf_res = SvfFilter::default();
        let (_, energy_res) = measure_energy(&mut svf_res, 1000.0, 1000.0, 15.0, 512, 2048);

        assert!(
            energy_res > energy_flat * 2.0,
            "high Q should boost energy at cutoff: flat={energy_flat}, resonant={energy_res}"
        );
    }

    // --- compute_lpg_cutoff tests ---

    #[test]
    fn depth_zero_returns_max_cutoff() {
        let max = 10000.0;
        for env in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let fc = compute_lpg_cutoff(env, 0.0, max);
            assert!(
                (fc - max).abs() < 0.01,
                "depth=0 should return max_cutoff for env={env}, got {fc}"
            );
        }
    }

    #[test]
    fn depth_one_env_zero_returns_min() {
        let fc = compute_lpg_cutoff(0.0, 1.0, 20000.0);
        assert!(
            (fc - MIN_CUTOFF_HZ).abs() < 0.01,
            "depth=1, env=0 should return MIN_CUTOFF_HZ, got {fc}"
        );
    }

    #[test]
    fn depth_one_env_one_returns_max() {
        let max = 20000.0;
        let fc = compute_lpg_cutoff(1.0, 1.0, max);
        assert!(
            (fc - max).abs() < 0.1,
            "depth=1, env=1 should return max_cutoff, got {fc}"
        );
    }

    #[test]
    fn midpoint_is_geometric_mean() {
        // At env=0.5, depth=1, cutoff should be the geometric mean of min and max.
        let max = 20000.0;
        let fc = compute_lpg_cutoff(0.5, 1.0, max);
        let geometric_mean = (MIN_CUTOFF_HZ * max).sqrt();
        assert!(
            (fc - geometric_mean).abs() < 1.0,
            "env=0.5 should give geometric mean {geometric_mean}, got {fc}"
        );
    }

    // --- resonance_to_q tests ---

    #[test]
    fn zero_resonance_is_butterworth() {
        let q = resonance_to_q(0.0);
        assert!(
            (q - Q_MIN).abs() < 1e-6,
            "resonance=0 should give Q_MIN, got {q}"
        );
    }

    #[test]
    fn one_resonance_is_max() {
        let q = resonance_to_q(1.0);
        assert!(
            (q - Q_MAX).abs() < 0.01,
            "resonance=1 should give Q_MAX, got {q}"
        );
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn output_is_finite(
            cutoff in 20.0_f32..20000.0,
            q in 0.707_f32..20.0,
            input in -1.0_f32..=1.0,
        ) {
            let mut svf = SvfFilter::default();
            for _ in 0..64 {
                let out = svf.process(input, cutoff, q, 44100.0);
                prop_assert!(out.is_finite(), "output must be finite, got {out}");
            }
        }

        #[test]
        fn cutoff_increases_with_envelope(
            env_low in 0.0_f32..0.49,
            env_high in 0.51_f32..1.0,
            depth in 0.01_f32..1.0,
            max_cutoff in 100.0_f32..20000.0,
        ) {
            let fc_low = compute_lpg_cutoff(env_low, depth, max_cutoff);
            let fc_high = compute_lpg_cutoff(env_high, depth, max_cutoff);
            prop_assert!(
                fc_high > fc_low,
                "higher env should give higher cutoff: env_low={env_low}→{fc_low}, env_high={env_high}→{fc_high}"
            );
        }

        #[test]
        fn resonance_is_monotonic(
            res_low in 0.0_f32..0.49,
            res_high in 0.51_f32..1.0,
        ) {
            let q_low = resonance_to_q(res_low);
            let q_high = resonance_to_q(res_high);
            prop_assert!(
                q_high > q_low,
                "higher resonance should give higher Q: {res_low}→{q_low}, {res_high}→{q_high}"
            );
        }
    }
}
