# DSP strategies for emulating analog sine oscillator non-linearities

Real analog sine oscillators never produce a mathematically perfect sinusoid. Their output carries a fingerprint of harmonic distortion, amplitude breathing, frequency drift, and phase noise — artifacts that listeners consistently describe as "warmth," "life," or "analog character." This document provides the mathematical foundations, measurement data, and algorithmic strategies needed to convincingly reconstruct these imperfections in a digital signal processing context. The key insight from both published research and practitioner consensus is that **inter-oscillator detuning and slow frequency drift matter more perceptually than harmonic distortion**, yet all imperfections interact to create the full analog experience. A well-designed DSP emulation should prioritize per-voice pitch variation, apply subtle even-harmonic waveshaping with antiderivative anti-aliasing, and layer slow stochastic drift using an Ornstein-Uhlenbeck process — roughly in that order of priority.

---

## 1. The physics of analog sine oscillator imperfections

### Topology-dependent distortion mechanisms

Every analog sine oscillator topology introduces characteristic nonlinearities. **Wien bridge oscillators** (the HP 200A lineage) use an amplitude-stabilizing element in the negative feedback path. The original Hewlett design used a 3W incandescent lamp whose filament resistance varies roughly 10:1 from cold (~70 Ω) to hot (~700 Ω). Because the lamp's thermal mass prevents it from tracking individual audio cycles, it acts as a slow envelope-sensitive gain control. The symmetric thermal response produces predominantly **odd harmonics** (3rd, 5th, 7th). When the JFET replaced the lamp in later designs, the inherently asymmetric drain-source channel resistance introduced **even harmonics** (2nd, 4th), with 2nd harmonic dominance increasing as the voltage across the JFET's drain-source exceeds ~100 mV.

Robinson's distortion formula for Wien bridge AGC captures the key relationship: the 3rd harmonic amplitude relative to the fundamental is X₃/X₁ = 1/(8π · [(A₀−γ)/γ] · (1−γ) · ωT), where A₀ is the actual loop gain, γ ≈ 3 is the ideal Wien bridge gain, and T is the AGC time constant. This reveals a fundamental tradeoff: faster AGC (smaller T) reduces distortion but slows settling. The gain margin is razor-thin — loop gain even **0.8% below the required value of 3.000** causes oscillations to cease entirely, while gain set too high drives the amplifier into clipping at 2% THD or worse.

**Quadrature (two-integrator loop) oscillators** suffer from a different pathology: DC offset accumulation. Op-amp input offset voltages (typically 1–5 mV) are integrated by each stage, causing DC levels to creep toward the supply rails. Without explicit AGC, these oscillators rely on op-amp output saturation for amplitude limiting, producing significant distortion. Practical quadrature oscillators exhibit I/Q amplitude imbalance of 0.1–1 dB.

**State-variable VCOs** based on the CEM3340 chip (used in the Prophet-5, Memorymoog, and Elka Synthex) introduce integrator leakage and exponential converter thermal sensitivity. The thermal voltage Vt = kT/q ≈ 25.85 mV at 300 K has a temperature coefficient of **+3333 ppm/°C**, meaning a 10 °C change produces a 3.3% frequency error (~57 cents). The best reported compensation achieves 3 cents drift over a 40 °C range using a MAT14 matched transistor quad and precision op-amps.

### Measured harmonic distortion across designs

Published THD data varies enormously by topology and design care. The following table compiles measured values from the literature:

| Oscillator design | THD | Dominant harmonics |
|---|---|---|
| HP 200A (1939, lamp AGC) | ~0.5–1% | 3rd (odd) |
| Wien bridge, JFET AGC (typical) | 0.001–0.3% | 2nd + 3rd |
| Wien bridge, lamp (well-trimmed) | 0.003–0.01% | 3rd |
| Wien bridge, optocoupler (Jim Williams AN43) | <0.0003% (<3 ppm) | Below measurement floor |
| Wien bridge, diode-limited | ~1–5% | Odd, broad spectrum |
| ICL8038 VCO chip (precision trim) | ~0.5% | Mixed |
| AJH MiniMod VCO (Moog clone, output buffer) | 0.008% at 1 kHz | — |
| Expert SSI2164-based SVF oscillator | <0.0004% | — |

