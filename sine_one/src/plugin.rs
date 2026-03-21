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

    // Accept NoteOn / NoteOff only.
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
    }

    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // TODO(process): implement NoteOn/NoteOff event handling and
        //   per-sample audio output once DSP structs are wired in.
        //   Host pre-zeroes the buffer, so silence is the default output.
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

        plugin.reset();

        assert_eq!(
            plugin.current_note, None,
            "current_note should be None after reset"
        );
        assert_eq!(
            plugin.current_velocity, 0.0,
            "current_velocity should be 0.0 after reset"
        );
    }
}
