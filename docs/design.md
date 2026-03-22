# SineOne — Technical Design

**Version:** 0.1 (initial)  
**Purpose:** First nih-plug CLAP plugin — pedagogical reference build  
**Audience:** The author learning the nih-plug build/test workflow

---

## Overview

SineOne is a minimal polyphonic sine-wave synthesizer CLAP plugin. It accepts MIDI NoteOn/NoteOff
events, plays up to 8 simultaneous sine oscillators tuned to incoming note pitches, and shapes each
voice's output with a two-stage AR (Attack/Release) envelope. There is no filter, no GUI, and no
effects. The entire codebase is intentionally small so every line is traceable back to a design
decision documented here.

---

## Plugin Type & I/O

| Property | Value |
|---|---|
| Type | Instrument |
| Audio input | None |
| Audio output | Stereo (mono signal with constant-power panning via PolyPan expression events) |
| MIDI input | `MidiConfig::Basic` — NoteOn, NoteOff, and polyphonic expressions (PolyPan) |
| MIDI output | None |
| CLAP features | `CLAP_FEATURE_INSTRUMENT`, `CLAP_FEATURE_SYNTHESIZER`, `CLAP_FEATURE_MONO` (when Voices=1) |
| GUI | None — generic host parameter list |

**Why stereo out?** Most DAWs expect instruments to produce stereo output. The plugin applies
constant-power panning (via `PolyPan` note expression events) to position the mono signal in the
stereo field. With no PolyPan event, output defaults to center (equal L/R).

---

## DSP Architecture

```
                 ┌─────────────── Voice 0 ───────────────────────┐
MIDI NoteOn ──►  │ SineOscillator → ArEnvelope → Vel → Pan Gains │──┐
allocate_voice() └───────────────────────────────────────────────┘  │
                 ┌─────────────── Voice 1 ───────────────────────┐  ├─► Sum → Gain(1/√N) → Output Gain → [L,R]
                 │ SineOscillator → ArEnvelope → Vel → Pan Gains │──┤
                 └───────────────────────────────────────────────┘  │
                 ...up to Voice 7                                   ┘
MIDI NoteOff ──► oldest voice with matching note → Release phase
MIDI PolyPan ──► voice with matching note → update pan gains
```

### SineOscillator

A simple phase accumulator. On each sample:

```
phase += frequency / sample_rate
if phase >= 1.0 { phase -= 1.0 }
output = sin(phase * 2π)
```

**Sine is already band-limited** — no PolyBLEP or oversampling is needed, unlike saw or square
oscillators. This makes it the ideal choice for a first plugin.

**Frequency calculation** combines the MIDI note and the fine-tune parameter:

```
base_freq  = midi_note_to_freq(note)       // nih_plug::util::midi_note_to_freq
detune_mult = 2^(fine_tune_cents / 1200)   // 1200 cents per octave
frequency  = base_freq * detune_mult
```

### ArEnvelope

A three-state linear envelope (`EnvState::Idle`, `Attack`, `Release`). "Holding" is not a
distinct state — it is the Attack state with level clamped at 1.0.

```
State machine:

  note_on()        attack complete      note_off()       level reaches 0
  ──────────►  Attack ──────────────► (holding) ──────► Release ──────────► Idle
                  ^                                         |
                  └─────── note_on() from any state ────────┘  (retrigger from current level)
```

State transition rules:
- **`note_on()`**: enters `Attack` from the current level (preserves level on retrigger). The
  attack increment is scaled to the remaining distance: `(1.0 - level) / attack_samples`, so
  retrigger from a non-zero level still takes the full attack duration to reach 1.0.
- **Attack**: increments `level` by `attack_increment` per sample; clamps at 1.0 (stays here
  until NoteOff arrives)
- **`note_off()`**: enters `Release` from any non-Idle state. The release decrement is computed
  from the level at the moment `note_off()` is called, so a mid-attack release ramps from the
  current level (not from 1.0). If level is 0.0 (e.g., note_off immediately after note_on),
  transitions directly to Idle.
