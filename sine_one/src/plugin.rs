use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use crate::dsp::voice::{NoteOnParams, Voice, MAX_VOICES};
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
}

impl Default for SineOne {
    fn default() -> Self {
        Self {
            params: Arc::new(SineOneParams::default()),
            sample_rate: 0.0,
            voices: core::array::from_fn(|_| Voice::default()),
            next_voice_age: 0,
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
        true
    }

    fn reset(&mut self) {
        // Zero all DSP state so the plugin starts from silence.
        for voice in &mut self.voices {
            voice.reset();
        }
        self.next_voice_age = 0;
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut next_event = context.next_event();

        // For this monophonic-compatible refactor, all events route to voices[0].
        // The full polyphonic allocator is wired in a subsequent commit.
        let voice = &mut self.voices[0];

        for (sample_idx, mut channel_samples) in buffer.iter_samples().enumerate() {
            // Handle all MIDI events scheduled at this sample.
            while let Some(event) = next_event {
                if event.timing() != sample_idx as u32 {
                    break;
                }

                match event {
                    NoteEvent::NoteOn { note, velocity, .. } => {
                        // Convert start_phase from degrees (0–360) to normalized [0, 1).
                        let start_phase_normalized = self.params.start_phase.value() / 360.0 % 1.0;

                        self.next_voice_age += 1;
                        voice.note_on(NoteOnParams {
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
                        // Only respond to NoteOff for the currently held note.
                        if voice.note() == Some(note) {
                            voice.note_off(self.params.release.value(), self.sample_rate);
                        }
                    }
                    // NOTE(poly_pan): We accept all PolyPan events regardless of the
                    //   note field. With voices=1, PolyPan may arrive before NoteOn in
                    //   the same buffer (e.g., from Bitwig's Randomize device). Strict
                    //   note matching would drop those events.
                    // REVIEW(pan_smoothing): Pan changes apply instantly at the sample
                    //   they arrive. If continuous pan automation causes audible zipper
                    //   noise, add a one-pole smoother here. Unlikely for per-note pan
                    //   from Bitwig's Randomize device.
                    NoteEvent::PolyPan { pan, .. } => {
                        voice.set_pan(pan);
                    }
                    _ => (),
                }

                next_event = context.next_event();
            }

            // Read smoothed fine-tune value per-sample to support real-time
            // pitch modulation (vibrato, automation). The smoother must be
            // consumed every sample even when no note is active.
            let fine_tune_cents = self.params.fine_tune.smoothed.next();

            // Generate audio from voice and write to stereo output.
            let (left_sample, right_sample) =
                voice.render_sample(fine_tune_cents, self.sample_rate);

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

    /// Helper: call initialize() on a plugin instance with the given sample rate.
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
        plugin
    }

    /// Helper: construct a default plugin and call initialize().
    fn init_plugin(sample_rate: f32) -> SineOne {
        initialize_plugin(SineOne::default(), sample_rate)
    }

    /// Helper: construct a plugin with a custom start_phase and call initialize().
    fn init_plugin_with_start_phase(sample_rate: f32, degrees: f32) -> SineOne {
        let plugin = SineOne {
            params: Arc::new(SineOneParams::with_start_phase(degrees)),
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
    fn retrigger_with_nonzero_start_phase_resets_phase() {
        let mut plugin = init_plugin_with_start_phase(44100.0, 90.0);

        // First NoteOn: process enough for attack to complete.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        run_process(&mut plugin, 500, events);

        // Second NoteOn (retrigger). At 90° start phase, the oscillator
        // should reset to phase 0.25 (sin(π/2) = 1.0), creating an
        // intentional transient.
        let events = vec![NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note: 69,
            velocity: 1.0,
        }];
        let (retrigger_left, _) = run_process(&mut plugin, 32, events);

        // At 90° start phase, the first sample should be near the peak
        // of the sine wave (sin(0.25 * 2π) = 1.0) × envelope level × velocity.
        // The envelope is near 1.0 after 500 samples of attack (10ms = 441 samples).
        // So the first sample should be close to 1.0.
        assert!(
            retrigger_left[0] > 0.7,
            "90° start phase should reset oscillator to peak on retrigger, got {}",
            retrigger_left[0]
        );
    }
}
