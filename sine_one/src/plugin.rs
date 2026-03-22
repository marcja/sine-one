use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use crate::dsp::envelope::ArEnvelope;
use crate::dsp::oscillator::SineOscillator;
use crate::params::SineOneParams;

/// A minimal monophonic sine-wave synthesizer CLAP plugin.
///
/// DSP state lives here on the plugin struct, NOT in Params. Params holds
/// only what the user/host controls; DSP state is ephemeral and not persisted.
pub struct SineOne {
    params: Arc<SineOneParams>,

    /// Cached sample rate from the last `initialize()` call.
    sample_rate: f32,
    oscillator: SineOscillator,
    envelope: ArEnvelope,
    /// Currently held MIDI note number, or `None` if no note is active.
    current_note: Option<u8>,
    /// Velocity of the current note, normalized to [0.0, 1.0].
    current_velocity: f32,
    /// Cached base frequency (Hz) for the current note, before fine-tune.
    /// Stored to avoid recomputing `midi_note_to_freq` every sample.
    current_base_freq: f32,
    /// Current pan position in [-1.0, 1.0]. Updated by PolyPan events.
    /// -1.0 = hard left, 0.0 = center, 1.0 = hard right.
    current_pan: f32,
}

impl Default for SineOne {
    fn default() -> Self {
        Self {
            params: Arc::new(SineOneParams::default()),
            sample_rate: 0.0,
            oscillator: SineOscillator::default(),
            envelope: ArEnvelope::default(),
            current_note: None,
            current_velocity: 0.0,
            current_base_freq: 0.0,
            current_pan: 0.0,
        }
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

        // Pre-compute envelope times from current parameter values so the
        // envelope is ready before the first note-on event arrives.
        self.envelope
            .set_attack(self.params.attack.value(), self.sample_rate);
        self.envelope
            .set_release(self.params.release.value(), self.sample_rate);

        true
    }

    fn reset(&mut self) {
        // Zero all DSP state so the plugin starts from silence.
        self.oscillator.reset();
        self.envelope.reset();
        self.current_note = None;
        self.current_velocity = 0.0;
        self.current_base_freq = 0.0;
        self.current_pan = 0.0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        for (sample_idx, mut channel_samples) in buffer.iter_samples().enumerate() {
            // Handle all MIDI events scheduled at this sample.
            while let Some(event) = next_event {
                if event.timing() != sample_idx as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        self.current_note = Some(note);
                        // nih-plug normalizes velocity to [0.0, 1.0].
                        self.current_velocity = velocity;

                        // Cache the base frequency for this note. The per-sample loop
                        // applies fine-tune on top of this each sample, so we avoid
                        // recomputing midi_note_to_freq every sample.
                        self.current_base_freq = util::midi_note_to_freq(note);

                        // Read attack time from params and trigger the envelope.
                        self.envelope
                            .set_attack(self.params.attack.value(), self.sample_rate);
                        self.envelope.note_on();
                    }
                    NoteEvent::NoteOff { note, .. } => {
                        // Only respond to NoteOff for the currently held note.
                        if self.current_note == Some(note) {
                            self.envelope
                                .set_release(self.params.release.value(), self.sample_rate);
                            self.envelope.note_off();
                            self.current_note = None;
                        }
                    }
                    // NOTE(poly_pan): We accept all PolyPan events regardless of the
                    //   note field. This is a monophonic synth with one voice, and
                    //   PolyPan may arrive before NoteOn in the same buffer (e.g., from
                    //   Bitwig's Randomize device). Strict note matching would drop
                    //   those events.
                    // REVIEW(pan_smoothing): Pan changes apply instantly at the sample
                    //   they arrive. If continuous pan automation causes audible zipper
                    //   noise, add a one-pole smoother here. Unlikely for per-note pan
                    //   from Bitwig's Randomize device.
                    NoteEvent::PolyPan { pan, .. } => {
                        self.current_pan = pan;
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // Read smoothed fine-tune value per-sample to support real-time
            // pitch modulation (vibrato, automation). The smoother must be
            // consumed every sample even when no note is active.
            let fine_tune_cents = self.params.fine_tune.smoothed.next();

            // Update oscillator frequency per-sample when a note is active.
            if self.current_note.is_some() {
                let freq =
                    crate::dsp::oscillator::apply_detune(self.current_base_freq, fine_tune_cents);
                self.oscillator.set_frequency(freq, self.sample_rate);
            }

            // Generate audio: oscillator × envelope × velocity.
            let osc_sample = self.oscillator.next_sample();
            let env_sample = self.envelope.next_sample();
            let mono_output = osc_sample * env_sample * self.current_velocity;

            // Apply constant-power stereo panning.
            let (left_sample, right_sample) =
                crate::dsp::pan::apply_constant_power_pan(mono_output, self.current_pan);

            // Write to stereo output. AUDIO_IO_LAYOUTS guarantees exactly 2 channels.
            let mut channels = channel_samples.iter_mut();
            if let Some(left) = channels.next() {
                *left = left_sample;
            }
            if let Some(right) = channels.next() {
                *right = right_sample;
            }
        }

        ProcessStatus::Normal
    }
}

impl ClapPlugin for SineOne {
    const CLAP_ID: &'static str = "com.sine-one.sine-one";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Minimal monophonic sine-wave synthesizer");
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

