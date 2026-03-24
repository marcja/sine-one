use super::envelope::ArEnvelope;
use super::oscillator::{apply_detune, SineOscillator};
use super::pan::apply_constant_power_pan;
use super::smoother::LinearSmoother;
use super::svf::{compute_lpg_cutoff, resonance_to_q, SvfFilter};
use super::wavefold::wavefold;

fn center_pan_gains() -> (f32, f32) {
    apply_constant_power_pan(1.0, 0.0)
}

/// Maximum number of simultaneous voices the plugin supports.
pub const MAX_VOICES: usize = 8;

/// Duration of the velocity crossfade ramp on smooth retrigger (seconds).
/// Short enough to be imperceptible, long enough to avoid a gain click
/// when retriggering with a different velocity.
const VELOCITY_RAMP_SECS: f32 = 0.002;

/// Attack times shorter than this (in ms) allow the start-phase transient
/// through. At this threshold the gate closes fully and the transient is
/// suppressed, preserving the slow-attack swell the user intended.
const TRANSIENT_GATE_MS: f32 = 10.0;

/// Minimum attack time (ms) from the parameter range. At this value the
/// transient gate is fully open (gate factor = 1.0).
const MIN_ATTACK_MS: f32 = 1.0;

/// Duration of the pan gain crossfade ramp (seconds). Prevents clicks when
/// PolyPan events change pan while the voice is producing audio (common in
/// mono mode with per-note pan randomization).
pub(crate) const PAN_RAMP_SECS: f32 = 0.002;

/// Parameters for triggering a note on a voice. Bundles the values needed
/// by `Voice::note_on()` to avoid excessive function arguments.
#[derive(Clone, Copy)]
pub struct NoteOnParams {
    pub note: u8,
    pub velocity: f32,
    pub base_freq: f32,
    /// Start phase in [0.0, 1.0), converted from degrees by the caller.
    pub start_phase_normalized: f32,
    pub sample_rate: f32,
    pub attack_ms: f32,
    /// Monotonic voice age counter for voice-stealing priority.
    pub age: u64,
}

/// Per-voice DSP state: oscillator, envelope, velocity, and note tracking.
///
/// Each `Voice` is an independent monophonic signal path that produces a
/// stereo sample via `render_sample()`. The plugin struct holds an array of
/// voices and sums their outputs for polyphonic operation.
///
/// All fields are stack-allocated. No heap allocation occurs during audio
/// processing.
pub struct Voice {
    osc: SineOscillator,
    env: ArEnvelope,
    svf: SvfFilter,
    velocity_smoother: LinearSmoother,
    /// The MIDI note this voice is currently playing, or `None` if idle.
    note: Option<u8>,
    /// Cached base frequency (Hz) for the current note, before fine-tune.
    base_freq: f32,
    /// Per-voice pan position in [-1.0, 1.0]. Updated by PolyPan events.
    pan: f32,
    /// Smoothed left pan gain. Ramps over ~2ms to prevent clicks when pan
    /// changes while the voice is producing audio.
    pan_left_smoother: LinearSmoother,
    /// Smoothed right pan gain. Ramps over ~2ms to prevent clicks.
    pan_right_smoother: LinearSmoother,
    /// Monotonically increasing counter set on `note_on()`. Used by the voice
    /// allocator to identify the oldest voice for stealing. Zero when idle.
    age: u64,
}

impl Default for Voice {
    fn default() -> Self {
        let (center_left, center_right) = center_pan_gains();
        let mut pan_left_smoother = LinearSmoother::default();
        pan_left_smoother.set_immediate(center_left);
        let mut pan_right_smoother = LinearSmoother::default();
        pan_right_smoother.set_immediate(center_right);

        Self {
            osc: SineOscillator::default(),
            env: ArEnvelope::default(),
            svf: SvfFilter::default(),
            velocity_smoother: LinearSmoother::default(),
            note: None,
            base_freq: 0.0,
            pan: 0.0,
            pan_left_smoother,
            pan_right_smoother,
            age: 0,
        }
    }
}

impl Voice {
    /// Returns the MIDI note this voice is currently playing, or `None` if idle.
    pub fn note(&self) -> Option<u8> {
        self.note
    }