For individual harmonic levels in a well-designed JFET AGC Wien bridge: **2nd harmonic at −60 to −80 dB, 3rd harmonic at −70 to −90 dB** relative to the fundamental, with higher harmonics falling off rapidly. Lamp-based designs show 3rd harmonic at −50 to −70 dB. Symmetric nonlinearities (lamps, thermistors, back-to-back diodes) produce exclusively odd harmonics, while asymmetric elements (single JFET, mismatched components, op-amp input offset) generate even harmonics. In practice, both coexist simultaneously.

### Waveform asymmetry and its characterization

Waveform asymmetry — the deviation from odd symmetry — is quantified by duty cycle deviation from 50% and the ratio of even to odd harmonic content. A 1% duty cycle deviation (50.5%/49.5%) corresponds to a 2nd harmonic roughly **−40 dB** below the fundamental. The primary physical causes are op-amp input offset voltage (1–5 mV for bipolar types like NE5532, reducible to 10 µV with precision op-amps like OPA227), JFET drain-source channel asymmetry due to channel-length modulation, and mismatched components in the frequency-setting network. At higher frequencies, op-amp slew rate limiting creates triangular peaks: for a TL072 at 13 V/µs with 10 V peak, this onset begins above ~200 kHz.

### AGC dynamics shape the amplitude envelope

The AGC time constant defines the oscillator's amplitude behavior and is one of its most characterful properties. Lamp-based AGC has a thermal time constant of **200 ms to 1 second** for standard incandescent bulbs, creating a distinctive slow "breathing" when frequency is changed — the amplitude overshoots, clips, and gradually settles as the filament temperature equilibrates. At very low oscillation frequencies where the period approaches the lamp's thermal time constant, the resistance tracks the instantaneous signal amplitude, dramatically increasing within-cycle distortion.

JFET-based AGC operates with time constants set by the integrator RC network, typically **10–100 ms**. Fast settings (short time constants) cause AGC ripple at twice the oscillation frequency, injecting harmonics into the control voltage. The AGC integrator should produce "only a constant or slowly varying DC voltage because a variation in the range of the oscillator frequency appears as harmonic distortion." Slow settings yield cleaner waveforms but introduce amplitude settling times of 100 ms to several seconds after perturbation. Optocoupler/LDR AGC falls between these extremes at 1–100 ms response time.

Startup behavior follows an exponential envelope: signal grows measurably within 5 ms, reaching controlled amplitude within **10–50 ms for JFET AGC** and **200 ms to 2 seconds for lamp AGC**.

### Frequency drift: components and magnitudes

Frequency drift originates primarily from the temperature coefficients of frequency-setting components. Metal film resistors drift at ±50–100 ppm/°C, while capacitor tempco varies dramatically by type: C0G/NP0 ceramic at 0 ± 30 ppm/°C (best), polystyrene film at −120 ppm/°C, and X7R ceramic at an effective ~3000 ppm/°C. For a 10 kHz Wien bridge using metal film resistors and C0G capacitors, a 20 °C internal warm-up produces a total frequency change of approximately **±0.16%** (±16 Hz).

X7R ceramic capacitors are particularly treacherous: they can lose **50–80% of nominal capacitance** under DC bias approaching rated voltage (a 4.7 µF/16 V X7R in an 0603 package may read only 1–2 µF at 12 V bias). Additionally, ceramic Class II capacitors follow a logarithmic aging law: ΔC/C = −k × log₁₀(t/t₀), where k ≈ 1–2.5% per decade of time for X7R. After 1000 hours, total capacitance loss can reach 7.5%. This aging is reversible by heating above the Curie point (~125 °C), which resets the crystalline structure. C0G/NP0 capacitors show negligible aging (<0.1% over lifetime).

For VCO exponential converters, the transistor Vbe temperature coefficient of approximately −2 mV/°C translates to **~5.8 cents/°C** at the keyboard extremes. Combined with the 1/300 per °C change in thermal voltage, this is the dominant drift mechanism in analog synthesizers during warmup.

### Phase noise and component aging

