use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sine_one::dsp::envelope::ArEnvelope;
use sine_one::dsp::oscillator::SineOscillator;

/// Benchmark: oscillator generating 512 samples (one typical buffer).
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

/// Benchmark: envelope attack phase over 512 samples.
fn envelope_attack_512_samples(c: &mut Criterion) {
    c.bench_function("envelope_attack_512_samples", |b| {
        b.iter(|| {
            let mut env = ArEnvelope::default();
            env.set_attack(10.0, 44100.0);
            env.set_release(300.0, 44100.0);
            env.note_on();
            for _ in 0..512 {
                black_box(env.next_sample());
            }
        });
    });
}

/// Benchmark: the combined DSP inner loop — oscillator × envelope × velocity
/// for 512 samples. This mirrors the per-sample work in `process()`.
fn combined_dsp_512_samples(c: &mut Criterion) {
    c.bench_function("combined_dsp_512_samples", |b| {
        let mut osc = SineOscillator::default();
        osc.set_frequency(440.0, 44100.0);
        let mut env = ArEnvelope::default();
        env.set_attack(10.0, 44100.0);
        env.set_release(300.0, 44100.0);
        env.note_on();
        let velocity: f32 = 100.0 / 127.0;

        b.iter(|| {
            for _ in 0..512 {
                let osc_sample = osc.next_sample();
                let env_sample = env.next_sample();
                black_box(osc_sample * env_sample * velocity);
            }
        });
    });
}

criterion_group!(
    benches,
    oscillator_512_samples,
    envelope_attack_512_samples,
    combined_dsp_512_samples,
);
criterion_main!(benches);
