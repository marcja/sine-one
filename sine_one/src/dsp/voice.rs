use super::envelope::ArEnvelope;
use super::oscillator::{apply_detune, SineOscillator};
use super::pan::apply_constant_power_pan;
use super::smoother::LinearSmoother;

/// Pre-computed constant-power pan gains. Caches the sin/cos results from
/// `apply_constant_power_pan` so the trig is computed once per PolyPan event
/// instead of once per sample.
#[derive(Clone, Copy)]
struct PanGains {
    left: f32,
    right: f32,
}

impl Default for PanGains {
    fn default() -> Self {
        // Default pan is center (0.0). Compute gains once.
        let (left, right) = apply_constant_power_pan(1.0, 0.0);
        Self { left, right }
    }
}

/// Maximum number of simultaneous voices the plugin supports.
pub const MAX_VOICES: usize = 8;

/// Duration of the velocity crossfade ramp on smooth retrigger (seconds).
/// Short enough to be imperceptible, long enough to avoid a gain click
/// when retriggering with a different velocity.
const VELOCITY_RAMP_SECS: f32 = 0.002;

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
    velocity_smoother: LinearSmoother,
    /// The MIDI note this voice is currently playing, or `None` if idle.
    note: Option<u8>,
    /// Cached base frequency (Hz) for the current note, before fine-tune.
    base_freq: f32,
    /// Per-voice pan position in [-1.0, 1.0]. Updated by PolyPan events.
    pan: f32,
    /// Cached left/right gains for the current pan position. Avoids
    /// per-sample sin/cos calls since pan only changes on PolyPan events.
    pan_gains: PanGains,
    /// Monotonically increasing counter set on `note_on()`. Used by the voice
    /// allocator to identify the oldest voice for stealing. Zero when idle.
    age: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            osc: SineOscillator::default(),
            env: ArEnvelope::default(),
            velocity_smoother: LinearSmoother::default(),
            note: None,
            base_freq: 0.0,
            pan: 0.0,
            pan_gains: PanGains::default(),
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
    /// the note this voice is playing. Pre-computes left/right gains so
    /// `render_sample()` avoids per-sample trig.
    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan;
        let (left, right) = apply_constant_power_pan(1.0, pan);
        self.pan_gains = PanGains { left, right };
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
    /// Handles the phase reset and velocity smoothing strategy:
    /// - **From silence** (envelope idle): reset phase to `start_phase_normalized`,
    ///   jump velocity immediately. The attack envelope provides the fade-in.
    /// - **Retrigger at 0° start phase**: skip phase reset (continuing phase avoids
    ///   a waveform dip to zero → click). Velocity ramps over ~2ms.
    /// - **Retrigger at non-zero start phase**: reset phase to create an intentional
    ///   transient. Velocity jumps immediately.
    pub fn note_on(&mut self, params: NoteOnParams) {
        self.note = Some(params.note);
        self.base_freq = params.base_freq;
        self.age = params.age;

        // Determine retrigger strategy based on current envelope state and start phase.
        let smooth_retrigger = !self.env.is_idle() && params.start_phase_normalized == 0.0;

        if smooth_retrigger {
            // 0° retrigger: phase continues, velocity ramps over ~2ms.
            let ramp_samples = (VELOCITY_RAMP_SECS * params.sample_rate) as u32;
            self.velocity_smoother
                .set_target(params.velocity, ramp_samples);
        } else {
            // From silence or non-zero start phase: reset phase and jump velocity.
            self.osc.set_phase(params.start_phase_normalized);
            self.velocity_smoother.set_immediate(params.velocity);
        }

        self.env.set_attack(params.attack_ms, params.sample_rate);
        self.env.note_on();
    }

    /// Release this voice's envelope. Called on NoteOff.
    pub fn note_off(&mut self, release_ms: f32, sample_rate: f32) {
        self.env.set_release(release_ms, sample_rate);
        self.env.note_off();
        self.note = None;
    }

    /// Generate one stereo sample from this voice.
    ///
    /// Applies fine-tune detune to the base frequency, runs the oscillator
    /// and envelope, multiplies by velocity, then applies constant-power
    /// stereo panning.
    ///
    /// Returns `(left, right)`. Returns `(0.0, 0.0)` when idle.
    pub fn render_sample(&mut self, fine_tune_cents: f32, sample_rate: f32) -> (f32, f32) {
        if self.env.is_idle() {
            return (0.0, 0.0);
        }

        // Apply fine-tune detune to the cached base frequency.
        let freq = apply_detune(self.base_freq, fine_tune_cents);
        self.osc.set_frequency(freq, sample_rate);

        // Generate audio: oscillator × envelope × smoothed velocity.
        let osc_sample = self.osc.next_sample();
        let env_sample = self.env.next_sample();
        let velocity = self.velocity_smoother.next_sample();
        let mono_output = osc_sample * env_sample * velocity;

        // Use cached pan gains — avoids sin/cos per sample since pan
        // only changes on discrete PolyPan events.
        (
            mono_output * self.pan_gains.left,
            mono_output * self.pan_gains.right,
        )
    }

    /// Zero all DSP state. Called by `Plugin::reset()`.
    pub fn reset(&mut self) {
        self.osc.reset();
        self.env.reset();
        self.velocity_smoother.reset();
        self.note = None;
        self.base_freq = 0.0;
        self.pan = 0.0;
        self.pan_gains = PanGains::default();
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

    const SR: f32 = 44100.0;
    const A4_FREQ: f32 = 440.0;
    const A4_NOTE: u8 = 69;

    /// Helper: trigger a note on a voice with standard test parameters.
    fn trigger_voice(voice: &mut Voice, note: u8, velocity: f32, start_phase_norm: f32) {
        let base_freq = util::midi_note_to_freq(note);
        voice.note_on(NoteOnParams {
            note,
            velocity,
            base_freq,
            start_phase_normalized: start_phase_norm,
            sample_rate: SR,
            attack_ms: 10.0,
            age: 1,
        });
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
            let (l, r) = voice.render_sample(0.0, SR);
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
            voice.render_sample(0.0, SR);
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
            voice.render_sample(0.0, SR);
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
            voice.render_sample(0.0, SR);
        }

        voice.reset();
        assert!(voice.is_idle(), "voice should be idle after reset");
        assert_eq!(voice.note(), None, "note should be None after reset");
        assert_eq!(voice.pan(), 0.0, "pan should be 0.0 after reset");
        assert_eq!(voice.age(), 0, "age should be 0 after reset");

        // Should produce silence.
        let (l, r) = voice.render_sample(0.0, SR);
        assert_eq!((l, r), (0.0, 0.0), "reset voice should output silence");
    }

    #[test]
    fn voice_render_returns_stereo() {
        let mut voice = Voice::default();
        // Use non-zero pan to verify stereo routing.
        voice.set_pan(1.0); // hard right
        trigger_voice(&mut voice, A4_NOTE, 1.0, 0.25); // 90° start phase for immediate signal

        // Render a few samples to get past zero crossing.
        let mut left_sum = 0.0_f32;
        let mut right_sum = 0.0_f32;
        for _ in 0..50 {
            let (l, r) = voice.render_sample(0.0, SR);
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
            voice.render_sample(0.0, SR);
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
    fn voice_retrigger_resets_phase_at_nonzero_start() {
        let mut voice = Voice::default();
        trigger_voice(&mut voice, A4_NOTE, 0.5, 0.0);

        // Advance to build up some state.
        for _ in 0..200 {
            voice.render_sample(0.0, SR);
        }
        assert!(!voice.is_idle());

        // Retrigger at 90° (0.25 normalized) — should reset phase and jump velocity.
        voice.note_on(NoteOnParams {
            note: A4_NOTE,
            velocity: 1.0,
            base_freq: A4_FREQ,
            start_phase_normalized: 0.25,
            sample_rate: SR,
            attack_ms: 10.0,
            age: 2,
        });

        // Velocity should jump immediately to 1.0.
        let vel = voice.velocity_smoother.next_sample();
        assert!(
            (vel - 1.0).abs() < 1e-6,
            "velocity should jump on non-zero start phase retrigger, got {vel}"
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
            v.render_sample(0.0, SR);
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
}