Phase noise in analog oscillators follows Leeson's model: L(fm) = 10·log₁₀[(2FkT/Ps) · (f₀/(2Q_L·fm))² · (1 + fc/fm)], where Q_L is the loaded quality factor, fm is offset frequency, and fc is the flicker noise corner. RC oscillators like the Wien bridge have very low Q (~0.33), yielding typical phase noise of **−80 to −100 dBc/Hz at 1 kHz offset**. LC oscillators achieve −100 to −120 dBc/Hz thanks to Q values of 10–100+. Crystal oscillators, with Q of 10,000–200,000, reach −130 to −175 dBc/Hz.

The 1/f (flicker) noise corner frequency varies by device technology: ~1 kHz for BJTs, 0.1–1 MHz for MOSFETs. This creates a −30 dB/decade close-in phase noise slope characteristic of analog oscillator "smearing."

Component aging sets the long-term stability floor. Metal film resistors drift ~25–50 ppm/year; carbon composition resistors can shift 5–10% over decades. For an oscillator using metal film resistors and C0G capacitors, long-term frequency drift is ~0.005% per year. If X7R capacitors were used instead, logarithmic aging would cause ~4.9% frequency increase after one year — illustrating why C0G or film capacitors are mandatory in oscillator circuits.

---

## 2. Mathematical models for analog non-linearities

### Waveshaping transfer functions and their harmonic spectra

The three canonical sigmoid waveshaping functions — tanh, algebraic, and arctan — all share odd symmetry (producing only odd harmonics) but differ in harmonic onset, growth rate, and computational cost. Vicanek (2021) provided exact closed-form harmonic analyses for all three.

**Tanh saturation: f(x) = tanh(a·x).** For small drive (a < π/2), the harmonic amplitudes from Taylor expansion are a₁ = a − a³/4 + a⁵/12, a₃ = −a³/12 + a⁵/24, and a₅ = a⁵/120. For large drive (a >> 1), the output converges to a square wave with amplitudes a_{2n+1} = (−1)ⁿ · 4/((2n+1)π). The exact general-case series involves contour integration: a_{2n+1} = (−1)ⁿ · (4/a) · Σ_{k=0}^∞ 1/(s·(r+s)^{2n+1}), where r = (k+½)π/a and s = √(r²+1). Computational cost is ~5.9 ns/sample in scalar x86, reducible to ~0.55 ns/sample with a polynomial sinh-morphing approximation. Tanh is the **"hardest" of the three sigmoids** — latest harmonic onset but steepest growth — and its harmonic spectrum falls off most rapidly, making it the preferred choice for digital audio due to reduced aliasing risk.

