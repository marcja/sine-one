# sine-one

A minimal monophonic sine-wave synthesizer CLAP plugin built with [nih-plug](https://github.com/robbert-vdh/nih-plug).

This is an intentionally simple first plugin. The goal is a complete, working build/test/deploy
workflow with every design decision documented — not a feature-rich instrument.

---

## What it does

- Accepts MIDI NoteOn / NoteOff
- Plays a sine oscillator tuned to the incoming note and velocity
- Shapes the output with a linear AR (Attack/Release) envelope
- Exposes three parameters: **Fine Tune** (±100 cents), **Attack** (1–5000 ms), **Release** (1–10000 ms)
- No filter, no polyphony, no GUI

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
├── deploy.sh               # build + validate + install in one step
├── README.md               # this file
├── xtask/                  # cargo xtask — handles .clap bundle creation
│   ├── Cargo.toml
│   └── src/main.rs
└── sine_one/               # the plugin crate
    ├── Cargo.toml
    └── src/
        ├── lib.rs          # nih_export_clap! macro entry point
        ├── plugin.rs       # SineOne struct + Plugin trait (initialize, reset, process)
        ├── params.rs       # SineOneParams — the three CLAP parameters
        ├── main.rs         # standalone binary entry point
        └── dsp/
            ├── mod.rs
            ├── oscillator.rs   # SineOscillator — phase accumulator + frequency math
            └── envelope.rs     # ArEnvelope — Idle/Attack/Release state machine
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
./deploy.sh
```

Then in Bitwig: **Preferences → Plug-ins → Rescan**.

### Gatekeeper (first install only)

macOS quarantines unsigned binaries. Clear it once after the first install:

```bash
xattr -d com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/SineOne.clap
```

### Standalone binary (no DAW needed)

```bash
cargo run --features standalone -- --output "Built-in Output"
```

Opens the plugin with CPAL system audio. Send MIDI from any source and hear output without
opening Bitwig. Useful for quick audio checks during development.

---

## Tests

Tests are organized in four layers:

| Layer | Command | What it covers |
|---|---|---|
| Unit | `cargo test` | `SineOscillator` and `ArEnvelope` in isolation; parameter range checks |
| Property-based | `cargo test` (included) | `proptest` — sine output always finite; envelope output always in [0.0, 1.0] |
| Integration | `cargo test` (included) | Plugin lifecycle: initialize → reset → process; silence before NoteOn |
| CLAP compliance | `clap-validator` (post-build) | Parameter round-trips, state save/load, fuzz pass |

Run `cargo test` freely — it's fast, requires no audio hardware, and covers the first three layers.
Run `clap-validator` after every bundle before loading in Bitwig.

### Benchmarks

```bash
cargo bench
```

Measures process-block throughput (samples/sec) via `criterion`. Sets a regression baseline on
first run; subsequent runs report statistical regressions. The target is processing a 512-sample
block well under the ~11.6ms deadline at 44100 Hz.

---

## Bitwig smoke tests

After installing and rescanning, verify the following manually:

1. Plugin appears in the Bitwig instrument browser under **SineOne**
2. **Fine Tune**, **Attack**, and **Release** appear in the device panel with correct ranges
3. A MIDI note produces a sine tone
4. Attack = 500ms: note fades in audibly over half a second
5. Release = 1000ms: note fades out after key release
6. Fine Tune automated from −100 to +100 cents: pitch sweeps smoothly, no zipper noise
7. Save project, close, reopen: all three parameters restore correctly
8. Rapid repeated MIDI notes produce no audible clicks

---

## Design reference

See [`sine_one/src/`](sine_one/src/) inline comments for implementation notes.
The full technical design document (DSP architecture, parameter rationale, state machine, open
questions) lives in `docs/design.md`.

---

## License

Personal use. CLAP-only build — no VST3, no GPLv3 bindings involved.