- **Release**: decrements `level` by `current_level_at_release_start / release_samples` per sample
  (linear ramp from wherever the envelope currently is, to zero, over `release_samples`); when
  `level ≤ 0` → `Idle`
- **Idle**: outputs 0.0

**Phase retrigger on NoteOn:** Phase reset behavior depends on context:

- **From silence** (envelope Idle): phase always resets to `start_phase`. The attack envelope
  ramps from zero, so no click occurs at any start phase.
- **Retrigger at 0° start phase**: phase continues from its current position. Since `sin(0) = 0`,
  a hard reset would create a waveform dip to zero (audible click). Phase continuity avoids this,
  giving a smooth retrigger. Velocity ramps over ~2ms via `LinearSmoother`.
- **Retrigger at non-zero start phase**: phase resets to `start_phase`, creating an intentional
  transient whose magnitude scales with `sin(start_phase)`. This is the desired "click" character
  that `start_phase` controls — 0° = smooth, 90° = maximum punch. Velocity jumps immediately.

The `start_phase` parameter (0–360°) thus serves double duty: it sets the initial waveform
position for notes from silence, and controls the retrigger transient character during performance.

### Velocity Scaling

In nih-plug, `NoteEvent::NoteOn { velocity, .. }` provides velocity as an `f32` already
normalized to [0.0, 1.0] (not a raw `u8` 0–127). The output is scaled directly:

```
output = osc_sample * envelope_level * velocity
```

Velocity is applied per-sample via a `LinearSmoother`. When starting from silence (envelope
Idle), velocity jumps to the target immediately — the attack envelope already provides a smooth
fade-in. On retrigger, velocity ramps linearly over ~2ms to avoid a gain discontinuity (click)
when the new note has a different velocity than the previous one.

### Stereo Panning

The plugin responds to `PolyPan` note expression events (available at `MidiConfig::Basic` in
nih-plug). Pan is a per-note value in [-1.0, 1.0]: -1.0 = hard left, 0.0 = center, 1.0 = hard
right. The default is center (equal L/R).

**Constant-power pan law** preserves perceived loudness across the stereo field:

```
theta = (pan + 1) * PI/4       — maps [-1, 1] to [0, PI/2]
left  = mono_output * cos(theta)
right = mono_output * sin(theta)
```

At center: `cos(PI/4) = sin(PI/4) = 1/sqrt(2)`, so `left² + right² = mono²` (constant power).
At hard left: `cos(0) = 1`, `sin(0) = 0` — full signal in L, silence in R. Vice versa at hard
right.

**Why constant-power instead of linear?** Linear panning (`left = (1-pan)/2`) causes a ~3 dB
perceived volume dip at center. Constant-power avoids this and is the industry-standard approach.

**Pan is not smoothed.** Changes apply instantly at the sample they arrive. For per-note pan
from devices like Bitwig's Randomize, zipper noise is unlikely. If continuous pan automation is
added later, a one-pole smoother could be inserted here.

**Pan persists across notes.** A PolyPan event sets the pan position until the next PolyPan or
`reset()`. It is not reset on NoteOn. This matches the behavior of Bitwig's Randomize device,
which may send PolyPan before or after NoteOn in the same buffer.

**Polyphonic PolyPan routing:** When `Voices > 1`, PolyPan events are routed to the voice
playing the matching MIDI note. When `Voices = 1` (mono mode), all PolyPan events are accepted
regardless of the `note` field — strict note matching would drop events that arrive before NoteOn.

---

## Polyphony

SineOne supports 1–8 polyphonic voices via the `Voices` parameter. Each voice is an independent
signal path (oscillator + envelope + velocity + pan). The plugin sums all active voices and applies
gain compensation.

### Voice Architecture

