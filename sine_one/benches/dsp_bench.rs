use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use nih_plug::util;
use sine_one::dsp::envelope::ArEnvelope;
use sine_one::dsp::oscillator::{apply_detune, SineOscillator};
use sine_one::dsp::smoother::LinearSmoother;
use sine_one::dsp::voice::{NoteOnParams, Voice, MAX_VOICES};
use sine_one::dsp::wavefold::wavefold;

const SAMPLE_RATE: f32 = 44100.0;
const BUFFER_SIZE: usize = 512;
const FINE_TUNE_CENTS: f32 = 5.0;

// Create one voice at the given note/pan, warmed up to steady state.
fn make_voice(note: u8, pan: f32, age: u64) -> Voice {
    let mut voice = Voice::default();
    voice.note_on(NoteOnParams {
        note,
        velocity: 0.8,
        base_freq: util::midi_note_to_freq(note),
        start_phase_normalized: 0.0,
        sample_rate: SAMPLE_RATE,
        attack_ms: 10.0,
        age,
    });
    voice.set_pan(pan, SAMPLE_RATE);
    // Warm up past pan ramp (~88 samples) and attack phase (441 samples).
    for _ in 0..BUFFER_SIZE {
        voice.render_sample(0.0, 0.0, SAMPLE_RATE);
    }
    voice
}

// Create 8 active voices on a C-major spread with varied pan positions.
// Each is warmed up so smoothers have settled and envelopes are at steady state.
fn make_active_voices() -> [Voice; MAX_VOICES] {
    let notes: [u8; MAX_VOICES] = [60, 64, 67, 72, 76, 79, 84, 88];
    let pans: [f32; MAX_VOICES] = [-0.75, -0.25, 0.25, 0.75, -0.5, 0.0, 0.5, 1.0];
    core::array::from_fn(|i| make_voice(notes[i], pans[i], (i + 1) as u64))
}

// Render one 512-sample buffer of polyphonic output — mirrors the per-sample
// hot path in `process()`. Shared by `voice/8_voices_512` and
// `realtime/8v_512_deadline` to keep the measured workload in sync.
fn render_buffer(voices: &mut [Voice], gain_smoother: &mut LinearSmoother) {
    for _ in 0..BUFFER_SIZE {
        let gain = gain_smoother.next_sample();
        let mut left_sum = 0.0_f32;
        let mut right_sum = 0.0_f32;
        for voice in voices.iter_mut() {
            let (l, r) = voice.render_sample(FINE_TUNE_CENTS, 0.0, SAMPLE_RATE);
            left_sum += l;
            right_sum += r;
        }
        left_sum *= gain;
        right_sum *= gain;
        black_box((left_sum, right_sum));
    }
}

// Gain compensation smoother at 1 / sqrt(MAX_VOICES).
fn make_gain_smoother() -> LinearSmoother {
    let mut smoother = LinearSmoother::default();
    smoother.set_immediate(1.0 / (MAX_VOICES as f32).sqrt());
    smoother
}

// ---------------------------------------------------------------------------
// Group 1: component
// ---------------------------------------------------------------------------

fn component_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("component");
    group.throughput(Throughput::Elements(BUFFER_SIZE as u64));

    group.bench_function("oscillator_512", |b| {
        let mut osc = SineOscillator::default();
        osc.set_frequency(440.0, SAMPLE_RATE);
        b.iter(|| {
            for _ in 0..BUFFER_SIZE {
                black_box(osc.next_sample());
            }
        });
    });

    group.bench_function("envelope_attack_512", |b| {
        b.iter(|| {
            let mut env = ArEnvelope::default();
            env.set_attack(10.0, SAMPLE_RATE);
            env.set_release(300.0, SAMPLE_RATE);
            env.note_on();
            for _ in 0..BUFFER_SIZE {
                black_box(env.next_sample());
            }
        });
    });

    group.bench_function("combined_dsp_512", |b| {
        b.iter(|| {
            // Recreate per iteration so the envelope stays in attack phase.
            let mut osc = SineOscillator::default();
            osc.set_frequency(440.0, SAMPLE_RATE);
            let mut env = ArEnvelope::default();
            env.set_attack(10.0, SAMPLE_RATE);
            env.set_release(300.0, SAMPLE_RATE);
            env.note_on();
            let velocity: f32 = 100.0 / 127.0;

            for _ in 0..BUFFER_SIZE {
                let osc_sample = osc.next_sample();
                let env_sample = env.next_sample();
                black_box(osc_sample * env_sample * velocity);
            }
        });
    });

    group.bench_function("wavefold_512", |b| {
        let mut osc = SineOscillator::default();
        osc.set_frequency(440.0, SAMPLE_RATE);
        b.iter(|| {
            for _ in 0..BUFFER_SIZE {
                let sample = osc.next_sample();
                // black_box fold_amount to prevent constant-folding the sin().
                black_box(wavefold(sample, black_box(0.5)));
            }
        });
    });

    group.bench_function("apply_detune_512", |b| {
        b.iter(|| {
            for _ in 0..BUFFER_SIZE {
                // black_box the inputs to prevent constant-folding the powf.
                black_box(apply_detune(
                    black_box(440.0_f32),
                    black_box(FINE_TUNE_CENTS),
                ));
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: voice
// ---------------------------------------------------------------------------

fn voice_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("voice");
    group.throughput(Throughput::Elements(BUFFER_SIZE as u64));

    group.bench_function("single_voice_512", |b| {
        let mut voice = make_voice(60, 0.0, 1);
        b.iter(|| {
            for _ in 0..BUFFER_SIZE {
                black_box(voice.render_sample(FINE_TUNE_CENTS, 0.0, SAMPLE_RATE));
            }
        });
    });

    group.bench_function("8_voices_512", |b| {
        let mut voices = make_active_voices();
        let mut gain_smoother = make_gain_smoother();
        b.iter(|| render_buffer(&mut voices, &mut gain_smoother));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: realtime — Karpathy Loop scalar metric
//
// Measures wall-clock time to render one 512-sample buffer with 8 active
// voices. The audio deadline is 512 / 44100 = 11,609,977 ns. An external
// agent computes: real_time_ratio = median_ns / 11_609_977 × 100%.
// ---------------------------------------------------------------------------

fn realtime_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("realtime");
    group.throughput(Throughput::Elements(BUFFER_SIZE as u64));
    group.warm_up_time(std::time::Duration::from_millis(500));
    group.measurement_time(std::time::Duration::from_secs(3));

    group.bench_function("8v_512_deadline", |b| {
        let mut voices = make_active_voices();
        let mut gain_smoother = make_gain_smoother();
        b.iter(|| render_buffer(&mut voices, &mut gain_smoother));
    });

    group.finish();
}

criterion_group!(component_benches, component_benchmarks,);
criterion_group!(voice_benches, voice_benchmarks,);
criterion_group!(realtime_benches, realtime_benchmarks,);
criterion_main!(component_benches, voice_benches, realtime_benches);
