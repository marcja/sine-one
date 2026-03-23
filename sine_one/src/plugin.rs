use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use crate::dsp::smoother::LinearSmoother;
use crate::dsp::voice::{allocate_voice, NoteOnParams, Voice, MAX_VOICES};
use crate::params::SineOneParams;

/// A polyphonic sine-wave synthesizer CLAP plugin.
///
/// DSP state lives here on the plugin struct, NOT in Params. Params holds
/// only what the user/host controls; DSP state is ephemeral and not persisted.
///
/// Voice state is stored in a fixed-size array of `Voice` structs. The `voices`
/// parameter controls how many slots are active (1–8). When `voices=1`, the
/// plugin behaves identically to its original monophonic design.
pub struct SineOne {
    params: Arc<SineOneParams>,

    /// Cached sample rate from the last `initialize()` call.
    sample_rate: f32,
    /// Fixed-size array of voice slots. Only `voices[0..voice_count]` are
    /// used for allocation; voices beyond that count are not assigned new
    /// notes but may still produce sound while their envelopes release.
    voices: [Voice; MAX_VOICES],
    /// Monotonically increasing counter for voice age tracking. Bumped on
    /// each `note_on()` and assigned to the voice, so the allocator can
    /// identify the oldest voice for stealing.
    next_voice_age: u64,
    /// Smooths the gain compensation factor (1.0 / voice_count) to avoid
    /// clicks when the voice count parameter changes at runtime.
    gain_smoother: LinearSmoother,
    /// Previous voice count, used to detect runtime voice count changes
    /// and release excess voices when the count decreases.
    previous_voice_count: usize,
}

impl Default for SineOne {
    fn default() -> Self {
        Self {
            params: Arc::new(SineOneParams::default()),
            sample_rate: 0.0,
            voices: core::array::from_fn(|_| Voice::default()),
            next_voice_age: 0,
            gain_smoother: LinearSmoother::default(),
            previous_voice_count: 1,
        }
    }
}

/// Gain compensation for polyphonic summing: 1/sqrt(N).
fn voice_gain(voice_count: usize) -> f32 {
    1.0 / (voice_count as f32).sqrt()
}

impl SineOne {
    /// Sync the gain smoother to the current voice-count parameter.
    /// Called from both `initialize()` and `reset()` so that `process()`
    /// always starts with the correct gain level.
    fn sync_voice_gain(&mut self) {
        let voice_count = self.params.voices.value() as usize;
        self.gain_smoother.set_immediate(voice_gain(voice_count));
        self.previous_voice_count = voice_count;
    }
}

impl Plugin for SineOne {
    const NAME: &'static str = "SineOne";
    const VENDOR: &'static str = "sine-one";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrument: no audio input, stereo output.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Accept NoteOn, NoteOff, and polyphonic note expressions (PolyPan).
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // Cache sample rate for use in process() when computing frequencies
        // and envelope times from parameter values.
        self.sample_rate = buffer_config.sample_rate;

        self.sync_voice_gain();

