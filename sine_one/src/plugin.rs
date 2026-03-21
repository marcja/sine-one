use std::num::NonZeroU32;
use std::sync::Arc;

use nih_plug::prelude::*;

use crate::params::SineOneParams;

/// A minimal monophonic sine-wave synthesizer CLAP plugin.
///
/// DSP state (oscillator, envelope, current note/velocity) will be added in
/// subsequent commits. This stub satisfies the Plugin trait so the crate
/// compiles as a valid CLAP plugin.
pub struct SineOne {
    params: Arc<SineOneParams>,
}

impl Default for SineOne {
    fn default() -> Self {
        Self {
            params: Arc::new(SineOneParams::default()),
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
        _buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        // TODO(initialize): compute sample-rate-dependent values here once
        //   DSP structs exist (e.g., converting ms to samples for envelope).
        true
    }

    fn reset(&mut self) {
        // TODO(reset): zero all DSP state (oscillator phase, envelope level,
        //   envelope state, current note, current velocity).
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

    #[test]
    fn plugin_can_be_constructed() {
        let plugin = SineOne::default();
        // Verify params() returns a valid Arc — this exercises the Plugin
        // trait wiring without needing audio buffers.
        let _params = plugin.params();
    }
}
