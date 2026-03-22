/// A linear parameter smoother that ramps from a current value to a target
/// over a specified number of samples. Used to avoid discontinuities when
/// parameters (e.g., velocity) change abruptly at note boundaries.
///
/// On the final sample of a ramp, the value snaps exactly to the target
/// to eliminate floating-point drift from repeated addition.
#[derive(Default)]
pub struct LinearSmoother {
    current: f32,
    target: f32,
    /// Per-sample step: (target - current) / ramp_samples at the time set_target() is called.
    increment: f32,
    /// Samples remaining in the current ramp. 0 means at target.
    remaining_samples: u32,
}

impl LinearSmoother {
    /// Jump immediately to `value` with no ramp. Use when starting from
    /// silence (envelope idle) where the attack envelope already provides
    /// a smooth fade-in.
    pub fn set_immediate(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.increment = 0.0;
        self.remaining_samples = 0;
    }

    /// Begin a linear ramp from the current value to `target` over
    /// `ramp_samples` samples. If `ramp_samples` is 0, behaves like
    /// `set_immediate`.
    pub fn set_target(&mut self, target: f32, ramp_samples: u32) {
        if ramp_samples == 0 {
            self.set_immediate(target);
            return;
        }
        self.target = target;
        self.increment = (target - self.current) / ramp_samples as f32;
        self.remaining_samples = ramp_samples;
    }

    /// Advance by one sample and return the current value.
    pub fn next_sample(&mut self) -> f32 {
        if self.remaining_samples == 0 {
            return self.current;
        }
        self.remaining_samples -= 1;
        if self.remaining_samples == 0 {
            // Snap to target on the final sample to avoid float drift.
            self.current = self.target;
        } else {
            self.current += self.increment;
        }
        self.current
    }

    /// Reset to zero. Called by `Plugin::reset()`.
    pub fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
        self.increment = 0.0;
        self.remaining_samples = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_value_is_zero() {
        let mut s = LinearSmoother::default();
        assert_eq!(s.next_sample(), 0.0);
    }

    #[test]
    fn set_immediate_jumps_to_target() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.8);
        assert_eq!(s.next_sample(), 0.8);
        assert_eq!(s.next_sample(), 0.8);
    }

    #[test]
    fn set_target_ramps_linearly() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.0);
        s.set_target(1.0, 4);
        let samples: Vec<f32> = (0..4).map(|_| s.next_sample()).collect();
        assert_eq!(samples, vec![0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn ramp_reaches_target_exactly() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.0);
        // Use a count that would cause float drift with naive addition.
        s.set_target(1.0, 3);
        for _ in 0..3 {
            s.next_sample();
        }
        // Final value should be exactly 1.0, not 0.9999... or 1.0001...
        assert_eq!(s.next_sample(), 1.0);
    }

    #[test]
    fn ramp_stays_at_target() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.0);
        s.set_target(0.5, 2);
        s.next_sample(); // 0.25
        s.next_sample(); // 0.5
                         // Should stay at 0.5.
        for _ in 0..10 {
            assert_eq!(s.next_sample(), 0.5);
        }
    }

    #[test]
    fn retarget_mid_ramp() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.0);
        s.set_target(1.0, 4);
        s.next_sample(); // 0.25
        s.next_sample(); // 0.5

        // Retarget from 0.5 toward 0.0 over 4 samples.
        s.set_target(0.0, 4);
        let samples: Vec<f32> = (0..4).map(|_| s.next_sample()).collect();
        assert_eq!(samples, vec![0.375, 0.25, 0.125, 0.0]);
    }

    #[test]
    fn zero_ramp_samples_jumps_immediately() {
        let mut s = LinearSmoother::default();
        s.set_immediate(0.5);
        s.set_target(1.0, 0);
        assert_eq!(s.next_sample(), 1.0);
    }
}