        true
    }

    fn reset(&mut self) {
        // Zero all DSP state so the plugin starts from silence.
        for voice in &mut self.voices {
            voice.reset();
        }
        self.next_voice_age = 0;
        self.gain_smoother.reset();
        self.sync_voice_gain();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        // Read voice count once per process block.
        let voice_count = (self.params.voices.value() as usize).clamp(1, MAX_VOICES);

        // Detect voice count changes: release excess voices and update gain.
        if voice_count != self.previous_voice_count {
            if voice_count < self.previous_voice_count {
                for voice in &mut self.voices[voice_count..self.previous_voice_count] {
                    if !voice.is_idle() {
                        voice.note_off(self.params.release.value(), self.sample_rate);
                    }
                }
            }
            self.previous_voice_count = voice_count;

            // Smooth gain change over ~5ms to avoid clicks on voice count transitions.
            let target_gain = voice_gain(voice_count);
            let ramp_samples = (0.005 * self.sample_rate) as u32;
            self.gain_smoother.set_target(target_gain, ramp_samples);
        }

        // Read output gain once per block — no smoother, so the value is
        // constant across all samples. Hoisted to avoid per-sample powf.
        let output_gain = util::db_to_gain(self.params.output_gain.value());

        for (sample_idx, mut channel_samples) in buffer.iter_samples().enumerate() {
            // Handle all MIDI events scheduled at this sample.
            while let Some(event) = next_event {
                if event.timing() != sample_idx as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        let idx = allocate_voice(&self.voices, voice_count);
                        let start_phase_normalized = self.params.start_phase.value() / 360.0 % 1.0;

                        self.next_voice_age += 1;
                        self.voices[idx].note_on(NoteOnParams {
                            note,
                            velocity,
                            base_freq: util::midi_note_to_freq(note),
                            start_phase_normalized,
                            sample_rate: self.sample_rate,
                            attack_ms: self.params.attack.value(),
                            age: self.next_voice_age,
                        });
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        // NOTE(note_off_scan): Scan all voice slots, not just
                        //   [..voice_count]. Voices beyond voice_count may still
                        //   be releasing and need NoteOff to reach Idle.
                        let target = self
                            .voices
                            .iter()
                            .enumerate()
                            .filter(|(_, v)| v.note() == Some(note))
                            .min_by_key(|(_, v)| v.age());

                        if let Some((idx, _)) = target {
                            self.voices[idx]
                                .note_off(self.params.release.value(), self.sample_rate);
                        }
                    }
                    // NOTE(pan_smoothing): Pan gains are smoothed over ~2ms
                    //   via LinearSmoother in Voice to prevent clicks when
                    //   PolyPan events arrive while the voice is producing
                    //   audio (common in mono mode with Bitwig's Randomize).
                    NoteEvent::PolyPan { note, pan, .. } => {
                        if voice_count == 1 {
                            // NOTE(mono_pan): In mono mode, accept all PolyPan
                            //   regardless of note field. PolyPan may arrive before
                            //   NoteOn in the same buffer (e.g., from Bitwig's
                            //   Randomize device).
                            self.voices[0].set_pan(pan, self.sample_rate);
                        } else {
                            // Route to the voice playing this note.
                            if let Some(voice) =
                                self.voices.iter_mut().find(|v| v.note() == Some(note))
                            {
                                voice.set_pan(pan, self.sample_rate);
                            }
                        }
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // Read smoothed per-sample values to support real-time automation.
            // The smoothers must be consumed every sample even when no note is active.
            let fine_tune_cents = self.params.fine_tune.smoothed.next();
            let fold = self.params.fold.smoothed.next();
            let gain = self.gain_smoother.next_sample();

            // Sum all active voices.
            let mut left_sum = 0.0_f32;
            let mut right_sum = 0.0_f32;
            for voice in &mut self.voices {
                if !voice.is_idle() {
                    let (l, r) = voice.render_sample(fine_tune_cents, fold, self.sample_rate);
                    left_sum += l;
                    right_sum += r;
                }
            }

            // Apply voice gain compensation and output gain.
            let combined_gain = gain * output_gain;
            left_sum *= combined_gain;
            right_sum *= combined_gain;

            // Write to stereo output. AUDIO_IO_LAYOUTS guarantees exactly 2 channels.
            let mut channels = channel_samples.iter_mut();
            if let Some(left) = channels.next() {
                *left = left_sum;
            }
            if let Some(right) = channels.next() {
                *right = right_sum;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for SineOne {
    const CLAP_ID: &'static str = "com.sine-one.sine-one";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Minimal polyphonic sine-wave synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Mono,
    ];
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// Minimal mock for `InitContext` — nih-plug does not expose test utilities,
    /// so we implement just enough to call `Plugin::initialize()`.
    struct MockInitContext;

    impl InitContext<SineOne> for MockInitContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }

        fn execute(&self, _task: ()) {}

        fn set_latency_samples(&self, _samples: u32) {}

        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    /// Mock `ProcessContext` that feeds a pre-built list of MIDI events.
    /// Events are consumed in order by `next_event()`.
    struct MockProcessContext {
        events: VecDeque<NoteEvent<()>>,
        transport: Transport,
    }

    impl MockProcessContext {
        fn new(sample_rate: f32, events: Vec<NoteEvent<()>>) -> Self {
            // HACK(transport): Transport::new() is pub(crate), so we zero-initialize
            //   and set public fields. All pub(crate) fields are Option types that are
            //   valid when zeroed (None). This is fragile if nih-plug adds non-zero-safe
            //   fields — pin the nih-plug git dependency and revisit on updates.
            let mut transport: Transport = unsafe { std::mem::zeroed() };
            transport.sample_rate = sample_rate;
            Self {
                events: events.into(),
                transport,
            }
        }
    }

    impl ProcessContext<SineOne> for MockProcessContext {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }

        fn execute_background(&self, _task: ()) {}

        fn execute_gui(&self, _task: ()) {}

        fn transport(&self) -> &Transport {
            &self.transport
        }

        fn next_event(&mut self) -> Option<NoteEvent<()>> {
            self.events.pop_front()
        }

        fn send_event(&mut self, _event: NoteEvent<()>) {}

        fn set_latency_samples(&self, _samples: u32) {}

        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    /// Helper: call initialize() then reset() on a plugin instance, matching
    /// the real nih-plug host lifecycle (Default → initialize → reset → process).
    fn initialize_plugin(mut plugin: SineOne, sample_rate: f32) -> SineOne {
        let layout = SineOne::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate,
            min_buffer_size: None,
            max_buffer_size: 512,
            process_mode: ProcessMode::Realtime,
        };
        let result = plugin.initialize(&layout, &config, &mut MockInitContext);
        assert!(result, "initialize() should return true");
        plugin.reset();
        plugin
    }

    /// Helper: construct a default plugin and call initialize().
    fn init_plugin(sample_rate: f32) -> SineOne {
        initialize_plugin(SineOne::default(), sample_rate)
    }

    /// Helper: construct a plugin with a custom voice count and call initialize().
    fn init_plugin_with_voices(sample_rate: f32, voice_count: i32) -> SineOne {
        let plugin = SineOne {
            params: Arc::new(SineOneParams::with_voices(voice_count)),
            ..SineOne::default()
        };
        initialize_plugin(plugin, sample_rate)
    }

    /// Helper: construct a plugin with a custom start_phase and call initialize().
    fn init_plugin_with_start_phase(sample_rate: f32, degrees: f32) -> SineOne {
        let plugin = SineOne {
            params: Arc::new(SineOneParams::with_start_phase(degrees)),
            ..SineOne::default()
        };
        initialize_plugin(plugin, sample_rate)
    }

    /// Helper: construct a plugin with a custom output gain (dB) and call initialize().
    fn init_plugin_with_output_gain(sample_rate: f32, db: f32) -> SineOne {
        let plugin = SineOne {
            params: Arc::new(SineOneParams::with_output_gain(db)),
            ..SineOne::default()
        };
        initialize_plugin(plugin, sample_rate)
    }

    /// Helper: call `process()` on the plugin with a stereo buffer of the given
    /// size and the provided MIDI events. Returns the left and right channel data.
    fn run_process(
        plugin: &mut SineOne,
        num_samples: usize,
        events: Vec<NoteEvent<()>>,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut left = vec![0.0f32; num_samples];
        let mut right = vec![0.0f32; num_samples];
        let mut buffer = Buffer::default();
        unsafe {
            buffer.set_slices(num_samples, |output_slices| {
                *output_slices = vec![&mut left, &mut right];
            });
        }
        let mut aux = AuxiliaryBuffers {
            inputs: &mut [],
            outputs: &mut [],
        };
        let mut context = MockProcessContext::new(plugin.sample_rate, events);
        let status = plugin.process(&mut buffer, &mut aux, &mut context);
        assert_eq!(status, ProcessStatus::Normal);
        (left, right)
    }

    /// Compute root-mean-square of a sample buffer.
    fn rms(samples: &[f32]) -> f32 {
        let sum: f32 = samples.iter().map(|s| s * s).sum();
        (sum / samples.len() as f32).sqrt()
    }

    /// Shorthand for constructing a NoteOn event on channel 0 with no voice ID.
    fn note_on(timing: u32, note: u8, velocity: f32) -> NoteEvent<()> {
        NoteEvent::NoteOn {
            timing,
            voice_id: None,
            channel: 0,
            note,
            velocity,
        }
    }

    /// Shorthand for constructing a NoteOff event on channel 0 with no voice ID.
    fn note_off(timing: u32, note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOff {
            timing,
            voice_id: None,
            channel: 0,
            note,
            velocity: 0.0,
        }
    }

    #[test]
    fn plugin_can_be_constructed() {
        let plugin = SineOne::default();
        // Verify params() returns a valid Arc — this exercises the Plugin
        // trait wiring without needing audio buffers.
        let _params = plugin.params();
    }

    /// Number of samples to skip in pan assertions to allow the ~2ms pan
    /// ramp to complete. Derived from `PAN_RAMP_SECS` at 44100 Hz + margin.
    const PAN_RAMP_MARGIN: usize = (crate::dsp::voice::PAN_RAMP_SECS * 44100.0) as usize + 12;

    #[test]
    fn initialize_stores_sample_rate() {
        let plugin = init_plugin(48000.0);
        assert_eq!(plugin.sample_rate, 48000.0);
    }

    #[test]
    fn initialize_returns_true_at_common_rates() {
        // Verify initialize succeeds at standard sample rates.
        for &sr in &[44100.0, 48000.0, 88200.0, 96000.0, 192000.0] {
            let _ = init_plugin(sr);
        }
    }

    #[test]
    fn reset_zeros_all_voices() {
        let mut plugin = init_plugin(44100.0);

        // Trigger a note to dirty voice state, then reset.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        run_process(&mut plugin, 100, events);

        plugin.reset();

        // After reset, all voices should be idle and produce silence.
        for (i, voice) in plugin.voices.iter().enumerate() {
            assert!(voice.is_idle(), "voice {i} should be idle after reset");
        }
        let (left, right) = run_process(&mut plugin, 64, vec![]);
        assert!(
            left.iter().all(|&s| s == 0.0),
            "should produce silence after reset"
        );
        assert!(
            right.iter().all(|&s| s == 0.0),
            "should produce silence after reset"
        );
    }

    #[test]
    fn silence_before_note_on() {
        let mut plugin = init_plugin(44100.0);

        // Process 512 samples with no MIDI events — output must be all zeros.
        let (left, right) = run_process(&mut plugin, 512, vec![]);

        assert!(
            left.iter().all(|&s| s == 0.0),
            "left channel should be silent before any NoteOn"
        );
        assert!(
            right.iter().all(|&s| s == 0.0),
            "right channel should be silent before any NoteOn"
        );
    }

    #[test]
    fn note_on_produces_nonzero_output() {
        let mut plugin = init_plugin(44100.0);

        // Send NoteOn at sample 0: A4 (note 69), velocity ~0.79 (100/127).
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 100.0 / 127.0,
        }];
        let (left, _right) = run_process(&mut plugin, 512, events);

        // At least some samples should be nonzero after the NoteOn triggers
        // the oscillator and envelope.
        let nonzero_count = left.iter().filter(|&&s| s != 0.0).count();
        assert!(
            nonzero_count > 0,
            "expected nonzero output after NoteOn, but all 512 samples were zero"
        );
    }

    #[test]
    fn note_off_eventually_silences() {
        let mut plugin = init_plugin(44100.0);

        // NoteOn at sample 0.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        // Process enough samples to complete the attack phase (default 10 ms = 441 samples).
        run_process(&mut plugin, 512, events);

        // NoteOff at sample 0 of the next buffer.
        let events = vec![NoteEvent::NoteOff {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 0.0,
        }];
        // Default release is 300 ms at 44100 Hz = 13230 samples. Process well beyond that.
        let release_samples = (300.0 * 44100.0 / 1000.0) as usize; // 13230
        let margin = 1000; // generous safety margin past release end
        let total_samples = release_samples + margin + 2000;
        let (left, _right) = run_process(&mut plugin, total_samples, events);

        // The last samples should be zero (envelope has reached Idle).
        let tail = &left[release_samples + margin..];
        assert!(
            tail.iter().all(|&s| s == 0.0),
            "expected silence after release completes, but found nonzero samples in tail"
        );
    }

    #[test]
    fn center_pan_both_channels_equal() {
        let mut plugin = init_plugin(44100.0);

        // With default center pan (0.0), both channels should be equal.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (left, right) = run_process(&mut plugin, 256, events);

        assert_eq!(
            left, right,
            "left and right channels should be equal at center pan"
        );
    }

    #[test]
    fn poly_pan_hard_left_silences_right() {
        let mut plugin = init_plugin(44100.0);

        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                pan: -1.0,
            },
        ];
        let (left, right) = run_process(&mut plugin, 256, events);

        // Left channel should have nonzero output.
        assert!(
            left.iter().any(|&s| s != 0.0),
            "left channel should have nonzero output at hard-left pan"
        );
        // Right channel should be silent after the pan ramp settles.
        assert!(
            right[PAN_RAMP_MARGIN..].iter().all(|&s| s.abs() < 1e-6),
            "right channel should be silent at hard-left pan (after ramp)"
        );
    }

    #[test]
    fn poly_pan_hard_right_silences_left() {
        let mut plugin = init_plugin(44100.0);

        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                pan: 1.0,
            },
        ];
        let (left, right) = run_process(&mut plugin, 256, events);

        // Right channel should have nonzero output.
        assert!(
            right.iter().any(|&s| s != 0.0),
            "right channel should have nonzero output at hard-right pan"
        );
        // Left channel should be silent after the pan ramp settles.
        assert!(
            left[PAN_RAMP_MARGIN..].iter().all(|&s| s.abs() < 1e-6),
            "left channel should be silent at hard-right pan (after ramp)"
        );
    }

    #[test]
    fn poly_pan_mid_buffer_timing() {
        let mut plugin = init_plugin(44100.0);
        let mid = 128u32;

        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
            NoteEvent::PolyPan {
                timing: mid,
                voice_id: None,
                channel: 0,
                note: 69,
                pan: 1.0,
            },
        ];
        let (left, right) = run_process(&mut plugin, 256, events);

        // Before the PolyPan (samples 0..128), center pan → L == R.
        // Skip sample 0 because the envelope starts from zero.
        for i in 1..mid as usize {
            assert!(
                (left[i] - right[i]).abs() < 1e-6,
                "before PolyPan at sample {i}: left {} != right {}",
                left[i],
                right[i]
            );
        }

        // After the PolyPan and pan ramp settles, hard-right → left should be ~0.
        let pan_settled = mid as usize + PAN_RAMP_MARGIN;
        for i in pan_settled..256 {
            assert!(
                left[i].abs() < 1e-6,
                "after hard-right pan at sample {i}: left {} should be ~0",
                left[i]
            );
        }
    }

    #[test]
    fn reset_clears_pan() {
        let mut plugin = init_plugin(44100.0);

        // Set a non-default pan via PolyPan event, then reset.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                pan: 0.5,
            },
        ];
        run_process(&mut plugin, 64, events);
        plugin.reset();

        // After reset, pan should be centered — verify by checking equal L/R output.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (left, right) = run_process(&mut plugin, 64, events);
        assert_eq!(left, right, "after reset, pan should be centered (L == R)");
    }

    #[test]
    fn poly_pan_persists_across_notes() {
        let mut plugin = init_plugin(44100.0);

        // Set pan first, then start a note — pan should persist.
        let events = vec![
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                pan: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
        ];
        let (left, right) = run_process(&mut plugin, 256, events);

        // Hard-right pan should persist: left silent (after ramp), right nonzero.
        assert!(
            right.iter().any(|&s| s != 0.0),
            "right channel should have output when pan persists"
        );
        assert!(
            left[PAN_RAMP_MARGIN..].iter().all(|&s| s.abs() < 1e-6),
            "left channel should be silent when pan is hard-right (after ramp)"
        );
    }

    #[test]
    fn note_on_resets_phase_to_zero() {
        let mut plugin = init_plugin(44100.0);

        // First NoteOn: capture first 64 samples of output.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (first_left, _) = run_process(&mut plugin, 64, events);

        // Advance 200 samples to move the oscillator phase forward.
        run_process(&mut plugin, 200, vec![]);

        // NoteOff and let release complete.
        let events = vec![NoteEvent::NoteOff {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 0.0,
        }];
        run_process(&mut plugin, 20000, events);

        // Second NoteOn: capture first 64 samples.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (second_left, _) = run_process(&mut plugin, 64, events);

        // Both NoteOns should produce identical output (deterministic phase).
        assert_eq!(
            first_left, second_left,
            "two NoteOns should produce identical output when phase is retriggered"
        );
    }

    #[test]
    fn note_on_with_90_degree_start_phase() {
        let mut plugin_0 = init_plugin(44100.0);
        let mut plugin_90 = init_plugin_with_start_phase(44100.0, 90.0);

        let events = || {
            vec![NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            }]
        };

        let (left_0, _) = run_process(&mut plugin_0, 64, events());
        let (left_90, _) = run_process(&mut plugin_90, 64, events());

        // The outputs should differ because the oscillator starts at different phases.
        assert_ne!(
            left_0, left_90,
            "0° and 90° start phase should produce different output"
        );
    }

    #[test]
    fn retrigger_continues_phase() {
        let mut plugin = init_plugin(44100.0);

        // First NoteOn — let oscillator run for 200 samples.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (pre_left, _) = run_process(&mut plugin, 200, events);

        // Capture last sample before retrigger.
        let last_before = pre_left[199];

        // Second NoteOn (retrigger, no NoteOff). Phase should continue,
        // not reset — avoiding a waveform discontinuity (click).
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (retrigger_left, _) = run_process(&mut plugin, 32, events);

        // The transition should be smooth: the difference between the last
        // sample before retrigger and the first sample after should be
        // bounded by the maximum single-sample delta of a 440 Hz sine at
        // 44100 Hz (≈ 2π * 440 / 44100 ≈ 0.063), plus some envelope margin.
        let delta = (retrigger_left[0] - last_before).abs();
        assert!(
            delta < 0.15,
            "retrigger should be smooth (phase continues), but delta was {delta}"
        );
    }

    #[test]
    fn retrigger_from_idle_resets_phase() {
        let mut plugin = init_plugin(44100.0);

        // First NoteOn: capture first 64 samples.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (first_left, _) = run_process(&mut plugin, 64, events);

        // Advance, NoteOff, and wait for full release (envelope → Idle).
        run_process(&mut plugin, 200, vec![]);
        let events = vec![NoteEvent::NoteOff {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 0.0,
        }];
        run_process(&mut plugin, 20000, events);

        // Second NoteOn from idle: phase should be reset, producing
        // identical output to the first NoteOn.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (second_left, _) = run_process(&mut plugin, 64, events);

        assert_eq!(
            first_left, second_left,
            "NoteOn from idle should reset phase and produce identical output"
        );
    }

    #[test]
    fn retrigger_velocity_change_is_smooth() {
        let mut plugin = init_plugin(44100.0);

        // NoteOn at full velocity, process enough for attack to complete.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (pre_left, _) = run_process(&mut plugin, 500, events);
        let last_before = pre_left[499];

        // Retrigger at very low velocity — big gain change.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 0.2,
        }];
        let (retrigger_left, _) = run_process(&mut plugin, 256, events);

        // The first sample after retrigger should not jump by more than
        // the natural oscillator delta + a small margin. Without smoothing,
        // the gain would drop by 0.8× instantly.
        let delta = (retrigger_left[0] - last_before).abs();
        assert!(
            delta < 0.15,
            "velocity change should be smooth, but first-sample delta was {delta}"
        );
    }

    #[test]
    fn retrigger_with_nonzero_start_phase_is_smooth() {
        let mut plugin = init_plugin_with_start_phase(44100.0, 90.0);

        // First NoteOn: process enough for attack to complete.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (pre_left, _) = run_process(&mut plugin, 500, events);
        let last_before = pre_left[499];

        // Second NoteOn (retrigger). Even at 90° start phase, retrigger
        // should continue the oscillator phase (no reset) to prevent
        // uncontrolled clicks while the envelope is at a non-zero level.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (retrigger_left, _) = run_process(&mut plugin, 32, events);

        // The first sample after retrigger should be continuous with the
        // waveform before retrigger (no sudden jump to sin(90°) = 1.0).
        let diff = (retrigger_left[0] - last_before).abs();
        assert!(
            diff < 0.1,
            "retrigger should be smooth (phase continues), but jump was {diff}"
        );
    }

    // --- Polyphonic tests ---

    #[test]
    fn two_notes_produce_two_voices() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Play two different notes simultaneously.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60, // C4
                velocity: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 64, // E4
                velocity: 1.0,
            },
        ];
        run_process(&mut plugin, 256, events);

        // Two voices should be active.
        let active = plugin.voices.iter().filter(|v| !v.is_idle()).count();
        assert_eq!(active, 2, "two notes should activate two voices");
    }

    #[test]
    fn voice_stealing_when_full() {
        let mut plugin = init_plugin_with_voices(44100.0, 2);

        // Fill both voice slots.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60,
                velocity: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 64,
                velocity: 1.0,
            },
        ];
        run_process(&mut plugin, 100, events);

        // Third note should steal the oldest voice.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 67,
            velocity: 1.0,
        }];
        run_process(&mut plugin, 100, events);

        // Note 67 should be playing. The stolen voice (note 60, oldest) was
        // replaced, so only notes 64 and 67 should be active.
        let active_notes: Vec<u8> = plugin.voices.iter().filter_map(|v| v.note()).collect();
        assert!(
            active_notes.contains(&67),
            "stolen voice should now play note 67, active: {active_notes:?}"
        );
        assert!(
            !active_notes.contains(&60),
            "oldest note (60) should have been stolen, active: {active_notes:?}"
        );
    }

    #[test]
    fn note_off_releases_correct_voice() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Play two notes.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60,
                velocity: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 64,
                velocity: 1.0,
            },
        ];
        run_process(&mut plugin, 256, events);

        // Release only note 60.
        let events = vec![NoteEvent::NoteOff {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 60,
            velocity: 0.0,
        }];
        run_process(&mut plugin, 64, events);

        // Note 64 should still be playing (not releasing).
        let still_holding: Vec<u8> = plugin.voices.iter().filter_map(|v| v.note()).collect();
        assert!(
            still_holding.contains(&64),
            "note 64 should still be held, active: {still_holding:?}"
        );
        assert!(
            !still_holding.contains(&60),
            "note 60 should have been released, active: {still_holding:?}"
        );
    }

    #[test]
    fn gain_compensation_scales_output() {
        // With 1/sqrt(N) gain compensation, a single note at voices=4
        // should be sqrt(4)=2x quieter than at voices=1.
        let mut plugin_1 = init_plugin_with_voices(44100.0, 1);
        let mut plugin_4 = init_plugin_with_voices(44100.0, 4);

        let events = || {
            vec![NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            }]
        };

        let (left_1, _) = run_process(&mut plugin_1, 512, events());
        let (left_4, _) = run_process(&mut plugin_4, 512, events());

        let rms_1 = rms(&left_1);
        let rms_4 = rms(&left_4);
        let ratio = rms_1 / rms_4;

        // 1/sqrt(4) = 0.5, so ratio should be ~2.0.
        assert!(
            (ratio - 2.0).abs() < 0.3,
            "gain compensation should make voices=4 about 2x quieter (1/sqrt(N)), ratio was {ratio}"
        );
    }

    #[test]
    fn polyphonic_output_is_sum_of_voices() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Play two notes simultaneously. The output should be nonzero and
        // differ from playing just one note.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60,
                velocity: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 67,
                velocity: 1.0,
            },
        ];
        let (left_two, _) = run_process(&mut plugin, 512, events);

        // Compare with a fresh plugin playing just one note.
        let mut plugin_one = init_plugin_with_voices(44100.0, 4);
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 60,
            velocity: 1.0,
        }];
        let (left_one, _) = run_process(&mut plugin_one, 512, events);

        // The two-note output should differ from the one-note output.
        assert_ne!(
            left_two, left_one,
            "two simultaneous notes should produce different output than one"
        );
    }

    // --- PolyPan routing tests ---

    #[test]
    fn poly_pan_routes_to_correct_voice() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Play one note and pan it hard-right via PolyPan targeting that note.
        // A second note has no PolyPan, so it stays at center.
        let events = vec![
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60,
                velocity: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 64,
                velocity: 1.0,
            },
            // Pan note 64 hard-right; note 60 stays at center.
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 64,
                pan: 1.0,
            },
        ];
        run_process(&mut plugin, 256, events);

        // Verify that the voice playing note 64 has pan 1.0, and the voice
        // playing note 60 is still at center (0.0).
        let voice_60 = plugin.voices.iter().find(|v| v.note() == Some(60));
        let voice_64 = plugin.voices.iter().find(|v| v.note() == Some(64));

        assert!(voice_60.is_some(), "voice playing note 60 should exist");
        assert!(voice_64.is_some(), "voice playing note 64 should exist");

        assert!(
            (voice_60.unwrap().pan() - 0.0).abs() < 1e-6,
            "note 60 should have center pan, got {}",
            voice_60.unwrap().pan()
        );
        assert!(
            (voice_64.unwrap().pan() - 1.0).abs() < 1e-6,
            "note 64 should have hard-right pan, got {}",
            voice_64.unwrap().pan()
        );
    }

    #[test]
    fn poly_pan_no_voice_is_ignored() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Send PolyPan for a note that isn't playing — should not panic.
        let events = vec![NoteEvent::PolyPan {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 72, // not playing
            pan: 1.0,
        }];
        run_process(&mut plugin, 64, events);
        // No panic = pass.
    }

    #[test]
    fn poly_pan_mono_mode_unchanged() {
        let mut plugin = init_plugin_with_voices(44100.0, 1);

        // In mono mode, PolyPan should still work regardless of note matching.
        let events = vec![
            NoteEvent::PolyPan {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 99, // different note than what we'll play
                pan: 1.0,
            },
            NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            },
        ];
        let (left, right) = run_process(&mut plugin, 256, events);

        // Hard-right pan: left should be ~0 (after ramp), right should have energy.
        assert!(
            right.iter().any(|&s| s != 0.0),
            "right channel should have output at hard-right pan in mono mode"
        );
        assert!(
            left[PAN_RAMP_MARGIN..].iter().all(|&s| s.abs() < 1e-6),
            "left channel should be silent at hard-right pan in mono mode (after ramp)"
        );
    }

    #[test]
    fn output_gain_attenuates_output() {
        // At -6 dB, output amplitude should be roughly half of 0 dB.
        let mut plugin_0db = init_plugin_with_output_gain(44100.0, 0.0);
        let mut plugin_minus6 = init_plugin_with_output_gain(44100.0, -6.0);

        let events = || {
            vec![NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            }]
        };

        let (left_0, _) = run_process(&mut plugin_0db, 512, events());
        let (left_6, _) = run_process(&mut plugin_minus6, 512, events());

        let ratio = rms(&left_0) / rms(&left_6);

        // -6 dB ≈ 0.501 linear gain, so ratio should be ~2.0.
        assert!(
            (ratio - 2.0).abs() < 0.3,
            "output at -6 dB should be ~half of 0 dB, ratio was {ratio}"
        );
    }

    #[test]
    fn output_gain_amplifies_output() {
        // At +6 dB, output amplitude should be roughly double 0 dB.
        let mut plugin_0db = init_plugin_with_output_gain(44100.0, 0.0);
        let mut plugin_plus6 = init_plugin_with_output_gain(44100.0, 6.0);

        let events = || {
            vec![NoteEvent::NoteOn {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 69,
                velocity: 1.0,
            }]
        };

        let (left_0, _) = run_process(&mut plugin_0db, 512, events());
        let (left_6, _) = run_process(&mut plugin_plus6, 512, events());

        let ratio = rms(&left_6) / rms(&left_0);

        // +6 dB ≈ 1.995 linear gain, so ratio should be ~2.0.
        assert!(
            (ratio - 2.0).abs() < 0.3,
            "output at +6 dB should be ~double 0 dB, ratio was {ratio}"
        );
    }

    #[test]
    fn voice_count_reduction_releases_excess_voices() {
        let mut plugin = init_plugin_with_voices(44100.0, 4);

        // Play 4 notes — one per voice slot.
        let events = vec![
            note_on(0, 60, 1.0),
            note_on(0, 64, 1.0),
            note_on(0, 67, 1.0),
            note_on(0, 72, 1.0),
        ];
        run_process(&mut plugin, 256, events);

        // All 4 voices should be active.
        for (i, voice) in plugin.voices[..4].iter().enumerate() {
            assert!(
                !voice.is_idle(),
                "voice {i} should be active before reduction"
            );
        }

        // Reduce voice count from 4 to 2 by swapping in new params.
        // NOTE: This replaces *all* params (not just voices) because nih-plug's
        // ParamMut::set_plain_value is pub(crate). Other params reset to defaults,
        // which is acceptable here because only voice count is checked.
        plugin.params = Arc::new(SineOneParams::with_voices(2));

        // Process another block — the reduction path should release voices 2 and 3.
        run_process(&mut plugin, 256, vec![]);

        // Voices beyond the new count should be releasing (or idle if release completed).
        for i in 2..4 {
            assert!(
                plugin.voices[i].is_releasing() || plugin.voices[i].is_idle(),
                "voice {i} should be releasing or idle after voice count reduction"
            );
        }
    }

    #[test]
    fn note_off_for_unplayed_note_is_noop() {
        let mut plugin = init_plugin(44100.0);

        // Play a note.
        run_process(&mut plugin, 256, vec![note_on(0, 69, 1.0)]);

        // Send NoteOff for a note that was never played.
        let (left_after, _) = run_process(&mut plugin, 256, vec![note_off(0, 42)]);

        // The active voice (note 69) should still be producing output.
        assert!(
            left_after.iter().any(|&s| s != 0.0),
            "NoteOff for unplayed note should not silence active voices"
        );
    }
}