    /// Returns the voice age counter. Higher values are newer.
    pub fn age(&self) -> u64 {
        self.age
    }

    /// Returns the current per-voice pan position.
    pub fn pan(&self) -> f32 {
        self.pan
    }

    /// Set the per-voice pan position. Called when a PolyPan event targets
    /// the note this voice is playing. The new gains are smoothed over ~2ms
    /// to prevent clicks when pan changes while the voice is producing audio.
    pub fn set_pan(&mut self, pan: f32, sample_rate: f32) {
        self.pan = pan;
        let (left, right) = apply_constant_power_pan(1.0, pan);
        let ramp_samples = (PAN_RAMP_SECS * sample_rate) as u32;
        self.pan_left_smoother.set_target(left, ramp_samples);
        self.pan_right_smoother.set_target(right, ramp_samples);
    }

    /// Returns `true` when the voice is silent (envelope idle, no note active).
    pub fn is_idle(&self) -> bool {
        self.env.is_idle()
    }

    /// Returns `true` when the voice's envelope is in the Release phase.
    /// Used by the voice allocator to prefer stealing releasing voices.
    pub fn is_releasing(&self) -> bool {
        self.env.is_releasing()
    }

    /// Trigger a note on this voice.
    ///
    /// Handles the phase reset, velocity smoothing, and transient strategy:
    /// - **From silence with non-zero start phase and short attack** (< 10 ms):
    ///   reset phase, jump velocity, and set an initial envelope level proportional
    ///   to `|sin(start_phase)|` gated by attack time. This creates the intentional
    ///   transient (click) that the start_phase parameter controls.
    /// - **From silence at 0° or with long attack**: reset phase, jump velocity,
    ///   normal envelope ramp from zero.
    /// - **Retrigger** (envelope not idle): phase always continues from its current
    ///   position and velocity ramps over ~2ms. This prevents uncontrolled clicks
    ///   from waveform discontinuities while the envelope is at a non-zero level.
    pub fn note_on(&mut self, params: NoteOnParams) {
        let was_idle = self.env.is_idle();
        self.note = Some(params.note);
        self.base_freq = params.base_freq;
        self.age = params.age;

        // Retrigger always continues phase to avoid uncontrolled waveform discontinuity
        // clicks while the envelope is at a non-zero level. Start phase transient only
        // applies when starting from silence (was_idle), where the envelope initial level
        // is controlled by the attack-gated mechanism below.
        let smooth_retrigger = !was_idle;

        if smooth_retrigger {
            // 0° retrigger: phase continues, velocity ramps over ~2ms.
            let ramp_samples = (VELOCITY_RAMP_SECS * params.sample_rate) as u32;
            self.velocity_smoother
                .set_target(params.velocity, ramp_samples);
        } else {
            // From silence: reset phase to start_phase and jump velocity.
            self.osc.set_phase(params.start_phase_normalized);
            self.velocity_smoother.set_immediate(params.velocity);
        }

        self.env.set_attack(params.attack_ms, params.sample_rate);

        if was_idle && params.start_phase_normalized != 0.0 {
            // Starting from silence with non-zero start phase: compute transient
            // level gated by attack time. Short attacks allow the click through;
            // long attacks suppress it to preserve the intended swell.
            //
            // transient_amplitude = |sin(start_phase × 2π)| — the waveform value
            //   at the start phase position (0° = 0.0, 90° = 1.0).
            // gate = linear ramp from 1.0 (at MIN_ATTACK_MS) to 0.0 (at TRANSIENT_GATE_MS).
            let transient_amplitude = (params.start_phase_normalized * std::f32::consts::TAU)
                .sin()
                .abs();
            let gate = ((TRANSIENT_GATE_MS - params.attack_ms)
                / (TRANSIENT_GATE_MS - MIN_ATTACK_MS))
                .clamp(0.0, 1.0);
            let initial_level = transient_amplitude * gate;
            self.env.note_on_at_level(initial_level);
        } else {
            self.env.note_on();
        }
    }

    /// Release this voice's envelope. Called on NoteOff.
    pub fn note_off(&mut self, release_ms: f32, sample_rate: f32) {
        self.env.set_release(release_ms, sample_rate);
        self.env.note_off();
        self.note = None;
    }

