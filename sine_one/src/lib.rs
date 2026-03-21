pub mod dsp;
mod params;
mod plugin;

// Re-export so integration tests and the standalone binary can access the type.
pub use plugin::SineOne;

use nih_plug::prelude::*;

nih_export_clap!(SineOne);