**Algebraic sigmoid: f(x) = x/√(1+x²).** Taylor expansion: x − x³/2 + 3x⁵/8. Softer knee than tanh — harmonics onset at lower amplitudes but grow more slowly. Exact harmonic amplitudes involve the recursive coefficients β₀=1, β_{k+1} = −(k+½)/(k+1)·βₖ. Computational cost is ~0.45 ns/sample using fast reciprocal square root (Carmack's method or SIMD rsqrt) — **13× faster than tanh**.

**Arctan shaper: f(x) = (2/π)·arctan(πx/2).** The simplest analytically — exact closed-form harmonic amplitudes: c_{2n+1} = (−1)ⁿ · v^{n+½}/(n+½), where v = a²/(a² + 2 + 2√(a²+1)). This is the **softest shaper** — earliest harmonic onset, slowest growth — and converges to the same square-wave limit at extreme drive.

**Polynomial waveshapers** offer direct harmonic-order control. The cubic soft clipper f(x) = (3/2)x − (1/2)x³ generates exactly 1st + 3rd harmonics with a smooth derivative-zero transition at the clipping threshold. The power-to-harmonic mapping follows Pascal's triangle: cos²(θ) = ½ + ½cos(2θ), cos³(θ) = ¾cos(θ) + ¼cos(3θ), cos⁴(θ) = ⅜ + ½cos(2θ) + ⅛cos(4θ). A polynomial of degree N generates harmonics up to the Nth. Odd powers produce only odd harmonics; even powers produce even harmonics plus DC offset.

**Chebyshev polynomials** enable precise injection of individual harmonics. The defining property T_n(cos θ) = cos(nθ) means that when a unit-amplitude cosine is passed through T_n, the output is purely the nth harmonic. The first six are T₁(x) = x, T₂(x) = 2x²−1, T₃(x) = 4x³−3x, T₄(x) = 8x⁴−8x²+1, T₅(x) = 16x⁵−20x³+5x, T₆(x) = 32x⁶−48x⁴+18x²−1. To synthesize a target harmonic spectrum with amplitudes α₁, α₂, ..., αₙ, the waveshaping function is simply f(x) = Σ αₙ·Tₙ(x). This works perfectly only for sinusoidal input at unit amplitude; non-unit amplitude or non-sinusoidal input creates intermodulation products.

### Generating controlled even harmonics

Even harmonics require breaking the odd symmetry of the transfer function. The fundamental identity cos²(ωt) = (1 + cos(2ωt))/2 shows how a squared term maps a pure cosine directly to 2nd harmonic plus DC. For the general polynomial f(x) = a₁x + a₂x² + a₃x³, the coefficients provide independent control: **2nd harmonic level is proportional to a₂**, and **3rd harmonic level is proportional to a₃**, with cross-coupling only through the fundamental amplitude. DC offset equals a₂/2 and must be removed by a high-pass filter.

Practical asymmetric models include adding a squared sigmoid term: f(x) = tanh(x) + b·tanh²(x), where the parameter b controls even/odd harmonic ratio. Alternatively, adding a DC bias before symmetric waveshaping — f(x) = tanh(x + d) — shifts the operating point on the sigmoid curve, creating asymmetry proportional to d. For spectral matching to measured analog data, the Chebyshev approach constructs f(x) = h₁·T₁(x) + h₂·T₂(x) + h₃·T₃(x) + ..., where the hₙ coefficients are set to match measured harmonic levels (LeBrun, 1979).

### The Van der Pol oscillator as amplitude-limiting model

The Van der Pol equation x″ − μ(1−x²)x′ + x = 0 directly models the amplitude-limiting physics of analog oscillators. The nonlinear damping term μ(1−x²)x′ provides **negative resistance for |x| < 1** (amplification) and **positive resistance for |x| > 1** (limiting), perfectly capturing the self-regulating behavior of real circuits.

The parameter μ controls waveform character: at μ = 0, a pure harmonic oscillator; at 0 < μ << 1, a near-sinusoidal limit cycle with small harmonic distortion; at μ >> 1, relaxation oscillations approaching a square wave. Poincaré-Lindstedt perturbation analysis gives the waveform for small μ as x(t) = 2cos(ωt) + (μ²/96)[−3cos(ωt) + cos(3ωt)] + O(μ⁴), showing that the **3rd harmonic amplitude scales as μ²/96** — very small for well-regulated oscillators. The frequency shifts as ω = 1 − μ²/16 + 17μ⁴/3072, exhibiting the amplitude-frequency coupling that characterizes real oscillators.

The Krylov-Bogolyubov averaging method yields the amplitude dynamics explicitly: a(t) = 2/√(1 + (4/a₀² − 1)e^{−μt}), showing exponential convergence to the limit cycle amplitude of 2 from any initial condition. The convergence rate is proportional to μ, mapping directly to the AGC time constant in physical oscillators. Bersani et al. (J. Math. Phys., 2018) computed the Lindstedt-Poincaré series to order 859, establishing convergence for μ ≲ 3.42.

### AGC loop transfer function

The linearized AGC loop has open-loop transfer function G(s) = (K_I · K_osc · K_d) / (s · (1 + s·τ_det)), where K_I is integrator gain, K_osc is oscillator amplitude sensitivity, K_d is detector gain, and τ_det is the detector time constant. For lamp-based AGC, the thermal model is C_th · dT/dt = I²R_cold − (T−T_ambient)/R_thermal, with the thermal time constant τ = R_thermal × C_th giving the characteristic 200 ms–1 s response. For JFET AGC, the drain-source resistance follows R_DS = V_P²/(2·I_DSS·(V_P − V_GS)), providing voltage-controlled attenuation with the time constant set by the AGC integrator's RC network.

Amplitude pumping occurs when the AGC bandwidth is too high — the control loop modulates amplitude at ~2× the oscillation frequency, creating audible sidebands. The design target is τ_AGC >> T_oscillation while remaining fast enough to track desired amplitude changes.

### AM-to-FM conversion creates correlated modulation

In real oscillators, amplitude changes cause frequency shifts through voltage-dependent capacitance. The instantaneous frequency model is ω(t) = ω₀ + K_AM-FM · A(t), where K_AM-FM = −ω₀/(2C_eff) · dC_eff/dA (Hegazi & Abidi, IEEE JSSC 2003). This AM-FM coupling is a major contributor to close-in 1/f³ phase noise in VCOs. Power supply noise simultaneously modulates both amplitude and frequency, producing characteristic **asymmetric sidebands** from correlated AM and FM. For virtual analog synthesis, this coupling is modeled as φ(t) = ω₀·t + k·∫A(τ)dτ, where k is the coupling coefficient and A(t) comes from the AGC model.

---

## 3. DSP implementation strategies

### Antiderivative anti-aliasing is the critical technique for waveshaped sines

Waveshaping a sine wave creates harmonics that can fold back below the Nyquist frequency as aliasing artifacts. Unlike hard-edged waveforms (where polyBLEP or minBLEP correct discontinuities), waveshaped sines have smooth distortion with no discrete jumps — making **ADAA (antiderivative anti-aliasing)** the method of choice.

First-order ADAA, introduced by Parker, Zavalishin, and Le Bivic (DAFx-16) and formalized by Bilbao, Esqueda, Parker, and Välimäki (IEEE SPL, 2017), replaces the memoryless function y = f(x) with the integrated form:

**y[n] = (F₁(x[n]) − F₁(x[n−1])) / (x[n] − x[n−1])**

where F₁ is the antiderivative of f. When consecutive samples are nearly equal (|x[n] − x[n−1]| < ε ≈ 10⁻⁵), the fallback y[n] = f((x[n] + x[n−1])/2) avoids division by zero. For tanh waveshaping, F₁(x) = log(cosh(x)), computable as |x| + log(1 + e^{−2|x|}) − log(2) to avoid overflow. First-order ADAA provides **~30–40 dB aliasing reduction** at essentially zero computational overhead (one extra function evaluation per sample). Second-order ADAA uses the second antiderivative and provides ~50–60 dB reduction. Combining first-order ADAA with modest 2× oversampling often exceeds the quality of 8× oversampling alone.

polyBLEP, BLIT, and minBLEP are **not applicable** to waveshaped sine distortion — they correct discrete waveform discontinuities, which do not arise from smooth nonlinear waveshaping. The DAFx-21 conference introduced IIR anti-aliasing filters for nonlinear waveshaping that outperform both FIR ADAA and 8× oversampling for certain transfer functions.

### Four architectures for controlled harmonic distortion

**Direct waveshaping** (y = f(x)) is the simplest approach: apply a memoryless nonlinear function to the sine output. With ADAA, this provides tunable, low-aliasing distortion. The drive parameter controls harmonic intensity. Limitation: static character — the distortion spectrum doesn't vary dynamically.

**Additive harmonic injection** generates harmonics explicitly: y(t) = sin(ωt) + a₂sin(2ωt) + a₃sin(3ωt) + .... This is perfectly bandlimited (zero aliasing possible) and offers per-harmonic control, but is computationally expensive for many harmonics and produces a "too clean" static spectrum lacking the intermodulation and dynamic interaction of analog distortion. Best limited to ≤8 harmonics.

**Feedback nonlinearity** feeds the output through a nonlinear function back into the oscillator phase: phase[n] = ω·n + β·f(output[n−1]). This creates dynamically rich spectra that change with amplitude — closer to analog behavior — but introduces aliasing and potential instability in the feedback path. Applying ADAA to feedback paths is non-trivial (Holters, DAFx-19).

**State-variable oscillator with nonlinear integrators** is the most structurally faithful approach. The Chamberlin SVF oscillator (x₁[n+1] = x₁[n] − g·x₂[n], x₂[n+1] = x₂[n] + g·x₁[n+1]) provides simultaneous sine and cosine outputs. Replacing linear updates with x₁[n+1] = x₁[n] − g·tanh(k·x₂[n]) introduces amplitude-dependent harmonic distortion, natural amplitude limiting (replacing AGC), and amplitude-dependent frequency shift — all from a single structural modification. The tanh saturation parameter k controls "drive" (k = 1 is subtle; k = 3–5 produces heavy distortion). Zavalishin's trapezoidal (TPT) form with g = tan(π·f/fs) eliminates the frequency warping of the forward Euler form, providing exact frequency matching at any specified frequency.

### Drift modeling with stochastic processes

Realistic frequency drift requires combining multiple time scales. The **Ornstein-Uhlenbeck process** dx = θ(μ−x)dt + σdW provides a mean-reverting random walk ideal for bounded drift. The discrete-time implementation is:

**x[n+1] = μ + (x[n] − μ) · exp(−dt/τ) + σ · √(1 − exp(−2dt/τ)) · randn()**

where τ = 2–30 seconds (reversion time), μ = 0 (mean), σ = 0.5–5 cents (drift magnitude). This should be combined with faster 1/f noise for sub-second jitter.

For **1/f noise generation**, two practical approaches exist. Paul Kellet's pinking filter applies a 7-stage IIR cascade to white noise (coefficients: b0 feedback = 0.99886, b1 = 0.99332, b2 = 0.96900, b3 = 0.86650, b4 = 0.55000, b5 = −0.7616, b6 = 0.115926), accurate to ±0.5 dB from 10 Hz to Nyquist. The Voss-McCartney algorithm sums N sample-and-hold random sources updated at octave-related rates, selecting which source to update based on the trailing zero count of the sample counter.

A complete multi-rate drift model combines:

- Slow thermal drift: O-U process with τ ≈ 10 s, σ ≈ 3–8 cents
- Fast component noise: 1/f-filtered pink noise scaled to ~0.3–1 cent, low-pass filtered to ~50 Hz
- Total frequency: f_base × 2^(total_drift/1200)

Each oscillator voice should have **independent** drift instances for realistic inter-voice detuning.

### Phase noise injection and AGC envelope modeling

Phase noise is injected by adding filtered noise to the phase accumulator: phase[n] = phase[n−1] + 2πf/fs + noise[n]. The noise spectral shape determines the character: white phase noise creates a flat noise floor; 1/f phase noise creates the −30 dB/decade close-in spectral skirt characteristic of analog oscillators; 1/f² (random walk) phase noise creates −40 dB/decade Brownian drift. Practical levels should target **0.001–0.01 radians RMS** — higher levels (>0.05 radians) sound overtly FM-modulated rather than subtly analog.

AGC dynamics are modeled as a sidechain envelope follower. The standard one-pole implementation uses attack_coeff = exp(−1/(τ_attack · fs)) and release_coeff = exp(−1/(τ_release · fs)). For lamp-like behavior, use attack ~500 ms, release ~1–2 s. For JFET-like behavior, attack ~10–50 ms, release ~50–200 ms. The gain correction should be **partial** — applying (target/envelope)^0.3 rather than full correction (exponent = 1) — so the AGC never fully compensates, leaving residual amplitude variation that sounds organic.

### Lookup tables versus direct computation

Modern CPUs have shifted the balance toward direct computation. A 1024-point LUT with linear interpolation yields ~−80 dB error for smooth functions like tanh (4 KB memory), while cubic Hermite interpolation improves to ~−120 dB (at 4 multiplies + 3 adds per lookup). However, LUT access is inherently serial, defeating SIMD parallelism. The Padé approximant tanh(x) ≈ x(27+x²)/(27+9x²) is accurate to ~0.1% for |x| < 3 and maps naturally to SIMD, processing 4–8 samples in ~10 cycles with SSE/AVX. The algebraic sigmoid x/√(1+x²) is even faster at ~0.45 ns/sample using hardware rsqrt. For ADAA, compute F₁(x) = log(cosh(x)) directly but consider LUTs for the second antiderivative F₂ involving the dilogarithm.

---

## 4. Perceptual considerations and implementation priorities

### Which imperfections define "analog character"

Converging evidence from blind listening tests, synthesizer designer interviews, DAFx research, and practitioner forums reveals a clear perceptual hierarchy. **Voice-to-voice detuning is the single most-cited characteristic** of analog sound. Classic polysynths like the Prophet-5 and OB-Xa had inherent per-voice pitch variation that created a "thick," "alive," "3D" quality. The sweet spot is **~5–15 cents of detune** — enough for perceptible thickness without hearing separate pitches. GForce Software's analysis notes this creates a region where "two tones are perceived as one but thicker."

**Filter nonlinearity** is consistently identified as the primary timbral differentiator between analog and digital — the signal-level-dependent behavior where cutoff frequency, resonance width, and distortion all change with input amplitude. Even harmonics (2nd, 4th, 6th) are perceived as "warm" and "musical," while odd harmonics (3rd, 5th, 7th) are heard as "harsh" and "edgy." Most analog VCFs produce a mixture emphasizing low-order, predominantly even harmonics at moderate levels.

**Slow frequency drift** contributes a "living" quality but ranks below detuning in importance. Well-designed modern VCOs drift less than 0.1 Hz over 24 hours after warmup; vintage VCOs showed noticeable drift in the first 15–30 minutes. The pitch just-noticeable difference for sequential tones is **~3–6 cents** in the 500–2000 Hz range for trained listeners, so drift below this threshold operates subliminally. The perceptual sweet spot for analog-character drift is ~5–15 cents of slow random wander at rates of 0.01–0.1 Hz.

**Noise floor** (typically −60 to −80 dB in analog synths) adds broadband "presence" most audible on quiet sustained tones, but is almost entirely masked in a mix. **Amplitude instability** detection threshold is ~2% modulation depth at slow rates (1–4 Hz) per Zwicker; most analog VCA instabilities fall below this, making it a subliminal contributor. **Phase noise** is the least perceptually significant — its effect is subsumed by the much larger frequency drift and detuning.

### Psychoacoustic thresholds for key parameters

Harmonic distortion becomes audible on a pure sine at approximately **0.1–1% THD** depending on frequency and harmonic order. The 7th harmonic is clearly audible at 1%, almost inaudible at 0.1%, and below threshold at 0.05%. Higher-order harmonics are more audible at the same level than lower-order. Even harmonics are harder to detect than odd at equivalent levels. In the context of musical signals with masking, the threshold rises to 1–5% for program material.

Amplitude modulation detection follows a U-shaped function of rate: the ear is most sensitive at 1–4 Hz (threshold ~2% depth, ~0.17 dB), less sensitive at 10 Hz (~5%, ~0.4 dB), and least sensitive above 50 Hz. For simultaneous tones, beating from detuning is detectable at **less than 1 cent**; perceptual "thickness" begins at ~3–7 cents.

### The virtual analog "uncanny valley"

When emulation is close but not quite right, experienced listeners report it can sound worse than either pure digital or real analog. This manifests most at extreme settings — heavy filter resonance, oscillator sync sweeps, and high-feedback patches. The solution identified by DAFx researchers is true circuit-level modeling rather than parameter matching: "A digital filter can be programmed to mimic non-linearity, but that mimicking will itself be fixed and linear and therefore, ultimately, unconvincing." Static imperfections (noise, drift) are much easier to model convincingly than **signal-dependent dynamic behavior** (how nonlinearities change with playing level and speed).

Modern blind tests consistently show that listeners struggle to distinguish well-implemented VA from analog. Välimäki and Huovilainen (Computer Music Journal, 2006) demonstrated that 4th-order DPW methods are "perceptually alias-free" across the piano register, and forum ABX tests with matched parameters typically split roughly 50/50. The remaining differences emerge primarily in filter behavior at extreme resonance settings.

---

## 5. Reference data, saturation model comparisons, and key literature

### Published saturation model benchmarks

Enderby and Baracskai (DAFx-12) provided the most direct comparison of soft clipping algorithms for a 2 kHz sine at 96 kHz sampling, measuring both THD and harmonic stability:

| Algorithm | THD (dB) | Harmonic stability |
|---|---|---|
| Two-stage quadratic | −13.32 | Low (unstable) |
| Cubic | −13.10 | Low |
| Tanh | **−9.99** | **High (stable)** |
| Exponential (E=5) | −9.42 | Highest |
| Reciprocal | −9.01 | Medium |

Tanh produces the most distortion among smooth algorithms but with excellent dynamic stability — harmonics maintain consistent ratios as input level varies. Cubic and polynomial models produce less total distortion but exhibit high harmonic instability (harmonic levels shift erratically with input level), which can be perceptually objectionable. All symmetric algorithms produce only odd harmonics.

**Tube/valve models** differ fundamentally from sigmoid-based models. The Koren triode model (the standard in SPICE simulation) uses Ip = (E1^Ex / Kg1) where E1 = Vp/Kp × log(1 + exp(Kp × (1/µ + Vg/Vp))). Tube saturation is **inherently asymmetric** — clipping differently on positive and negative half-cycles — producing both even and odd harmonics naturally without requiring an explicit asymmetry parameter. The transfer function involves the 3/2 power law rather than exponential saturation, and the parameter Ex (typically 1.2–1.5) controls the characteristic's shape. Diode clipper models follow the exponential I-V characteristic I = Is(e^{V/nVt} − 1), solved in real-time using the Lambert W function (D'Angelo and Välimäki, 2012).