```
                 ┌─ Voice 0: Osc → Env → Vel → Pan ─┐
MIDI NoteOn ──►  ├─ Voice 1: Osc → Env → Vel → Pan ─┤──► Sum ──► Gain(1/√N) ──► Output Gain ──► [L,R]
                 ├─ Voice 2: ...                     ┤
allocate_voice() └─ Voice N-1: ...                   ┘
```

Each `Voice` struct (in `dsp/voice.rs`) bundles `SineOscillator`, `ArEnvelope`, `LinearSmoother`
(velocity), pan gains, a MIDI note, a base frequency, and a monotonic age counter. All fields are
stack-allocated — no heap allocation in the audio path.

### Voice Stealing: Oldest Voice with Release Priority

When all voice slots are occupied and a new NoteOn arrives, the allocator steals a voice:

1. **First idle voice** — no stealing needed
2. **Oldest voice in Release state** — already fading out, least disruptive to steal
3. **Oldest voice in Attack/hold state** — last resort

"Oldest" = lowest `age` counter (a `u64` bumped on each `note_on()`).

This strategy was chosen for its simplicity (one comparison, no amplitude analysis) and good
musical results (releasing voices are preferentially reclaimed).

### Gain Compensation

The summed output is scaled by `1/√N` (RMS-based compensation), then by the user's output gain:

```
voice_gain  = 1.0 / sqrt(voices_param)
output_gain = db_to_gain(output_gain_param)
output_L    = sum_L * voice_gain * output_gain
output_R    = sum_R * voice_gain * output_gain
```

**Why `1/√N` instead of `1/N`?** Linear `1/N` scaling assumes all voices peak simultaneously
(worst-case coherent addition). In practice, voices are statistically independent — different
pitches, phases, and envelope stages make simultaneous peaking unlikely. `1/√N` (RMS-based
scaling) preserves perceived loudness across voice counts: a single note at `Voices=4` is √4 = 2×
quieter than at `Voices=1`, not 4×. This is standard practice in polyphonic synthesizers.

The voice gain factor is smoothed over ~5ms via `LinearSmoother` to avoid clicks when the `Voices`
parameter changes at runtime. Output gain is read once per process block (no smoothing).

**Output Gain attenuverter** (-24 dB to +12 dB, default 0 dB): Provides manual control over the
final output level. At 0 dB (unity), the plugin's output level is determined entirely by voice gain
compensation and velocity. Negative values attenuate; positive values amplify (up to +12 dB ≈ 4×).

### NoteOff Routing

On NoteOff, the plugin scans **all** voice slots (not just `[0..voice_count]`) for the oldest
voice playing the matching note. This ensures voices that are releasing beyond the current voice
count (after the user decreased `Voices`) can still be found and released properly.

### Voice Count Changes at Runtime

When `Voices` decreases from N to M (M < N):
- Voices in slots M..N receive `note_off()` and fade out via their release envelopes
- New notes are only allocated to slots 0..M
- The render loop still ticks all MAX_VOICES slots to allow releasing voices to complete

When `Voices` increases: no immediate effect. New slots become available for the next NoteOn.

---

## Parameter System

| Parameter | `#[id]` | Type | Range | Default | Smoothing | Unit | Notes |
|---|---|---|---|---|---|---|---|
| Fine Tune | `"fine_tune"` | `FloatParam` | −100 to +100 | 0.0 | `Linear(20ms)` | cents | ±1 semitone |
| Attack | `"attack"` | `FloatParam` | 1 to 5000 | 10.0 | `None` | ms | Skewed (log-ish feel) |
| Release | `"release"` | `FloatParam` | 1 to 10000 | 300.0 | `None` | ms | Skewed (log-ish feel) |
| Start Phase | `"start_phase"` | `FloatParam` | 0 to 360 | 0.0 | `None` | ° | Oscillator phase on NoteOn |
| Voices | `"voices"` | `IntParam` | 1 to 8 | 1 | `None` | — | Polyphonic voice count |
| Output Gain | `"output_gain"` | `FloatParam` | −24 to +12 | 0.0 | `None` | dB | Attenuverter; read per-block |

