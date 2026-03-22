# CLAUDE.md — sine-one

Instructions for Claude Code working on this project. Read this file, `README.md`, and
`docs/design.md` before starting any work session.

---

## Project orientation

`sine-one` is a first-ever nih-plug CLAP plugin. The primary goal is **pedagogical**: every
decision should be explainable, every module should be small and readable, and the codebase should
teach its author how nih-plug works. Prefer clarity over cleverness at every tradeoff.

The full technical design (DSP architecture, parameter rationale, state machine, open questions)
is in `docs/design.md`. Read it before touching DSP or parameter code.

---

## Workflow: TDD loop

Every unit of work follows this exact sequence. Do not skip or reorder steps.

```
1. WRITE A FAILING TEST
   Write the test first. Run `cargo test` and confirm it fails (red).
   If the test passes before any implementation exists, the test is wrong — fix it.

2. WRITE THE MINIMUM CODE TO PASS
   Implement only what is needed to make the failing test green.
   Do not implement anything not yet covered by a test.

3. RUN ALL CHECKS
   cargo fmt
   cargo clippy -- -D warnings
   cargo test
   All three must be clean. Fix any issues before proceeding.

4. RESOLVE REVIEWS
   Run any applicable review commands (e.g., /simplify, /review).
   Apply all suggestions or document explicitly why a suggestion was declined (as a comment).
   Re-run checks after applying changes.

5. COMMIT
   See commit format below.
   Each commit must be small, concrete, and — whenever possible — deliver something
   externally visible (a test that proves a behavior, a parameter the host can see,
   audio output that can be heard in standalone mode).

6. ANALYZE AND PROPOSE CLAUDE.md CHANGES
   After committing, review the conversation that led to this commit.
   If any corrections, clarifications, or better patterns emerged, propose a concrete
   diff to this file (CLAUDE.md) and ask the user to confirm before applying.
```

---

## Commit message format

Every commit message follows this structure:

```
[scope] verb: short description (≤72 chars total)

Layer: <DSP | Params | Plugin | Tests | Build | Docs | Config>
Tests: <unit | property | integration | none> — <comma-separated test names, or "n/a">
Why: One sentence. What problem does this commit solve, or what does it teach?
Next: One sentence. What is the logical next commit after this one?
```

### Scope tokens

| Scope | Use for |
|---|---|
| `dsp/osc` | `src/dsp/oscillator.rs` |
| `dsp/env` | `src/dsp/envelope.rs` |
| `params` | `src/params.rs` |
| `plugin` | `src/plugin.rs` (Plugin trait impl) |
| `lib` | `src/lib.rs` (exports, top-level wiring) |
| `build` | `Cargo.toml`, `xtask/`, `bundler.toml` |
| `tests` | `tests/` integration test files |
| `bench` | `benches/` |
| `docs` | `README.md`, `docs/`, `CLAUDE.md` |

### Verb vocabulary

Use exactly one of: `add`, `implement`, `fix`, `refactor`, `test`, `remove`, `document`, `configure`.

### Examples

```
[dsp/osc] add: SineOscillator with phase accumulator and frequency setter

Layer: DSP
Tests: unit — sine_output_in_range, sine_phase_is_periodic, reset_clears_phase
Why: Oscillator is the first leaf DSP struct; no dependencies on params or plugin glue.
Next: [dsp/env] add ArEnvelope state machine — oscillator is its only prerequisite.
```

```
[dsp/env] add: ArEnvelope — Idle/Attack/Release state machine

Layer: DSP
Tests: unit — idle_outputs_zero, attack_ramps_up, attack_reaches_one, release_ramps_down, retrigger_resets_level
Why: Envelope shapes the oscillator output; required before plugin.rs can produce shaped audio.
Next: [params] add SineOneParams — three FloatParams (fine_tune, attack, release).
```

```
[plugin] implement: process() — NoteOn/NoteOff routing and per-sample audio output

Layer: Plugin
Tests: integration — note_on_produces_nonzero_output, silence_before_note_on
Why: First commit where the plugin produces audible output; validates the full signal path.
Next: [build] configure xtask bundle and deploy.sh so the plugin can be loaded in Bitwig.
```

### Why this format?

- **Scope** tells you where to look in the file tree immediately.
- **Layer** gives Claude Code a quick map of what's been built and what's still missing.
- **Tests** makes the test suite self-documenting in the git log.
- **Why/Next** create an explicit chain of reasoning across commits — useful when resuming a
  session and the conversation context has been lost.

---

## Commit granularity rules

Prefer commits that are **one thing**. Use these as guides:

- One DSP struct per commit (`SineOscillator` is one commit, `ArEnvelope` is another)
- One layer per commit (DSP and Plugin trait wiring should not be in the same commit)
- Tests for a struct ship **in the same commit** as the struct — never ahead, never behind
- Build/config changes (Cargo.toml, bundler.toml, xtask/) are their own commit
- `CLAUDE.md` changes are always their own separate commit with scope `[docs]`

**Do not batch.** A commit that says "add oscillator, envelope, and params" makes the git log
useless for learning and makes rollback difficult.

---

## Codetags

Use codetags as inline comments when an implementation is intentionally incomplete, approximate,
or requires revisiting. Always include a reason.

