# SineOne — Technical Design

**Version:** 0.1 (initial)  
**Purpose:** First nih-plug CLAP plugin — pedagogical reference build  
**Audience:** The author learning the nih-plug build/test workflow

---

## Overview

SineOne is a minimal monophonic sine-wave synthesizer CLAP plugin. It accepts MIDI NoteOn/NoteOff
events, plays a sine oscillator tuned to the incoming note pitch, and shapes the output with a
two-stage AR (Attack/Release) envelope. There is no filter, no GUI, and no polyphony. The entire
codebase is intentionally small so every line is traceable back to a design decision documented here.

---

## Plugin Type & I/O

| Property | Value |
|---|---|
| Type | Instrument |
| Audio input | None |
| Audio output | Stereo (both channels carry the same mono signal) |
| MIDI input | `MidiConfig::Basic` — NoteOn + NoteOff only |
| MIDI output | None |
| CLAP features | `CLAP_FEATURE_INSTRUMENT`, `CLAP_FEATURE_SYNTHESIZER`, `CLAP_FEATURE_MONO` |
| GUI | None — generic host parameter list |

**Why stereo out with a mono signal?** Most DAWs expect instruments to produce stereo output.
Producing identical L/R is the simplest way to satisfy that without adding any panning or
width DSP to the plugin itself.

---

## DSP Architecture

```
MIDI NoteOn ──► SineOscillator ──► ArEnvelope ──► gain (velocity) ──► [L] out
                    │                                                    [R] out (same)
MIDI NoteOff ───────┘ (triggers Release phase)
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
                  └─────── note_on() from any state ────────┘  (retrigger resets to 0)
```

State transition rules:
- **`note_on()`**: always resets `level` to 0.0 and enters `Attack`
- **Attack**: increments `level` by `1.0 / attack_samples` per sample; clamps at 1.0 (stays here
  until NoteOff arrives)
- **`note_off()`**: enters `Release` from any non-Idle state. The release decrement is computed
  from the level at the moment `note_off()` is called, so a mid-attack release ramps from the
  current level (not from 1.0). If level is 0.0 (e.g., note_off immediately after note_on),
  transitions directly to Idle.
- **Release**: decrements `level` by `current_level_at_release_start / release_samples` per sample
  (linear ramp from wherever the envelope currently is, to zero, over `release_samples`); when
  `level ≤ 0` → `Idle`
- **Idle**: outputs 0.0; oscillator phase continues advancing (so it's in a reasonable position
  on retrigger — no click from a phase discontinuity)

**Why not reset oscillator phase on NoteOn?** For a sine wave, resetting phase to 0 on every
NoteOn can cause a click if the previous sample wasn't near zero. Letting the phase run freely
is safe for a sine and avoids this. (Contrast: for a saw or square, you'd reset to minimize
transients.)

### Velocity Scaling

In nih-plug, `NoteEvent::NoteOn { velocity, .. }` provides velocity as an `f32` already
normalized to [0.0, 1.0] (not a raw `u8` 0–127). The output is scaled directly:

```
output = osc_sample * envelope_level * velocity
```

This velocity value is stored on the plugin struct at note-on and applied per-sample in the
process loop.

---

## Parameter System

| Parameter | `#[id]` | Type | Range | Default | Smoothing | Unit | Notes |
|---|---|---|---|---|---|---|---|
| Fine Tune | `"fine_tune"` | `FloatParam` | −100 to +100 | 0.0 | `Linear(20ms)` | cents | ±1 semitone |
| Attack | `"attack"` | `FloatParam` | 1 to 5000 | 10.0 | `None` | ms | Skewed (log-ish feel) |
| Release | `"release"` | `FloatParam` | 1 to 10000 | 300.0 | `None` | ms | Skewed (log-ish feel) |

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

All plugin state is captured by the three `Params` fields — no custom serialization needed. When
the DAW saves a project or preset, nih-plug serializes the `Params` struct automatically via the
`#[derive(Params)]` macro.