    /// Helper: construct a plugin and call initialize() with the given sample rate.
    fn init_plugin(sample_rate: f32) -> SineOne {
        let mut plugin = SineOne::default();
        let layout = SineOne::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate,
            min_buffer_size: None,
            max_buffer_size: 512,
            process_mode: ProcessMode::Realtime,
        };
        let result = plugin.initialize(&layout, &config, &mut MockInitContext);
        assert!(result, "initialize() should return true");
        plugin
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

    #[test]
    fn plugin_can_be_constructed() {
        let plugin = SineOne::default();
        // Verify params() returns a valid Arc — this exercises the Plugin
        // trait wiring without needing audio buffers.
        let _params = plugin.params();
    }

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
    fn reset_zeros_oscillator() {
        let mut plugin = init_plugin(44100.0);

        // Dirty the oscillator by setting a frequency and advancing samples.
        plugin.oscillator.set_frequency(440.0, 44100.0);
        for _ in 0..50 {
            plugin.oscillator.next_sample();
        }

        plugin.reset();

        // After reset, oscillator should behave identically to a fresh one.
        let mut fresh = SineOscillator::default();
        fresh.set_frequency(440.0, 44100.0);
        plugin.oscillator.set_frequency(440.0, 44100.0);
        assert_eq!(plugin.oscillator.next_sample(), fresh.next_sample());
    }

    #[test]
    fn reset_zeros_envelope() {
        let mut plugin = init_plugin(44100.0);

        // Trigger the envelope so it's in a non-idle state.
        plugin.envelope.set_attack(10.0, 44100.0);
        plugin.envelope.note_on();
        for _ in 0..100 {
            plugin.envelope.next_sample();
        }

        plugin.reset();

        // After reset, envelope should output 0.0 (Idle).
        assert_eq!(plugin.envelope.next_sample(), 0.0);
    }

    #[test]
    fn reset_clears_note_and_velocity() {
        let mut plugin = init_plugin(44100.0);

        // Simulate a note being active.
        plugin.current_note = Some(69);
        plugin.current_velocity = 100.0 / 127.0;
        plugin.current_base_freq = 440.0;

        plugin.reset();

        assert_eq!(
            plugin.current_note, None,
            "current_note should be None after reset"
        );
        assert_eq!(
            plugin.current_velocity, 0.0,
            "current_velocity should be 0.0 after reset"
        );
        assert_eq!(
            plugin.current_base_freq, 0.0,
            "current_base_freq should be 0.0 after reset"
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
        // Right channel should be silent.
        assert!(
            right.iter().all(|&s| s.abs() < 1e-6),
            "right channel should be silent at hard-left pan"
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
        // Left channel should be silent.
        assert!(
            left.iter().all(|&s| s.abs() < 1e-6),
            "left channel should be silent at hard-right pan"
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

        // After the PolyPan (samples 128..256), hard-right → left should be ~0.
        for i in mid as usize..256 {
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

        // Set a non-default pan value, then reset.
        plugin.current_pan = 0.5;
        plugin.reset();

        assert_eq!(
            plugin.current_pan, 0.0,
            "current_pan should be 0.0 after reset"
        );
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

        // Hard-right pan should persist: left silent, right nonzero.
        assert!(
            right.iter().any(|&s| s != 0.0),
            "right channel should have output when pan persists"
        );
        assert!(
            left.iter().all(|&s| s.abs() < 1e-6),
            "left channel should be silent when pan is hard-right"
        );
    }
}
