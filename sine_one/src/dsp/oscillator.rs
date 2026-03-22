use std::f32::consts::TAU;

/// A simple phase-accumulator sine oscillator.
///
/// On each sample the phase advances by `phase_increment` (= frequency / sample_rate).
/// Output is `sin(phase * 2π)`. Sine waves are inherently band-limited, so no
/// PolyBLEP or oversampling is needed.
#[derive(Default)]
pub struct SineOscillator {
    /// Current phase in [0, 1). One full cycle = 1.0.
    phase: f32,
    /// Phase added per sample: frequency / sample_rate.
    phase_increment: f32,
}

impl SineOscillator {
    /// Set the oscillator frequency. Call this whenever the note or sample rate changes.
    ///
    /// `phase_increment = frequency / sample_rate` — this ratio determines how far
    /// the phase pointer advances each sample.
    pub fn set_frequency(&mut self, frequency: f32, sample_rate: f32) {
        // For audible frequencies the increment is always < 1.0. Guard against
        // supersonic input that would break the single-subtraction phase wrap.
        debug_assert!(
            frequency < sample_rate,
            "frequency ({frequency}) must be less than sample_rate ({sample_rate})"
        );
        self.phase_increment = frequency / sample_rate;
    }

    /// Advance the phase by one sample and return the sine value.
    ///
    /// Phase wraps at 1.0 to stay in [0, 1). The output is `sin(phase * 2π)`,
    /// which is always in [-1.0, 1.0].
    pub fn next_sample(&mut self) -> f32 {
        // Convert normalized phase [0, 1) to radians [0, 2π) and compute sine.
        let sample = (self.phase * TAU).sin();
        self.phase += self.phase_increment;
        // Wrap phase to [0, 1) to prevent unbounded growth. A single subtraction
        // suffices because phase_increment < 1.0 (enforced by set_frequency).
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        sample
    }

    /// Zero the phase accumulator. Called by `Plugin::reset()` to return the
    /// oscillator to a known state.
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// Apply a fine-tune offset in cents to a base frequency.
///
/// Formula: `base_freq * 2^(cents / 1200)`.
/// 1200 cents = one octave (frequency doubles).
/// NOTE: uses `powf` which is expensive. Called per-sample in process() to
/// support real-time pitch modulation; acceptable cost for a monophonic synth.
pub fn apply_detune(base_freq: f32, cents: f32) -> f32 {
    // Pitch interval in cents: 1200 cents = 1 octave = frequency × 2.
    // So the multiplier is 2^(cents / 1200).
    base_freq * 2.0_f32.powf(cents / 1200.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nih_plug::util::midi_note_to_freq;
    use proptest::prelude::*;

    #[test]
    fn sine_output_in_range() {
        let mut osc = SineOscillator::default();
        // A4 at 44100 Hz — arbitrary choice for exercising the oscillator.
        osc.set_frequency(440.0, 44100.0);
        for _ in 0..1000 {
            let sample = osc.next_sample();
            assert!(
                (-1.0..=1.0).contains(&sample),
                "sample {sample} out of [-1, 1]"
            );
        }
    }

    #[test]
    fn sine_phase_is_periodic() {
        let mut osc = SineOscillator::default();
        // Use a frequency that divides the sample rate evenly so one period
        // is an exact integer number of samples (44100 / 441 = 100).
        let sample_rate = 44100.0;
        let frequency = 441.0;
        osc.set_frequency(frequency, sample_rate);

        let first_sample = osc.next_sample();

        // Advance exactly one period: 100 samples.
        let period_samples = (sample_rate / frequency) as usize;
        for _ in 1..period_samples {
            osc.next_sample();
        }
        let one_period_later = osc.next_sample();

        assert!(
            (first_sample - one_period_later).abs() < 1e-4,
            "expected periodicity: first={first_sample}, after one period={one_period_later}"
        );
    }

    #[test]
    fn reset_clears_phase() {
        let mut osc = SineOscillator::default();
        osc.set_frequency(440.0, 44100.0);

        // Advance past zero phase.
        for _ in 0..50 {
            osc.next_sample();
        }

        osc.reset();

        // A fresh oscillator at the same frequency should produce the same output.
        let mut fresh = SineOscillator::default();
        fresh.set_frequency(440.0, 44100.0);

        assert_eq!(osc.next_sample(), fresh.next_sample());
    }

    #[test]
    fn midi_note_to_freq_a4() {
        // MIDI note 69 = A4 = 440 Hz. Verify the nih-plug utility is called correctly.
        let freq = midi_note_to_freq(69);
        assert!((freq - 440.0).abs() < 1e-6, "expected 440.0, got {freq}");
    }

    #[test]
    fn fine_tune_zero_cents_no_change() {
        let base = 440.0;
        let result = apply_detune(base, 0.0);
        assert!(
            (result - base).abs() < 1e-6,
            "0 cents should leave frequency unchanged: got {result}"
        );
    }

    #[test]
    fn fine_tune_1200_cents_octave_up() {
        let base = 440.0;
        // 1200 cents = one octave = frequency doubles.
        let result = apply_detune(base, 1200.0);
        assert!(
            (result - 880.0).abs() < 1e-3,
            "1200 cents should double frequency: expected 880.0, got {result}"
        );
    }

    proptest! {
        #[test]
        fn sine_is_always_finite(freq in 20.0f32..20000.0, sr in 22050.0f32..192000.0) {
            let mut osc = SineOscillator::default();
            osc.set_frequency(freq, sr);
            for _ in 0..512 {
                prop_assert!(osc.next_sample().is_finite());
            }
        }
    }
}