### Essential academic references

The foundational paper chain for anti-aliased virtual analog oscillators runs from **Stilson and Smith** ("Alias-Free Digital Synthesis of Classic Analog Waveforms," ICMC 1996) through **Välimäki and Huovilainen** ("Antialiasing Oscillators in Subtractive Synthesis," IEEE SPM 2007) to the ADAA lineage: **Parker, Zavalishin, Le Bivic** (DAFx-16), **Bilbao, Esqueda, Parker, Välimäki** (IEEE SPL 2017), and **Holters** (DAFx-19 / Applied Sciences 2020). Esqueda, Pöntynen, Välimäki, and Parker's "Virtual Analog Buchla 259 Wavefolder" (DAFx-17) provides detailed circuit-level analysis with SPICE validation. D'Angelo's Ph.D. thesis at Aalto University (2014) covers wave digital filter approaches to nonlinear circuit modeling including the Moog ladder filter and triode models.

Key books include **Zavalishin**, "The Art of VA Filter Design" (Native Instruments, rev. 2.1.2, 2020, freely available), which covers TPT/zero-delay feedback methods for SVF oscillators and filters; **Julius O. Smith III**'s freely available CCRMA books (Physical Audio Signal Processing, Spectral Audio Signal Processing); **Zölzer**, "DAFX: Digital Audio Effects" (Wiley, 2nd ed., 2011), Chapter 12 on virtual analog effects; and **Pirkle**, "Designing Audio Effect Plugins in C++" for practical implementation. Vicanek's "Waveshaper Harmonics" (2021) provides the definitive harmonic analysis of sigmoid waveshapers, while Yeh's Stanford Ph.D. thesis (2009) covers guitar amplifier distortion circuit modeling comprehensively.