    /// Generate one stereo sample from this voice.
    ///
    /// Signal chain: oscillator → wavefold → LPG filter → envelope × velocity → pan.
    /// The envelope level is sampled once and used for both the LPG cutoff
    /// calculation and the amplitude multiplication.
    ///
    /// Returns `(left, right)`. Returns `(0.0, 0.0)` when idle.
    pub fn render_sample(
        &mut self,
        fine_tune_cents: f32,
        fold: f32,
        lpg_depth: f32,
        lpg_cutoff_hz: f32,
        lpg_resonance: f32,
        sample_rate: f32,
    ) -> (f32, f32) {
        if self.env.is_idle() {
            return (0.0, 0.0);
        }

        // Apply fine-tune detune to the cached base frequency.
        let freq = apply_detune(self.base_freq, fine_tune_cents);
        self.osc.set_frequency(freq, sample_rate);

        // Generate audio: oscillator → wavefold.
        let osc_sample = self.osc.next_sample();
        let folded = wavefold(osc_sample, fold);

        // Sample envelope level once — used for both LPG cutoff and amplitude.
        let env_level = self.env.next_sample();

        // Apply LPG filter when depth > 0. Bypass avoids unnecessary tan() call.
        let filtered = if lpg_depth > 0.0 {
            let fc = compute_lpg_cutoff(env_level, lpg_depth, lpg_cutoff_hz);
            let q = resonance_to_q(lpg_resonance);
            self.svf.process(folded, fc, q, sample_rate)
        } else {
            folded
        };

        let velocity = self.velocity_smoother.next_sample();
        let mono_output = filtered * env_level * velocity;

        let pan_left = self.pan_left_smoother.next_sample();
        let pan_right = self.pan_right_smoother.next_sample();
        (mono_output * pan_left, mono_output * pan_right)
    }

    /// Zero all DSP state. Called by `Plugin::reset()`.
    pub fn reset(&mut self) {
        self.osc.reset();
        self.env.reset();
        self.svf.reset();
        self.velocity_smoother.reset();
        self.note = None;
        self.base_freq = 0.0;
        self.pan = 0.0;
        let (center_left, center_right) = center_pan_gains();
        self.pan_left_smoother.set_immediate(center_left);
        self.pan_right_smoother.set_immediate(center_right);
        self.age = 0;
    }
}