| Tag | Meaning |
|---|---|
| `TODO` | Known missing behavior; should be implemented in a subsequent commit |
| `FIXME` | Known bug or incorrect behavior being deferred |
| `HACK` | Working but fragile, non-obvious, or non-idiomatic; should be cleaned up |
| `NOTE` | Pedagogical explanation for the author; not a defect |
| `REVIEW` | A design decision that should be revisited once the plugin is audible |

Format:

```rust
// TODO(retrigger): currently resets level to 0.0 on NoteOn mid-release;
//   consider retriggering from current level for smoother legato behavior.
//   See docs/design.md "Open Questions #1".

// HACK(velocity): linear velocity scaling; feels natural for mouse input but
//   quadratic (velocity^2 / 127^2) is more expressive on a real keyboard.
//   See docs/design.md "Open Questions #2".

// NOTE: sine oscillators are inherently band-limited, so no PolyBLEP or
//   oversampling is needed here. Contrast with saw/square oscillators.
```

Codetags are searchable: `grep -r "TODO\|FIXME\|HACK\|REVIEW" sine_one/src/`

---

## Code style

- **Explicit over implicit**: name variables for what they represent (`attack_samples`, not `n`).
- **Comment the DSP math**: when a formula appears (e.g., `2.0_f32.powf(cents / 1200.0)`),
  add a comment that states what it computes in plain English.
- **No magic numbers**: all numeric constants should be named (`const TWO_PI: f32 = ...`) or
  accompanied by a comment explaining their origin.
- **Keep functions short**: if a function body exceeds ~20 lines, consider splitting it.
- **`#[allow(...)]` is forbidden** without a comment explaining why the lint is wrong for this case.

---

## nih-plug conventions to follow

These are rules specific to nih-plug that are easy to get wrong:

- DSP state (oscillator phase, envelope level) lives on the **plugin struct**, NOT in `Params`.
  `Params` holds only what the user/host controls.
- `#[id = "stable-string"]` on every `FloatParam` — this string is persisted in DAW sessions and
  must never change once any real session has been saved.
- Call `util::midi_note_to_freq(note)` from nih-plug; do not reimplement MIDI-to-Hz conversion.
- `initialize()` is where sample-rate-dependent values are computed (e.g., converting ms to
  samples). Do not do this in `Default::default()`.
- `reset()` must zero all DSP state: oscillator phase, envelope level, envelope state, current
  note, current velocity.
- `reset()` must also re-initialize any derived state that `initialize()` computes (e.g., gain
  compensation). The host calls `reset()` after `initialize()`, so anything set in `initialize()`
  but not re-set in `reset()` will be lost.
- nih-plug's built-in `Smoother` on `FloatParam` is not initialized in the test harness (always
  returns 0.0 from `.smoothed.next()`). Use `.value()` for params where testability matters and
  smoothing is not critical. The `fine_tune` param works with `.smoothed.next()` only because its
  default (0.0 cents) produces no audible effect.
- Never allocate in `process()`. The `assert_process_allocs` Cargo feature will abort in debug
  builds if you do.

---

## Quality gates (must all pass before any commit)

```bash
cargo fmt           # formatting — no diff allowed
cargo clippy -- -D warnings   # zero warnings
cargo test          # all tests green
```

After bundle builds, additionally:

```bash
clap-validator validate target/bundled/SineOne.clap --only-failed   # zero failures
```

---

## Suggested build order

Follow this sequence. Each step is a candidate commit boundary.

```
1. [build]   Cargo workspace scaffold (Cargo.toml, xtask/, sine_one/Cargo.toml, bundler.toml)
2. [lib]     Stub lib.rs + plugin.rs that compiles (Plugin trait with empty impls)
3. [dsp/osc] SineOscillator struct + unit tests
4. [dsp/env] ArEnvelope struct + unit tests (proptest included)
5. [params]  SineOneParams — three FloatParams + param range unit tests
6. [plugin]  initialize() and reset() — wire sample rate into DSP structs
7. [plugin]  process() — NoteOn/NoteOff event handling + per-sample output
8. [tests]   Lifecycle integration tests (silence before NoteOn, note_on_produces_output)
9. [build]   cargo xtask deploy + standalone binary feature
10. [bench]  criterion benchmark for process() block
11. [docs]   README and design.md updates reflecting any implementation deltas
```

This order ensures every commit builds on a green test suite and no step requires two things
to exist simultaneously before either is testable.

---

## Post-commit CLAUDE.md review

After every commit, before ending the session:

1. Re-read the conversation since the last commit.
2. Check whether any of the following occurred:
   - A correction was made to code Claude Code produced
   - A convention was established that isn't yet in this file
   - A nih-plug behavior was discovered that differs from what's documented here
   - A workflow step was added, skipped, or modified by the user
3. If yes to any of the above: draft a specific proposed change to this file (CLAUDE.md) using a
   `diff`-style or before/after block. Present it to the user and wait for confirmation before
   applying. Do not self-apply CLAUDE.md edits.
4. If no changes are warranted, say so briefly ("No CLAUDE.md updates needed after this commit.")

---

## Reading the git log

```bash
git log --oneline          # scan scope + verb + description
git log                    # full messages with Layer / Tests / Why / Next
git log --grep="dsp/"      # filter to DSP commits only
git log --grep="TODO"      # find commits that introduced deferred work
grep -r "TODO\|FIXME\|HACK\|REVIEW" sine_one/src/   # find all open codetags
```