**Cytomic (Andrew Simper)** published the foundational SVF implementation paper (SvfLinearTrapOptimised2) now used in Ableton EQ8, Xfer Serum, and numerous other products. **Jatin Chowdhury**'s blog and the chowdsp_wdf C++ library provide practical ADAA tutorials and wave digital filter implementations. The **Electric Druid** site offers detailed analysis of CEM3340 implementations across famous synthesizers.

---

## Conclusion: a practical synthesis architecture

The evidence points toward a layered DSP architecture. At its core, a **state-variable oscillator with tanh-saturating integrators** provides the most structurally faithful analog emulation — simultaneously generating amplitude-dependent harmonic distortion, natural amplitude limiting, and amplitude-frequency coupling from a single computational structure. Apply **first-order ADAA** (using the log(cosh) antiderivative) combined with 2× oversampling for clean, alias-free waveshaping. Layer per-voice **Ornstein-Uhlenbeck drift** (τ ≈ 10 s, σ ≈ 5–8 cents) with 1/f-filtered fast jitter (~0.5 cents) for realistic frequency instability. Add static per-voice pitch offsets of ±5–15 cents for the critical detuning character. Model AGC breathing with a slow envelope follower (attack 500 ms, release 1.5 s) applying partial correction. Inject a small noise floor at −65 dB and phase noise at ~0.005 radians RMS with 1/f spectral shaping. Use **Chebyshev polynomial waveshaping** to fine-tune the harmonic spectrum to match specific measured analog profiles — targeting 2nd harmonic at −35 to −45 dB and 3rd harmonic at −45 to −55 dB for typical analog character. The most impactful improvement per CPU cycle invested is voice-to-voice detuning; the least impactful is phase noise modeling. Novel approaches using neural ODEs and differentiable signal processing (emerging from DAFx-25 research) may eventually learn the full nonlinear dynamics directly from recorded hardware, but the white-box models described here remain the practical engineering standard.