/// Find the best voice slot to assign for a new note.
///
/// Allocation priority:
/// 1. First idle voice (no stealing needed)
/// 2. Oldest voice in Release state (already fading out — least disruptive steal)
/// 3. Oldest voice in Attack/hold state (last resort)
///
/// "Oldest" means lowest `age` value (age is a monotonic counter incremented
/// on each `note_on()`).
///
/// `voice_count` limits the search to `voices[0..voice_count]`, allowing the
/// plugin to dynamically reduce polyphony without resizing the array.
///
/// # Preconditions
/// `voice_count` must be in `1..=voices.len()`. Validated by `debug_assert!`;
/// the caller in `process()` guarantees this via `.clamp(1, MAX_VOICES)`.
pub fn allocate_voice(voices: &[Voice], voice_count: usize) -> usize {
    debug_assert!(voice_count > 0 && voice_count <= voices.len());

    // 1. First idle voice — no stealing needed.
    for (i, voice) in voices[..voice_count].iter().enumerate() {
        if voice.is_idle() {
            return i;
        }
    }

    // 2. Oldest releasing voice (lowest age among Release-state voices).
    let oldest_releasing = voices[..voice_count]
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_releasing())
        .min_by_key(|(_, v)| v.age());

    if let Some((i, _)) = oldest_releasing {
        return i;
    }

    // 3. Oldest active voice (lowest age among all voices — last resort).
    voices[..voice_count]
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| v.age())
        .map(|(i, _)| i)
        .expect("voice_count > 0 guarantees at least one voice")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nih_plug::util;

    use super::super::svf::MAX_CUTOFF_HZ;

    const SR: f32 = 44100.0;
    const A4_FREQ: f32 = 440.0;
    const A4_NOTE: u8 = 69;

    /// LPG-off render args: (lpg_depth, lpg_cutoff_hz, lpg_resonance).
    /// Used in tests that don't exercise the LPG to make the bypass intent explicit.
    const LPG_OFF: (f32, f32, f32) = (0.0, MAX_CUTOFF_HZ, 0.0);

    /// Helper: trigger a note on a voice with standard test parameters (10ms attack).
    fn trigger_voice(voice: &mut Voice, note: u8, velocity: f32, start_phase_norm: f32) {
        trigger_voice_with_attack(voice, note, velocity, start_phase_norm, 10.0);
    }

    #[test]
    fn voice_starts_idle() {
        let voice = Voice::default();
        assert!(voice.is_idle(), "fresh voice should be idle");
        assert_eq!(voice.note(), None, "fresh voice should have no note");
    }

    #[test]
    fn voice_note_on_produces_output() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.8, 0.25); // 90° start phase

        assert!(!voice.is_idle(), "voice should not be idle after note_on");
        assert_eq!(voice.note(), Some(A4_NOTE));

        // Render several samples and check for nonzero output.
        let mut found_nonzero = false;
        for _ in 0..100 {
            let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            if l != 0.0 || r != 0.0 {
                found_nonzero = true;
                break;
            }
        }
        assert!(
            found_nonzero,
            "voice should produce nonzero output after note_on"
        );
    }

    #[test]
    fn voice_note_off_releases() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.8, 0.0);

        // Advance through attack.
        let attack_samples = (10.0 * SR / 1000.0) as usize;
        for _ in 0..attack_samples + 10 {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        voice.note_off(100.0, SR);
        assert_eq!(voice.note(), None, "note should be cleared after note_off");
        assert!(
            voice.is_releasing(),
            "voice should be releasing after note_off"
        );

        // Complete release.
        let release_samples = (100.0 * SR / 1000.0) as usize;
        for _ in 0..release_samples + 10 {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }
        assert!(
            voice.is_idle(),
            "voice should be idle after release completes"
        );
    }

    #[test]
    fn voice_reset_zeros_state() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.8, 0.0);
        for _ in 0..100 {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        voice.reset();
        assert!(voice.is_idle(), "voice should be idle after reset");
        assert_eq!(voice.note(), None, "note should be None after reset");
        assert_eq!(voice.pan(), 0.0, "pan should be 0.0 after reset");
        assert_eq!(voice.age(), 0, "age should be 0 after reset");

        // Should produce silence.
        let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        assert_eq!((l, r), (0.0, 0.0), "reset voice should output silence");
    }

    #[test]
    fn voice_render_returns_stereo() {
        let mut voice = Voice::default();
        // Use non-zero pan to verify stereo routing.
        voice.set_pan(1.0, SR); // hard right
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.25); // 90° start phase for immediate signal

        // Render through the pan ramp so gains have settled.
        let ramp_warmup = (PAN_RAMP_SECS * SR) as usize + 10;
        for _ in 0..ramp_warmup {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        // Now measure stereo balance after ramp has completed.
        let mut left_sum = 0.0_f32;
        let mut right_sum = 0.0_f32;
        for _ in 0..50 {
            let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            left_sum += l.abs();
            right_sum += r.abs();
        }

        // Hard right: left should be ~0, right should have energy.
        assert!(
            left_sum < 0.01,
            "hard-right pan should produce near-zero left, got {left_sum}"
        );
        assert!(
            right_sum > 0.1,
            "hard-right pan should produce nonzero right, got {right_sum}"
        );
    }

    #[test]
    fn voice_retrigger_continues_phase_at_zero_start() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.8, 0.0);

        // Advance to build up some state.
        for _ in 0..200 {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        // Retrigger at 0° start phase — should NOT reset phase.
        // The voice is not idle, so smooth retrigger applies.
        assert!(!voice.is_idle());
        let velocity_before = voice.velocity_smoother.next_sample();
        // Re-advance smoother state (consumed one sample above).
        voice.velocity_smoother.set_immediate(velocity_before);

        voice.note_on(NoteOnParams {
            note: A4_NOTE,
            velocity: 1.0,
            base_freq: A4_FREQ,
            start_phase_normalized: 0.0,
            sample_rate: SR,
            attack_ms: 10.0,
            age: 2,
        });

        // Velocity should be ramping (not immediate) — check that first sample
        // after retrigger is not exactly the new velocity.
        let vel_after = voice.velocity_smoother.next_sample();
        assert!(
            (vel_after - 1.0).abs() > 0.001,
            "velocity should ramp on 0° retrigger, got {vel_after}"
        );
    }

    #[test]
    fn voice_retrigger_is_smooth_at_nonzero_start() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.5, 0.0);

        // Advance to build up some state.
        for _ in 0..200 {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }
        assert!(!voice.is_idle());

        // Retrigger at 90° (0.25 normalized) — should still smooth retrigger
        // (phase continues, velocity ramps) to avoid uncontrolled clicks.
        voice.note_on(NoteOnParams {
            note: A4_NOTE,
            velocity: 1.0,
            base_freq: A4_FREQ,
            start_phase_normalized: 0.25,
            sample_rate: SR,
            attack_ms: 10.0,
            age: 2,
        });

        // Velocity should ramp (not jump) — first sample should not be at target.
        let vel = voice.velocity_smoother.next_sample();
        assert!(
            (vel - 1.0).abs() > 0.001,
            "velocity should ramp on retrigger even with non-zero start phase, got {vel}"
        );
    }

    // --- allocate_voice tests ---

    /// Helper: make a voice active with a given age by triggering note_on.
    fn make_active_voice(age: u64, note: u8) -> Voice {
        let mut v = Voice::default();
        v.note_on(NoteOnParams {
            note,
            velocity: 0.8,
            base_freq: util::midi_note_to_freq(note),
            start_phase_normalized: 0.0,
            sample_rate: SR,
            attack_ms: 10.0,
            age,
        });
        v
    }

    /// Helper: make a releasing voice with a given age.
    fn make_releasing_voice(age: u64, note: u8) -> Voice {
        let mut v = make_active_voice(age, note);
        // Advance through attack so release has a nonzero level.
        let attack_samples = (10.0 * SR / 1000.0) as usize;
        for _ in 0..attack_samples + 10 {
            v.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }
        v.note_off(100.0, SR);
        v
    }

    #[test]
    fn allocate_picks_idle_first() {
        let voices = [
            make_active_voice(1, 60),
            Voice::default(), // idle
            make_active_voice(2, 62),
            Voice::default(), // idle
        ];
        // Should pick the first idle voice (index 1).
        assert_eq!(allocate_voice(&voices, 4), 1);
    }

    #[test]
    fn allocate_steals_oldest_releasing() {
        let voices = [
            make_active_voice(3, 60),
            make_releasing_voice(1, 62), // releasing, age 1 (oldest)
            make_releasing_voice(2, 64), // releasing, age 2
            make_active_voice(4, 65),
        ];
        // No idle voices. Should steal oldest releasing (index 1, age 1).
        assert_eq!(allocate_voice(&voices, 4), 1);
    }

    #[test]
    fn allocate_steals_oldest_active() {
        let voices = [
            make_active_voice(3, 60),
            make_active_voice(1, 62), // oldest active (age 1)
            make_active_voice(2, 64),
            make_active_voice(4, 65),
        ];
        // No idle or releasing voices. Should steal oldest active (index 1, age 1).
        assert_eq!(allocate_voice(&voices, 4), 1);
    }

    #[test]
    fn allocate_all_idle_returns_zero() {
        let voices: [Voice; 4] = core::array::from_fn(|_| Voice::default());
        assert_eq!(allocate_voice(&voices, 4), 0);
    }

    #[test]
    fn allocate_respects_voice_count_limit() {
        let voices = [
            make_active_voice(1, 60),
            make_active_voice(2, 62),
            Voice::default(), // idle, but at index 2 (beyond voice_count=2)
            Voice::default(),
        ];
        // voice_count=2, so only slots 0..2 are searched.
        // Both are active, so steal oldest (index 0, age 1).
        assert_eq!(allocate_voice(&voices, 2), 0);
    }

    // --- start phase transient tests ---

    /// Helper: trigger a note with a specific attack time.
    fn trigger_voice_with_attack(
        voice: &mut Voice,
        note: u8,
        velocity: f32,
        start_phase_norm: f32,
        attack_ms: f32,
    ) {
        let base_freq = util::midi_note_to_freq(note);
        voice.note_on(NoteOnParams {
            note,
            velocity,
            base_freq,
            start_phase_normalized: start_phase_norm,
            sample_rate: SR,
            attack_ms,
            age: 1,
        });
    }

    #[test]
    fn voice_from_silence_at_90_degrees_short_attack_has_immediate_output() {
        let mut voice = Voice::default();
        // 90° = 0.25 normalized, 1ms attack (fully open gate).
        trigger_voice_with_attack(&mut voice, A4_NOTE, 1.0, 0.25, 1.0);

        // First render_sample should produce non-negligible output because
        // the envelope starts at initial_level = |sin(90°)| * 1.0 = 1.0.
        let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        let mono = l.abs().max(r.abs());
        assert!(
            mono > 0.5,
            "90° start phase with 1ms attack should produce immediate output, got {mono}"
        );
    }

    #[test]
    fn voice_from_silence_at_90_degrees_long_attack_starts_quiet() {
        let mut voice = Voice::default();
        // 90° = 0.25 normalized, 500ms attack (gate fully closed).
        trigger_voice_with_attack(&mut voice, A4_NOTE, 1.0, 0.25, 500.0);

        // First sample should be near zero — long attack suppresses the transient.
        let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        let mono = l.abs().max(r.abs());
        assert!(
            mono < 0.01,
            "90° start phase with 500ms attack should start quiet, got {mono}"
        );
    }

    #[test]
    fn voice_from_silence_at_zero_degrees_starts_quiet() {
        let mut voice = Voice::default();
        // 0° = 0.0 normalized, 1ms attack (gate open, but sin(0) = 0).
        trigger_voice_with_attack(&mut voice, A4_NOTE, 1.0, 0.0, 1.0);

        // First sample should be near zero — sin(0°) = 0, no transient regardless.
        let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        let mono = l.abs().max(r.abs());
        assert!(
            mono < 0.01,
            "0° start phase should start quiet regardless of attack, got {mono}"
        );
    }

    // --- pan smoothing tests ---

    #[test]
    fn pan_change_is_smoothed() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.25); // 90° for immediate signal

        // Set pan hard-left and render through the full ramp to settle.
        voice.set_pan(-1.0, SR);
        let ramp_samples = (PAN_RAMP_SECS * SR) as usize + 10;
        for _ in 0..ramp_samples {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        // Now switch to hard-right. On the very first sample after the change,
        // the left channel should NOT be zero — it should still be near the old
        // hard-left gain, proving the pan is smoothed rather than instant.
        voice.set_pan(1.0, SR);
        let (left, _right) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        assert!(
            left.abs() > 0.01,
            "first sample after pan change should not be zero (smoothing), got left={left}"
        );
    }

    #[test]
    fn pan_ramp_completes() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.25); // 90° for immediate signal

        // Set pan hard-right and render through the full ramp + margin.
        voice.set_pan(1.0, SR);
        let ramp_samples = (PAN_RAMP_SECS * SR) as usize + 20;
        for _ in 0..ramp_samples {
            voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        // After the ramp completes, left should be ~0 (hard-right pan).
        let mut left_sum = 0.0_f32;
        let mut right_sum = 0.0_f32;
        for _ in 0..50 {
            let (l, r) = voice.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            left_sum += l.abs();
            right_sum += r.abs();
        }
        assert!(
            left_sum < 0.01,
            "after ramp to hard-right, left should be ~0, got {left_sum}"
        );
        assert!(
            right_sum > 0.1,
            "after ramp to hard-right, right should have energy, got {right_sum}"
        );
    }

    // --- wavefold tests ---

    #[test]
    fn voice_render_fold_nonzero_changes_timbre() {
        // Two identical voices, one with fold=0, one with fold=0.5.
        // Their outputs should differ, proving fold is applied.
        let mut voice_dry = Voice::default();
        let mut voice_wet = Voice::default();
        trigger_voice(&mut voice_dry, A4_NOTE, 1.0, 0.0);
        trigger_voice(&mut voice_wet, A4_NOTE, 1.0, 0.0);

        // Advance both through attack.
        let attack_samples = (10.0 * SR / 1000.0) as usize + 10;
        for _ in 0..attack_samples {
            voice_dry.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            voice_wet.render_sample(0.0, 0.5, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
        }

        // Compare several samples in the hold phase.
        let mut any_differ = false;
        for _ in 0..100 {
            let (dl, _) = voice_dry.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            let (wl, _) = voice_wet.render_sample(0.0, 0.5, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            if (dl - wl).abs() > 1e-6 {
                any_differ = true;
                break;
            }
        }
        assert!(
            any_differ,
            "fold=0.5 should produce different output than fold=0"
        );
    }

    // --- LPG tests ---

    #[test]
    fn voice_render_lpg_zero_depth_matches_bypass() {
        // Two identical voices: one with explicit lpg=0, one with default args.
        // Output should be identical since lpg_depth=0 bypasses the SVF.
        let mut voice_a = Voice::default();
        let mut voice_b = Voice::default();
        trigger_voice(&mut voice_a, A4_NOTE, 1.0, 0.0);
        trigger_voice(&mut voice_b, A4_NOTE, 1.0, 0.0);

        let attack_samples = (10.0 * SR / 1000.0) as usize + 10;
        for _ in 0..attack_samples {
            voice_a.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            voice_b.render_sample(0.0, 0.0, 0.0, 500.0, 0.5, SR);
        }

        // In the hold phase, both should produce identical output because
        // lpg_depth=0 bypasses the filter regardless of cutoff/resonance.
        for _ in 0..100 {
            let (al, ar) = voice_a.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            let (bl, br) = voice_b.render_sample(0.0, 0.0, 0.0, 500.0, 0.5, SR);
            assert_eq!((al, ar), (bl, br), "lpg_depth=0 should bypass filter");
        }
    }

    #[test]
    fn voice_render_lpg_darkens_output() {
        // With lpg=1.0 and low cutoff, output should differ from lpg=0.
        let mut voice_dry = Voice::default();
        let mut voice_lpg = Voice::default();
        trigger_voice(&mut voice_dry, A4_NOTE, 1.0, 0.0);
        trigger_voice(&mut voice_lpg, A4_NOTE, 1.0, 0.0);

        // Advance through attack to hold phase.
        let attack_samples = (10.0 * SR / 1000.0) as usize + 10;
        for _ in 0..attack_samples {
            voice_dry.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            voice_lpg.render_sample(0.0, 0.0, 1.0, 500.0, 0.0, SR);
        }

        // Compare samples — they should differ since the LPG filter is active.
        let mut any_differ = false;
        for _ in 0..100 {
            let (dl, _) = voice_dry.render_sample(0.0, 0.0, LPG_OFF.0, LPG_OFF.1, LPG_OFF.2, SR);
            let (wl, _) = voice_lpg.render_sample(0.0, 0.0, 1.0, 500.0, 0.0, SR);
            if (dl - wl).abs() > 1e-6 {
                any_differ = true;
                break;
            }
        }
        assert!(
            any_differ,
            "lpg=1.0 with 500Hz cutoff should produce different output than lpg=0"
        );
    }

    #[test]
    fn voice_reset_clears_svf() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.0);

        // Process with LPG active to dirty SVF state.
        for _ in 0..100 {
            voice.render_sample(0.0, 0.0, 1.0, 1000.0, 0.5, SR);
        }

        voice.reset();

        // After reset, the SVF state should be zero. Verify by checking that
        // a fresh voice and a reset voice produce the same output.
        let mut fresh = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.0);
        trigger_voice(&mut fresh, A4_NOTE, 1.0, 0.0);

        for _ in 0..50 {
            let (rl, rr) = voice.render_sample(0.0, 0.0, 1.0, 1000.0, 0.0, SR);
            let (fl, fr) = fresh.render_sample(0.0, 0.0, 1.0, 1000.0, 0.0, SR);
            assert!(
                (rl - fl).abs() < 1e-6 && (rr - fr).abs() < 1e-6,
                "reset voice should match fresh voice: reset=({rl},{rr}) fresh=({fl},{fr})"
            );
        }
    }
}
