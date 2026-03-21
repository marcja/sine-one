use nih_plug::prelude::*;

/// Plugin parameters — user/host-controllable values.
///
/// Currently empty; FloatParams (fine_tune, attack, release) will be added
/// in a dedicated [params] commit once DSP structs exist to consume them.
#[derive(Params, Default)]
pub struct SineOneParams {}