**Notes on smoothing choices:**
- `Fine Tune` uses `Linear(20ms)` so pitch slides smoothly when automated (avoids zipper noise).
- `Attack` and `Release` use `None` because they control the *shape* of the next envelope, not a
  sample-by-sample audio signal. Their value is read at note-on/note-off boundaries, not
  per-sample. Smoothing here would be meaningless.

**Notes on range skewing:**
- For time parameters (attack, release), the perceptually useful range is very nonlinear: the
  difference between 1ms and 10ms matters a lot; the difference between 4990ms and 5000ms is
  imperceptible. Use `FloatRange::Skewed` with `FloatRange::skew_factor(-2.0)` to spread the low
  end. This is a one-liner in nih-plug.

**The `#[id]` contract:** The `#[id = "..."]` string is what the DAW persists in project files.
It must never change after the first real session is saved with this plugin. It is decoupled from
the display name on purpose.

---

## State & Preset Design

All plugin state is captured by the six `Params` fields — no custom serialization needed. When
the DAW saves a project or preset, nih-plug serializes the `Params` struct automatically via the
`#[derive(Params)]` macro.

DSP state (oscillator phases, envelope states, current velocities, current notes, voice ages) is
**not** persisted. On reload, all voices start silent (Idle envelope, phase 0) and wait for the
next NoteOn, which is correct behavior.

---

## File & Module Structure

```
sine-one/
├── Cargo.toml              # workspace manifest
├── Cargo.lock
├── bundler.toml            # [sine_one] name = "SineOne" — lives at workspace root
├── xtask/
│   ├── Cargo.toml          # [dependencies] nih_plug_xtask = { git = "..." }
│   └── src/
│       └── main.rs         # "deploy" subcommand + nih_plug_xtask delegation
└── sine_one/
    ├── Cargo.toml          # plugin crate (see Cargo.toml section below)
    └── src/
        ├── lib.rs          # nih_export_clap!(plugin::SineOne); re-exports
        ├── plugin.rs       # SineOne struct + Plugin trait impl (process, initialize, reset)
        ├── params.rs       # SineOneParams struct with five FloatParams + one IntParam
        ├── dsp/
        │   ├── mod.rs      # pub mod oscillator; envelope; pan; smoother; voice;
        │   ├── oscillator.rs   # SineOscillator: phase, set_frequency(), next_sample(), reset()
        │   ├── envelope.rs     # ArEnvelope: EnvState enum, ArEnvelope struct, all methods
        │   ├── smoother.rs     # LinearSmoother: linear ramp for click-free parameter transitions
        │   ├── pan.rs          # apply_constant_power_pan(): stereo panning via sin/cos pan law
        │   └── voice.rs        # Voice: per-voice DSP bundle; allocate_voice(): voice stealing
        └── main.rs         # standalone binary: nih_export_standalone::<SineOne>()
```

**Pedagogical note on the split:** `params.rs`, `plugin.rs`, and `dsp/` are deliberately in
separate files so you can read each concern in isolation:
- `params.rs` = "what does the user control?"
- `dsp/` = "what does the audio math look like?"
- `plugin.rs` = "how does nih-plug wire them together?"

### Key Cargo.toml sections

```toml
# sine_one/Cargo.toml

[lib]
crate-type = ["cdylib", "lib"]   # cdylib for plugin shared library; lib for tests/benches

[features]
standalone = ["nih_plug/standalone"]

[[bin]]
name = "sine_one_standalone"
path = "src/main.rs"
required-features = ["standalone"]

[dependencies]
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git",
             default-features = false,
             features = ["assert_process_allocs"] }

[dev-dependencies]
proptest = "1"
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "dsp_bench"
harness = false
```

**`assert_process_allocs`**: This feature causes debug builds to abort on any heap allocation
inside `process()`. It fires immediately when you accidentally allocate in the hot path (e.g., by
pushing to a Vec, formatting a String). Keep it on during all development.