DSP state (oscillator phase, envelope state, current velocity, current note) is **not** persisted.
On reload, the voice starts silent (Idle envelope, phase 0) and waits for the next NoteOn, which
is correct behavior.

---

## File & Module Structure

```
sine-one/
├── Cargo.toml              # workspace manifest
├── Cargo.lock
├── bundler.toml            # [sine_one] name = "SineOne" — lives at workspace root
├── deploy.sh               # build + validate + install (see Build & Test Plan)
├── xtask/
│   ├── Cargo.toml          # [dependencies] nih_plug_xtask = { git = "..." }
│   └── src/
│       └── main.rs         # fn main() { nih_plug_xtask::main() }
└── sine_one/
    ├── Cargo.toml          # plugin crate (see Cargo.toml section below)
    └── src/
        ├── lib.rs          # nih_export_clap!(plugin::SineOne); re-exports
        ├── plugin.rs       # SineOne struct + Plugin trait impl (process, initialize, reset)
        ├── params.rs       # SineOneParams struct with three FloatParams
        ├── dsp/
        │   ├── mod.rs      # pub mod oscillator; pub mod envelope;
        │   ├── oscillator.rs   # SineOscillator: phase, set_frequency(), next_sample(), reset()
        │   └── envelope.rs     # ArEnvelope: EnvState enum, ArEnvelope struct, all methods
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
- `retrigger_resets_level` — calling `note_on()` during release resets level to 0.0

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
- `both_channels_equal` — verify L and R channels carry identical output (mono duplication)

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
- [x] Oscillator and envelope state live directly on the plugin struct (stack/inline), not in a `Box`

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

### Deploy Script (`deploy.sh`)

```bash
#!/bin/bash
set -e
PLUGIN_NAME="sine_one"
BUNDLE="target/bundled/SineOne.clap"
CLAP_DIR="$HOME/Library/Audio/Plug-Ins/CLAP"

# Build
cargo xtask bundle "$PLUGIN_NAME" --release

# Validate CLAP compliance (fail fast)
clap-validator validate "$BUNDLE" --only-failed

# Install
cp -r "$BUNDLE" "$CLAP_DIR/"
echo "Installed to $CLAP_DIR/SineOne.clap"
echo "→ Rescan plugins in Bitwig: Preferences > Plug-ins > Rescan"
```

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
2. **Parameters visible** — Fine Tune, Attack, Release appear in the device panel with correct
   ranges and units
3. **Note produces sound** — draw a MIDI note; hear a sine tone
4. **AR envelope audible** — set Attack to 500ms; note should fade in; set Release to 1000ms;
   note should fade out after key release
5. **Fine tune works** — automate Fine Tune from -100 to +100 cents; pitch should sweep smoothly
6. **State save/load** — save project, close, reopen; parameters restore correctly
7. **No click on retrigger** — rapid MIDI notes should not produce audible clicks

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

1. **Retrigger from mid-release**: Currently, `note_on()` during release resets level to 0.0 and
   restarts attack. An alternative is to retrigger from the current release level (no dip). The
   zero-reset approach is simpler and slightly more audible as an attack. Revisit if legato
   behavior is desired.

2. **Velocity curve**: The current `velocity / 127.0` is linear (fully linear velocity response).
   A quadratic or logarithmic curve (`velocity^2 / 127^2`) often feels more natural on keyboards.
   Leave linear for now; easy to change in one place later.

3. **Phase initialization**: Oscillator phase starts at 0 on plugin load. On the very first note,
   the sine starts at 0 (silent) which is fine — the attack ramp covers any transient. On retrigger
   the phase is wherever it left off, which avoids phase discontinuity. This is the correct
   behavior; document here in case it looks like a bug.

4. **Mono vs. stereo output layout**: Both channels carry the same signal (duplication). If you
   later want to add pan or stereo effects (e.g., slight detuning between L/R), the I/O layout
   already supports it.
