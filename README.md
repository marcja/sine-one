# sine-one

A minimal polyphonic sine-wave synthesizer CLAP plugin built with [nih-plug](https://github.com/robbert-vdh/nih-plug).

This is an intentionally simple first plugin. The goal is a complete, working build/test/deploy
workflow with every design decision documented — not a feature-rich instrument.

---

## What it does

- Accepts MIDI NoteOn / NoteOff
- Plays a sine oscillator tuned to the incoming note and velocity
- Polyphonic: 1–8 voices with voice stealing (oldest releasing, then oldest active)
- Shapes the output with a linear AR (Attack/Release) envelope
- Responds to **PolyPan** note expression events for stereo panning (e.g., from Bitwig's Randomize device)
- Attack-gated start-phase transient: short attacks (< 10 ms) allow an intentional amplitude pop from non-zero start phase; long attacks suppress it
- Gain-compensated voice summing (1/√N) so perceived loudness stays constant as voice count changes
- Click-free retriggers: velocity and pan ramp over ~2 ms on voice reuse; phase continues rather than resetting
- No filter, no GUI

### Parameters

| Parameter | Range | Default | Notes |
|---|---|---|---|
| **Fine Tune** | ±100 cents | 0 ct | Smoothed at 20 ms for zipper-free automation |
| **Attack** | 1–5000 ms | 10 ms | Log-skewed; read at note-on boundaries |
| **Release** | 1–10000 ms | 300 ms | Log-skewed; read at note-off boundaries |
| **Start Phase** | 0–360° | 0° | Oscillator phase on NoteOn from silence |
| **Voices** | 1–8 | 1 | Polyphonic voice count; 1 = monophonic behavior |
| **Output Gain** | −24 to +12 dB | 0 dB | Final scaling after voice gain compensation |

---

## Prerequisites

Install the Rust toolchain if you haven't already:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-apple-darwin   # native M-series target
```

Install build and validation tools:

```bash
cargo install clap-validator   # CLAP compliance testing
cargo install cargo-watch      # optional: watch mode
```

---

## Project structure

```
sine-one/
├── Cargo.toml              # workspace manifest
├── Cargo.lock
├── bundler.toml            # tells xtask the bundle name ("SineOne")
├── README.md               # this file
├── docs/
│   ├── design.md           # full technical design document
│   └── references/         # technical reference material
├── xtask/                  # cargo xtask — handles .clap bundle creation + deploy
│   ├── Cargo.toml
│   └── src/main.rs
└── sine_one/               # the plugin crate
    ├── Cargo.toml
    ├── benches/
    │   └── dsp_bench.rs    # criterion benchmarks (component, voice, realtime)
    └── src/
        ├── lib.rs          # nih_export_clap! macro entry point
        ├── plugin.rs       # SineOne struct + Plugin trait (initialize, reset, process)
        ├── params.rs       # SineOneParams — six CLAP parameters
        ├── main.rs         # standalone binary entry point
        └── dsp/
            ├── mod.rs
            ├── oscillator.rs   # SineOscillator — phase accumulator + frequency math
            ├── envelope.rs     # ArEnvelope — Idle/Attack/Release state machine
            ├── voice.rs        # Voice — per-voice DSP path + voice allocation
            ├── pan.rs          # Constant-power stereo panning (sin/cos pan law)
            └── smoother.rs     # LinearSmoother — click-free parameter ramps
```

The split is intentional: `params.rs` is what the user controls, `dsp/` is the audio math,
`plugin.rs` is the nih-plug glue. You can read each concern in isolation.

---

## Development workflow

### Fast feedback loop

```bash
cargo check                   # type-check only (~seconds, no binary)
cargo clippy -- -D warnings   # lint; warnings are treated as errors
cargo test                    # run all unit + property + integration tests
```

None of these require audio hardware or a DAW. Run them freely.

### Build the CLAP bundle

```bash
cargo xtask bundle sine_one --release
# Output: target/bundled/SineOne.clap
```

### Validate CLAP compliance

```bash
clap-validator validate target/bundled/SineOne.clap --only-failed
```

This checks parameter round-trips, state save/load, threading invariants, and fuzzes the plugin
with 50 random parameter permutations. Zero failures = safe to load in Bitwig.

### Build + validate + install in one step

```bash
cargo xtask deploy
```

Then in Bitwig: **Preferences → Plug-ins → Rescan**.

### Gatekeeper (first install only)

macOS quarantines unsigned binaries. Clear it once after the first install:

```bash
xattr -d com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/SineOne.clap
```

### Standalone binary (no DAW needed)

```bash
cargo run -p sine_one --features standalone -- --output "Built-in Output"
```

Opens the plugin with CPAL system audio. Send MIDI from any source and hear output without
opening Bitwig. Useful for quick audio checks during development.

---

## Tests

Tests are organized in four layers:

| Layer | Command | What it covers |
|---|---|---|
| Unit | `cargo test` | DSP components in isolation (`SineOscillator`, `ArEnvelope`, `Voice`, `LinearSmoother`, `pan`); parameter range checks |
| Property-based | `cargo test` (included) | `proptest` — sine output always finite; envelope output always in [0.0, 1.0]; constant-power pan across range |
| Integration | `cargo test` (included) | Plugin lifecycle: initialize → reset → process; silence before NoteOn; polyphonic voice allocation and stealing; pan routing; gain compensation |
| CLAP compliance | `clap-validator` (post-build) | Parameter round-trips, state save/load, fuzz pass |

Run `cargo test` freely — it's fast, requires no audio hardware, and covers the first three layers.
Run `clap-validator` after every bundle before loading in Bitwig.

### Benchmarks

```bash
cargo bench
```

Criterion benchmarks are organized in three groups:

| Group | What it measures |
|---|---|
| **component** | Individual DSP units: oscillator, envelope, combined osc×env, `apply_detune` — all at 512 samples |
| **voice** | Single voice and 8-voice polyphonic rendering (512 samples) |
| **realtime** | 8-voice 512-sample buffer vs. the 11.6 ms hardware deadline at 44100 Hz; computes real-time ratio (median time / deadline × 100%) |

Sets a regression baseline on first run; subsequent runs report statistical regressions.
HTML reports are generated in `target/criterion/`.

---

## Bitwig smoke tests

After installing and rescanning, verify the following manually:

1. Plugin appears in the Bitwig instrument browser under **SineOne**
2. All six parameters appear in the device panel with correct ranges
3. A MIDI note produces a sine tone
4. Attack = 500 ms: note fades in audibly over half a second
5. Release = 1000 ms: note fades out after key release
6. Fine Tune automated from −100 to +100 cents: pitch sweeps smoothly, no zipper noise
7. Save project, close, reopen: all six parameters restore correctly
8. Rapid repeated MIDI notes produce no audible clicks
9. Randomize device → Pan: signal moves in the stereo field between notes
10. Voices = 4: play a chord — all four notes sound simultaneously
11. Voices = 1: play a chord — only the last note sounds (monophonic behavior)
12. Change Voices from 4 to 2 while holding a chord — excess voices release gracefully, no clicks
13. Start Phase = 90°, Attack = 1 ms: note begins with an intentional amplitude pop
14. Start Phase = 0°, Attack = 1 ms: note begins cleanly (no pop)
15. Output Gain = −12 dB: output is noticeably quieter than 0 dB

---

## Design reference

See [`sine_one/src/`](sine_one/src/) inline comments for implementation notes.
The full technical design document (DSP architecture, parameter rationale, state machine, open
questions) lives in `docs/design.md`.

---

## License

Personal use. CLAP-only build — no VST3, no GPLv3 bindings involved.