---

## Testing Strategy

### Layer 1 — Unit tests (`cargo test`)

Tests live in `#[cfg(test)]` blocks in the same file as the struct under test.

**`oscillator.rs`:**
- `sine_output_in_range` — 1000 samples of `next_sample()` should all be in [-1.0, 1.0]
- `sine_phase_is_periodic` — sample at phase=0 and phase=1.0 (one full period later) should be
  equal within floating-point tolerance
- `reset_clears_phase` — after `reset()`, `next_sample()` at the same frequency produces the same
  output as a freshly constructed oscillator
- `midi_note_to_freq_a4` — note 69 should yield 440.0 Hz exactly (nih-plug utility function;
  test it once to confirm you're calling it correctly)
- `fine_tune_zero_cents_no_change` — 0 cents offset leaves frequency unchanged
- `fine_tune_1200_cents_octave_up` — 1200 cents doubles the frequency

**`envelope.rs`:**
- `idle_outputs_zero` — fresh envelope outputs 0.0
- `attack_ramps_up` — after `note_on()`, samples increase monotonically from 0 toward 1.0
- `attack_reaches_one` — after enough samples (≥ attack_samples), level is exactly 1.0
- `hold_stays_at_one` — once attack completes, level remains 1.0 until `note_off()` is called
- `release_ramps_down` — after `note_off()`, samples decrease monotonically
- `release_reaches_idle` — after enough samples, state is `Idle` and level is 0.0
- `retrigger_preserves_level` — calling `note_on()` mid-attack preserves the current level
- `retrigger_during_release_preserves_level` — calling `note_on()` mid-release preserves the current level

**`pan.rs`:**
- `center_pan_equal_channels` — pan=0 produces L == R ≈ sample × 1/√2
- `hard_left_silences_right` — pan=-1 → right=0, left=sample
- `hard_right_silences_left` — pan=1 → left=0, right=sample
- `constant_power_at_center` — L² + R² ≈ sample²
- `pan_is_monotonic` — as pan goes -1→1, left decreases, right increases

**`params.rs`:**
- `param_defaults_in_range` — verify each param's default value is within its declared min/max
  (simple smoke test; catches a common copy-paste error)

### Layer 2 — Property-based tests (`proptest`)

**`oscillator.rs`:**
```rust
proptest! {
    fn sine_is_always_finite(freq in 20.0f32..20000.0, sr in 22050.0f32..192000.0) {
        let mut osc = SineOscillator::new();
        osc.set_frequency(freq, sr);
        for _ in 0..512 {
            prop_assert!(osc.next_sample().is_finite());
        }
    }
}
```

**`pan.rs`:**
```rust
proptest! {
    fn constant_power_across_range(pan in -1.0f32..=1.0, sample in -1.0f32..=1.0) {
        // left² + right² should equal sample² (constant power) and both must be finite
    }
}
```

**`envelope.rs`:**
```rust
proptest! {
    fn envelope_output_bounded(attack_ms in 1.0f32..5000.0, release_ms in 1.0f32..10000.0) {
        // After note_on → hold → note_off, output is always in [0.0, 1.0] and is_finite()
        let sr = 44100.0;
        let mut env = ArEnvelope::new();
        env.set_attack(attack_ms, sr);
        env.set_release(release_ms, sr);
        env.note_on();
        for _ in 0..(attack_ms * sr / 1000.0) as usize + 10 {
            let v = env.next_sample();
            prop_assert!(v.is_finite() && v >= 0.0 && v <= 1.0);
        }
        env.note_off();
        for _ in 0..(release_ms * sr / 1000.0) as usize + 10 {
            let v = env.next_sample();
            prop_assert!(v.is_finite() && v >= 0.0 && v <= 1.0);
        }
    }
}
```

### Layer 3 — Integration tests (plugin lifecycle)

These live in the `#[cfg(test)]` module at the bottom of `sine_one/src/plugin.rs`, using mock
`InitContext` and `ProcessContext` implementations. They exercise the full plugin lifecycle
(initialize → reset → process) without a real DAW or audio driver.

Tests:
- `plugin_can_be_constructed` — verify `SineOne::default()` creates a valid plugin and `params()`
  returns a valid Arc
- `initialize_stores_sample_rate` — call `initialize()` at 48000 Hz, verify `sample_rate` is cached
- `initialize_returns_true_at_common_rates` — verify `initialize()` succeeds at 44100, 48000,
  88200, 96000, and 192000 Hz
- `reset_zeros_oscillator` — dirty the oscillator, call `reset()`, verify it matches a fresh one
- `reset_zeros_envelope` — trigger the envelope, call `reset()`, verify it outputs 0.0 (Idle)
- `reset_clears_note_and_velocity` — set active note and velocity, call `reset()`, verify both cleared
- `silence_before_note_on` — process 512 samples with no MIDI events; output must be all zeros
- `note_on_produces_nonzero_output` — send NoteOn(note=69, velocity≈0.79), process 512 samples;
  at least some output should be nonzero
- `note_off_eventually_silences` — after NoteOff with default release (300ms), output should reach
  zero within the expected number of samples
- `center_pan_both_channels_equal` — verify L and R channels are equal at default center pan
- `poly_pan_hard_left_silences_right` — hard-left pan produces nonzero L, silent R
- `poly_pan_hard_right_silences_left` — hard-right pan produces nonzero R, silent L
- `poly_pan_mid_buffer_timing` — PolyPan at sample 128: center before, panned after
- `reset_clears_pan` — `reset()` returns pan to center (0.0)
- `poly_pan_persists_across_notes` — pan set before NoteOn persists (not reset on NoteOn)

**Pedagogical note:** These tests don't require a real DAW or audio driver. You instantiate the
plugin struct directly and call its methods with mock contexts. This is the layer that confirms
"does my plugin work end-to-end as a Rust program?" before you ever open Bitwig.

### Layer 4 — CLAP compliance (`clap-validator`)

Run after every `cargo xtask bundle`:

```bash
clap-validator validate target/bundled/SineOne.clap --only-failed
```

What it checks automatically:
- Plugin scan time (<100ms warning threshold)
- Parameter text-to-value and value-to-text round-trips
- State save and load round-trips
- Basic threading invariants
- 50 random parameter permutations × 5 audio buffers (fuzz pass)
- Descriptor validity (CLAP features list, etc.)

If this produces zero failures, you're safe to load in Bitwig.

### Performance Benchmarks (`criterion`)

Target: process a 512-sample block well under the ~11.6ms budget at 44100 Hz.
The DSP is trivial, so these benchmarks primarily teach the benchmark workflow.

Three benchmarks in `benches/dsp_bench.rs`:
- `oscillator_512_samples` — oscillator only, 512 samples at 440 Hz / 44100 Hz
- `envelope_attack_512_samples` — envelope only, 512 samples during attack phase
- `combined_dsp_512_samples` — oscillator × envelope × velocity, mirroring the per-sample work
  in `process()`

```rust
// benches/dsp_bench.rs (simplified)
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn oscillator_512_samples(c: &mut Criterion) {
    c.bench_function("oscillator_512_samples", |b| {
        let mut osc = SineOscillator::default();
        osc.set_frequency(440.0, 44100.0);
        b.iter(|| {
            for _ in 0..512 {
                black_box(osc.next_sample());
            }
        });
    });
}

criterion_group!(benches, oscillator_512_samples, /* ... */);
criterion_main!(benches);
```

### Realtime Safety Checklist

- [x] `assert_process_allocs` feature enabled in `Cargo.toml`
- [x] No `Vec::push`, `String::new`, or any allocation in `process()`
- [x] No `Mutex` or `RwLock` in `process()` (no GUI shared state in this plugin anyway)
- [x] Voice array (`[Voice; 8]`) lives directly on the plugin struct (stack/inline), not in a `Box`
- [x] Per-voice pan gains are cached at event-rate, avoiding per-sample sin/cos calls

---

## Build & Test Plan

### Day-0 Setup (one-time)

```bash
# Rust toolchain
rustup target add aarch64-apple-darwin

# Tools
cargo install cargo-watch    # optional: watch mode for fast iteration
cargo install clap-validator # CLAP compliance testing
```

### Development Loop

```bash
# Fast check after every edit (seconds, no binary produced)
cargo check

# Lint — treat warnings as errors
cargo clippy -- -D warnings

# Run all tests (no audio hardware needed)
cargo test

# Build CLAP bundle
cargo xtask bundle sine_one --release
```

### Deploy (`cargo xtask deploy`)

Builds, validates, and installs in one step:

```bash
cargo xtask deploy
```

This runs:
1. `cargo xtask bundle sine_one --release` — build the CLAP bundle
2. `clap-validator validate target/bundled/SineOne.clap --only-failed` — validate compliance
3. Copy bundle to `~/Library/Audio/Plug-Ins/CLAP/`

The deploy subcommand is implemented in `xtask/src/main.rs`. All other xtask
subcommands (e.g., `bundle`) are delegated to `nih_plug_xtask::main()`.

### Gatekeeper (first install only)

```bash
xattr -d com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/SineOne.clap
```

### Standalone Binary (GUI-less audio preview)

```bash
cargo run -p sine_one --features standalone -- --output "Built-in Output"
# Opens plugin with CPAL audio; send MIDI from any source; hear output without Bitwig
```

### Bitwig Smoke Tests (manual, after install)

1. **Plugin loads** — appears in Bitwig browser under Instruments
2. **Parameters visible** — Fine Tune, Attack, Release, Start Phase, Voices, Output Gain appear in
   the device panel with correct ranges and units
3. **Note produces sound** — draw a MIDI note; hear a sine tone
4. **AR envelope audible** — set Attack to 500ms; note should fade in; set Release to 1000ms;
   note should fade out after key release
5. **Fine tune works** — automate Fine Tune from -100 to +100 cents; pitch should sweep smoothly
6. **State save/load** — save project, close, reopen; parameters restore correctly
7. **Smooth retrigger** — rapid MIDI notes should not produce audible clicks or dips to silence
8. **Pan expression** — add a Randomize device before SineOne; randomize Pan; hear the signal
   move in the stereo field between notes

---

## Pre-Commit Hook

Install at `.git/hooks/pre-commit`:

```bash
#!/bin/sh
set -e
cargo fmt -- --check
cargo clippy -- -D warnings
cargo check
cargo test
```

Make it executable: `chmod +x .git/hooks/pre-commit`

---

## Open Questions

1. **~~Retrigger from mid-release~~** *(resolved)*: `note_on()` now preserves the current level
   and re-enters Attack from wherever the envelope is. This avoids the audible dip to zero on
   fast retrigger and produces smoother legato behavior.

2. **Velocity curve**: The current `velocity / 127.0` is linear (fully linear velocity response).
   A quadratic or logarithmic curve (`velocity^2 / 127^2`) often feels more natural on keyboards.
   Leave linear for now; easy to change in one place later.

3. **~~Phase initialization~~** *(resolved)*: The oscillator phase is reset to the configurable
   `start_phase` parameter (0–360°, default 0°) on NoteOn **only when starting from silence**
   (envelope Idle). On retrigger, the phase continues from its current position to avoid a
   waveform discontinuity (click). Velocity is also smoothed over ~2ms on retrigger via a
   `LinearSmoother` to avoid gain discontinuities.

4. **~~Mono vs. stereo output layout~~** *(resolved)*: The plugin now applies constant-power
   panning via `PolyPan` note expression events. At center pan (default), both channels carry
   equal signal. The `dsp/pan.rs` module implements the sin/cos pan law.
