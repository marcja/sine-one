# DSP Strategies for Vactrol and Lowpass Gate Emulation

The most effective approach to emulating vactrols in software combines a physics-informed asymmetric slew model for the envelope, a power-law resistance mapping, and a topology-preserving discretization of the Buchla 292 filter circuit. **The definitive published reference is Parker & D'Angelo's DAFx-13 paper**, which provides a complete, real-time-capable model of the Buchla 292 LPG including vactrol dynamics, control circuitry, and audio path — validated against hardware. This report distills that work alongside newer physical models (Najnudel et al., DAFx-23), behavioral approaches (Mutable Instruments Streams), and practical implementation data into a three-layer reference suitable for direct implementation at 44.1–96 kHz.

---

## LAYER 1 — The Vactrol Envelope Model

### How vactrols work at the physics level

A vactrol pairs an LED with a cadmium sulfide (CdS) photoresistor inside a light-tight package. CdS is a II-VI semiconductor with a **bandgap of ~2.4 eV** (peak spectral sensitivity at **515–550 nm**). When photons strike the CdS crystal, they excite electrons from the valence band into the conduction band, creating electron-hole pairs that increase conductivity. The device exhibits enormous photoconductive gain — approximately **900 electrons flow per absorbed photon** under typical bias conditions — because intentional crystal defects (chlorine-compensated copper in n-type CdS) create sensitizing centers with large hole-capture cross-sections but small electron-capture cross-sections, dramatically extending electron lifetime.

The LED's optical output is approximately **linear with forward current** from ~10 µA to 50 mA. The VTL5C series uses LEDs with forward voltage ~1.65 V typical at 20 mA, maximum current 40 mA. Older units used red/IR LEDs (~1.5 V Vf); current Xvive production uses green/yellow LEDs matched to the CdS peak sensitivity.

The defining characteristic is **extreme asymmetry between attack and decay**. Turn-on is fast because photogenerated carriers rapidly flood the conduction band — **2.5–12 ms** to reach 63% of final conductance depending on model and drive current. Turn-off is slow and nonlinear because the CdS crystal contains **multiple types of trapping centers at different energy depths** in the forbidden gap. When light ceases, free carriers recombine quickly (initial fast resistance rise), but trapped carriers are released thermally at rates governed by each trap's activation energy: **r_detrap ∝ exp(−E_trap / kT)**. Shallow traps empty in milliseconds; deep traps take seconds to minutes. The superposition of many such release rates creates the characteristic multi-timescale decay that is not describable by any single exponential.

### Published specifications for common vactrol models

The table below consolidates datasheet values and community measurements for the models most relevant to LPG design.

| Parameter | VTL5C3 | VTL5C4 | VTL5C1 | VTL5C2 | NSL-32 (Silonex) |
|---|---|---|---|---|---|
| CdS material type | 3 | 4 | 1 | 0 | — |
| R_on @ 1 mA (Ω) | 10k (CoolAudio) | 1,200 | 20,000 | 5,500 | ~500 (@ 20 mA) |
| R_on @ 10 mA (Ω) | 1k (CoolAudio) | 125 | 600 | 800 | — |
| R_on @ 40 mA (Ω) | 500 (CoolAudio) | 75 | 200 | 200 | — |
| R_off (dark, min) | ≥ 10 MΩ | ≥ 400 MΩ | ≥ 50 MΩ | ≥ 1 MΩ | ≥ 500 kΩ |
| Slope (log-log) | 20 | 18.7 | 15 | 24 | — |
| Dynamic range (dB) | 75 | 72 | 100 | 69 | — |
| Turn-on to 63% | 2.5 ms | 6.0 ms (spec) / 80 µs (meas.) | 2.5 ms | 3.5 ms | 3.5 ms (Silonex) / 26 µs (meas.) |
| Turn-off to 100 kΩ | 35 ms max | 1.5 s max / ~180 ms (meas.) | 35 ms max | 500 ms max | 500 ms (Silonex) / 120 ms (meas.) |
| Temp coefficient | Very low | High | Very high | Low | 0.7–1.0 %/°C |
| Light history effect | Very small | Large | Very large | Small | — |
| Max cell voltage | 250 V | 50 V | 100 V | 200 V | 60 V |

Measured values (from modularsynthesis.com comparative study at 5 mA drive) show significant unit-to-unit and brand-to-brand variation. Xvive VTL5C3 units measured **3.5–8.4 kΩ** at 5 mA; Excelitas units measured **2.3–4.5 kΩ** at the same current. The VTL5C3 is the standard Buchla 292 vactrol. The VTL5C3/2 is its dual-element variant (matched pair in one package), with functionally identical per-element characteristics.

The **PerkinElmer "slope" parameter** quantifies the steepness of the resistance-vs-current curve on a log-log plot: slope = log(R @ 0.5 mA / R @ 20 mA) / log(R_dark). A slope of 24 (VTL5C2) is most switch-like; 15 (VTL5C1) gives the most gradual transition and widest usable analog control range.

### The resistance-vs-illumination power law

CdS photoresistors obey an inverse power law:

**R = k · E^(−γ)**

where R is resistance, E is illuminance, k is a material constant, and γ is the sensitivity exponent. In log form this is a straight line: log R = log k − γ · log E. **Typical γ values for CdS are 0.7–0.9**, with a broader range of 0.5–1.0 depending on formulation. The GL5528 (a common standalone CdS cell) has γ ≈ 0.7 between 10 and 100 lux.

This sub-linear exponent (γ < 1) means resistance changes are **compressed** relative to illumination changes. At low light levels, small current changes produce large resistance swings (high sensitivity); at high light, the response compresses. Combined with the LED's approximately linear current-to-light relationship, this creates a **quasi-logarithmic** voltage-control characteristic that contributes to the "natural" sound of vactrol-based circuits.

### The light-history / memory effect

CdS resistance depends not only on current illumination but on **recent illumination history**. A dark-adapted cell (≥15 hours in darkness) has higher resistance at a given illumination than a light-adapted cell (24 hours at ~30 foot-candles). The ratio R_light-history / R_dark-history varies by material type and illumination level:

| Material type | @ 0.01 fc | @ 0.1 fc | @ 1.0 fc | @ 10 fc |
|---|---|---|---|---|
| Type 3 (VTL5C3) | 1.50 | 1.30 | 1.20 | 1.10 |
| Type 4 (VTL5C4) | 4.50 | 3.00 | 1.70 | 1.10 |
| Type 1 (VTL5C1) | 5.50 | 3.10 | 1.50 | 1.10 |

At the low light levels typical inside a vactrol package, Types 1 and 4 show **3–5.5× resistance variation** from history alone. Type 3 (VTL5C3) has the smallest memory effect, which is one reason it became the standard LPG vactrol — its behavior is more predictable and repeatable.

The physics: trap occupancy state persists after illumination ceases. A recently illuminated cell has more filled traps, providing a reservoir of carriers for slow thermal release, effectively lowering resistance compared to a dark-adapted cell at the same light level.

### Mathematical models for vactrol attack and decay

**Model 1 — Simple exponential (inadequate).** The basic first-order model R(t) = R_dark + (R_min − R_dark) · exp(−t/τ) fails because real vactrol decay is multi-rate: an initial fast phase followed by a prolonged slow tail. A single exponential decays too uniformly, lacking the "struck object" quality. It also cannot capture the intensity-dependent attack speed or the history effect.

**Model 2 — Sum of exponentials.** Two to four terms capture the multi-rate decay:

**R(t) = Σᵢ Aᵢ · exp(−t/τᵢ) + R_offset**

This maps directly to the physics: each term represents a carrier population associated with a distinct trap depth. The fast term (τ₁ ~ 5–50 ms) represents direct recombination and shallow trap release; the slow term (τ₂ ~ 200–2000 ms) represents deep trap release. Mutable Instruments Streams implements exactly this approach with two parallel decays at different rates for VCF and VCA control signals.

**Model 3 — Stretched exponential (Kohlrausch-Williams-Watts).** A single-parameter generalization:

**R(t) = A₀ · exp(−(t/τ)^β)**

where **β < 1** is the stretching parameter. For β = 1 this recovers a standard exponential; as β decreases, the initial decay accelerates while the tail extends — exactly matching vactrol behavior. Measured values for CdS quantum dots give **β ≈ 0.5**. The stretched exponential can be decomposed into a continuous distribution of simple exponentials, physically corresponding to the distribution of trap depths. For implementation, the Prony series approximation (tabulated weights and time constants for specific β values) converts this to a practical sum-of-exponentials form.

**Model 4 — Asymmetric first-order ODE with direction- and state-dependent time constant.** This is the approach used by Parker & D'Angelo (DAFx-13) and is the most practical for real-time use:

**dR/dt = (R_target − R) / τ_eff**

where:
- R_target is the steady-state resistance for the current LED current
- If R is decreasing (attack): τ_eff = τ_attack / (1 + k · R_current)
- If R is increasing (decay): τ_eff = τ_decay / (1 + k · R_current)
- τ_attack ≈ 12 ms, τ_decay ≈ 250 ms for VTL5C3/2

The **state-dependent modulation** (the 1 + k · R_current term) is essential — it makes the vactrol respond faster when already in a low-resistance state (recently illuminated), capturing the intensity-dependent response speed. Without this modulation, the model sounds static.

**Model 5 — The Parker & D'Angelo heuristic vactrol model (DAFx-13).** Per sample:

1. Compute error = input − output
2. If error > 0 (resistance decreasing, attack): use τ = τ_attack ≈ 12 ms
3. If error ≤ 0 (resistance increasing, decay): use τ = τ_decay ≈ 250 ms
4. Modulate τ by current output: τ_effective = τ / (1 + k · output)
5. Update: output += error · (1/Fs) / τ_effective

This is a one-pole lowpass with direction-dependent cutoff, further modulated by its own output. The discrete-time implementation uses the exact exponential form: alpha = exp(−dt/τ_eff), then output = alpha · output_prev + (1 − alpha) · target.

**Model 6 — Najnudel et al. port-Hamiltonian model (DAFx-23).** The most physically rigorous published model, based on the Iverson & Smith (1987) semiconductor equations. Two coupled ODEs describe electron and hole charge dynamics:

**dq⁻/dt = −f_opt − ν⁻₀(q⁻ − q⁺)q⁻**

**dq⁺/dt = −f_opt − ν⁺₀(q_τ + q⁺ − q⁻)q⁺**

where q⁻ and q⁺ are free electron and hole charges, q_τ is defect charge (constant), f_opt is optical flow proportional to LED power, and ν⁻₀, ν⁺₀ are recombination rate constants. Resistance is then:

**R(q⁺, q⁻) = 1 / (μ⁺₀ q⁺ + μ⁻₀ q⁻)**

bounded by dark and illuminated limits. The attack/decay asymmetry, intensity dependence, and history effect all emerge naturally from the nonlinear dynamics without heuristic switching. Parameters were fit to VTL5C3/2 measurements by least squares. The port-Hamiltonian formulation guarantees passivity (energy conservation), making the model inherently stable. Computational cost is moderate — two coupled ODEs per sample — but higher than the Parker heuristic.

**Model 7 — Mutable Instruments Streams (Émilie Gillet).** The plucked mode uses:

1. On trigger, initialize two state variables to maximum
2. Per sample, decay each independently: state[i] −= state[i] · k_decay[i] (exponential decay via recursive multiply)
3. state[0] uses fast_decay_coefficient (VCF envelope, shorter)
4. state[1] uses decay_coefficient (VCA envelope, longer)
5. Each passes through a secondary asymmetric slew limiter with different rise/fall coefficients
6. Output: the VCA state controls amplitude, the VCF state controls filter cutoff scaled by frequency_amount

This is a **dual-exponential model cascaded with asymmetric slew limiters** — computationally cheap, musically effective, and exposes intuitive parameters.

### Building a parametric model that morphs between vactrol types

A practical parametric model needs eight parameters:

1. **R_min** — minimum resistance (fully illuminated): 30–600 Ω
2. **R_max** — maximum resistance (dark): 100 kΩ – 20 MΩ
3. **τ_attack** — attack time constant: 1–50 ms
4. **τ_decay_fast** — initial fast decay: 10–100 ms
5. **τ_decay_slow** — long tail decay: 100–2000 ms
6. **mix** — blend between fast and slow decay: 0–1
7. **β** — nonlinearity/stretching exponent: 0.3–1.0 (1.0 = pure exponential)
8. **γ** — current-to-resistance power law exponent: ~0.7–1.4

Anchor parameter sets for morphing:

| Vactrol type | τ_attack | τ_decay_fast | τ_decay_slow | R_min | R_max | γ |
|---|---|---|---|---|---|---|
| VTL5C3 (Buchla standard) | 2.5 ms | 35 ms | 250 ms | 1 kΩ | 10 MΩ | 1.4 |
| VTL5C4 (slow, low R_on) | 6 ms | 180 ms | 1500 ms | 75 Ω | 400 MΩ | 1.2 |
| VTL5C1 (fast, high range) | 2.5 ms | 35 ms | 200 ms | 600 Ω | 50 MΩ | 1.0 |
| NSL-32 (Silonex, medium) | 3.5 ms | 120 ms | 500 ms | 500 Ω | 500 kΩ | 1.1 |

**Interpolate in log domain** (log τ, log R) between anchor points for musically useful intermediate behaviors. Linear interpolation of log-transformed parameters produces smooth perceptual transitions.

---

## LAYER 2 — Resistance-to-Signal Interaction Model

### The complete transfer chain

The signal chain from control voltage to audio effect is:

**CV → control circuit → I_LED (LED current) → optical coupling → E (illuminance) → CdS photoconductor → R_f(t) (vactrol resistance) → audio gain and/or filter cutoff**

Each stage introduces nonlinearity.

### LED current to resistance mapping

Parker & D'Angelo fit the VTL5C3/2 datasheet to obtain the power-law relationship:

**R_f = A / I_f^γ + B**

where **A = 3.464 Ω·A^1.4**, **B = 1136.212 Ω**, **γ = 1.4**, and I_f is LED forward current in amperes. This is valid for I_f ∈ [10 µA, 40 mA]. At I_f = 40 mA (maximum), R_f ≈ 1,140 Ω. At I_f = 10 µA, R_f ≈ 2.3 MΩ. The residual offset B represents the minimum achievable resistance even at high drive — a physical limitation of the CdS material's bulk resistance.

For other vactrol types, the same functional form applies with different A, B, and γ. The exponent γ relates to the PerkinElmer "slope" parameter and to the CdS gamma discussed in Layer 1.

### Mapping resistance to VCA gain

The Buchla 292's VCA action comes from a **voltage divider** formed by the vactrol resistance R_f and the shunt resistance R_α:

**G_VCA = R_α / (R_α + 2·R_f)**

In "Both" (combo) mode, R_α = 5 MΩ. When R_f is small (vactrol lit, ~1 kΩ), gain approaches unity: G ≈ 5M / (5M + 2k) ≈ 0.9996. When R_f is large (vactrol dark, ~10 MΩ), gain drops: G ≈ 5M / (5M + 20M) = 0.2, and the filter poles provide additional attenuation. In "VCA" mode, R_α = 5 kΩ, making the divider dominate: G ≈ 5k / (5k + 2·R_f), which gives a much wider attenuation range but minimal filtering.

The gain in dB follows: **G_dB = 20·log₁₀(R_α / (R_α + 2·R_f))**. Because R_f follows a power law of LED current, which itself follows the vactrol's nonlinear slew dynamics, the effective gain-vs-time curve during decay is a complex, multi-rate function — not a simple exponential fade.

### Mapping resistance to filter cutoff

When the vactrol controls an RC lowpass filter, the cutoff frequency is:

**f_c = 1 / (2π · R_f · C)**

For the Buchla 292 with C₁ = 1 nF: at R_f = 1 kΩ (lit), f_c ≈ 159 kHz (well above audio — filter is fully open). At R_f = 1 MΩ (nearly dark), f_c ≈ 159 Hz. At R_f = 10 MΩ (dark), f_c ≈ 16 Hz. This **five-decade resistance sweep maps to a five-decade frequency sweep**, covering the entire audio range and beyond.

The 292's actual filter has two non-coincident poles (from C₁ = 1 nF and C₂ = 220 pF), so the effective cutoff behavior is more complex than a single RC, but the fundamental R → f_c mapping applies to each pole independently.

### The power-law nonlinearity creates the "natural" sound

Because R ∝ I^(−1.4) and f_c ∝ 1/R, we get **f_c ∝ I^1.4** — the cutoff frequency is a superlinear function of LED current. During decay, as the vactrol's internal state decays from its peak, the cutoff frequency drops faster than linearly. Combined with the simultaneous amplitude drop from the voltage divider, this creates a **frequency-dependent amplitude decay** where high frequencies disappear first, mimicking the natural physics of struck acoustic objects where higher vibrational modes dissipate energy faster than the fundamental.

### Modeling the history effect in DSP

For applications requiring accurate history-dependent behavior, implement the memory effect as a **slow state variable** tracking recent average illumination:

**dH/dt = (I_LED − H) / τ_history**

where τ_history ≈ 1–10 seconds (the timescale of trap filling/emptying). Then modify the resistance mapping:

**R_f = (A / I_f^γ + B) · (1 + η · (1 − H/H_max))**

where η is the history sensitivity coefficient (0 for VTL5C3, ~0.5–1.0 for VTL5C4/VTL5C1, based on the PerkinElmer ratio data). When H is low (dark-adapted), resistance increases by factor (1 + η); when H is high (light-adapted), the factor approaches 1. For VTL5C3 with its "very small" history effect, this correction can be omitted.

---

## LAYER 3 — Complete LPG Model

### The Buchla 292 audio path topology

The Buchla 292 audio path, as analyzed by Parker & D'Angelo from schematics, consists of two vactrol resistances R_f in series with the signal path, two shunt capacitors (C₁ = 1 nF and C₂ = 220 pF) forming two RC lowpass poles, a shunt resistance R_α to ground, and an optional feedback capacitor C₃ = 4.7 nF (lowpass mode only) providing Sallen-Key-like positive feedback.

The signal flows: **Input → R_f → node Vx → R_f → V+ (op-amp non-inverting input) → unity-gain buffer → Output**. C₂ (220 pF) shunts from Vx to ground; C₁ (1 nF) shunts from V+ to ground; R_α connects V+ to ground. In lowpass mode, C₃ feeds back from the output through a gain stage (gain = a) to the Vx node.

The continuous-time transfer function is:

**H_LPG(s) = 1 / (α₁ + α₂·s + α₃·s²)**

where:
- **α₁ = 1 + 2·R_f / R_α** (controls DC gain / VCA attenuation)
- **α₂ = R_f·(2·C₁ + C₂ − C₃·(a−1) + (C₂+C₃)·R_f/R_α)** (controls damping / bandwidth)
- **α₃ = R_f²·C₁·(C₂ + C₃)** (controls resonant frequency)

**DC gain**: G(0) = R_α / (R_α + 2·R_f). This is the voltage-divider action that makes the 292 simultaneously a VCA and a filter — as R_f increases, both the cutoff drops and the passband gain drops.

**Maximum stable feedback gain**: a_max = (2·C₁·R_α + (C₂+C₃)·(R_α+R_f)) / (C₃·R_α). For a > a_max, poles cross the imaginary axis and the filter self-oscillates. Define a normalized resonance parameter **a_norm ∈ (0, 1]** and set a = a_norm · a_max to decouple resonance from cutoff.

### Three modes and their component values

| Component | "Both" / Combo | "VCA" / Gate | "Lowpass" |
|---|---|---|---|
| C₁ | 1 nF | 1 nF | 1 nF |
| C₂ | 220 pF | 220 pF | 220 pF |
| C₃ | 0 (disconnected) | 0 (disconnected) | 4.7 nF |
| R_α | 5 MΩ | 5 kΩ | 5 MΩ |

**Combo mode** produces two non-coincident filter poles with an effective slope closer to **−6 dB/octave** than −12 dB/oct, plus the voltage-divider VCA action. This is the classic LPG sound.

**VCA mode** lowers R_α to 5 kΩ, pushing the filter poles to very high frequencies where they are inaudible. The voltage divider G = 5k / (5k + 2·R_f) provides clean amplitude control. An input gain stage compensates ~4–5 dB for the level difference between modes.

**Lowpass mode** adds C₃ = 4.7 nF as Sallen-Key positive feedback, producing a resonant peak at cutoff. The original Buchla 292 does not have user-adjustable resonance; derivative designs (Thomas White/NRM version, which Parker & D'Angelo modeled) add a variable-gain feedback amplifier.

### Why LPGs produce the "pluck" and "bongo" character

Four mechanisms combine to create the percussion-like timbre:

The **coupled VCA+VCF decay** means high frequencies die away faster than low frequencies during the same decay event. As R_f increases, both the amplitude drops (voltage divider) and the filter closes (RC cutoff drops). This is exactly how real acoustic objects behave — higher modes of vibration dissipate energy faster than the fundamental. No separate envelope generator is needed; the vactrol's response to a brief trigger pulse inherently produces this behavior.

The **nonlinear, multi-rate decay curve** provides a fast initial transient (the "attack" of the percussive sound when heard in reverse — the initial bright burst) followed by a long, slowly fading tail. This profile closely resembles the energy dissipation in struck membranes and bars.

The **power-law resistance mapping** means the filter cutoff and gain change non-uniformly during decay. The initial cutoff drop is rapid (while R_f is small and changing fast), then slows as R_f grows large — the brightness "lingers" briefly before fading, much like the ring of a bongo.

The **R_f-dependent VCA gain curve** is itself nonlinear (the voltage divider is not a linear function of R_f), adding further shaping to the amplitude envelope.

The DSP parameters controlling pluck character are: **vactrol rise time** (attack sharpness), **vactrol fall time** (decay duration), **coupling ratio** between VCA and filter (set by R_α — higher R_α means more coupling), **resonance** (adds metallic "ping"), **initial brightness** (determined by minimum R_f / maximum LED current), and **input spectrum** (harmonically rich inputs produce the most dramatic timbral evolution).

### Topology-preserving discretization of the 292 filter

The recommended approach from Parker & D'Angelo replaces all continuous-time integrators and differentiators with their trapezoidal-rule discrete equivalents while preserving the circuit topology. This is critical because **direct-form IIR filters (DF2T via bilinear transform) become unstable under the fast coefficient modulation inherent to LPG operation** — their internal states don't correspond to physical capacitor voltages, so coefficient changes corrupt stored energy.

The trapezoidal integrator in DF2T form:

**y[n] = s[n] + (1/(2·Fs)) · x[n]**
**s[n+1] = s[n] + (1/Fs) · x[n]**

The trapezoidal differentiator in DF2T form:

**y[n] = s[n] + 2·Fs · x[n]**
**s[n+1] = −s[n] − 4·Fs · x[n]**

The three coupled outputs (y_d for the differentiator, y_x for the first integrator, y_o for the second integrator) form a system with delay-free loops that must be solved simultaneously. The analytical solution is:

**y_x = (a₂·b₄·s_d + 2Fs·(b₃ − 2b₄d₁Fs)·s_o − 2a₂Fs·s_x − a₂b₁·y_i) / (a₁b₃ − a₂b₂ − 2a₁b₄d₁Fs + 2a₂b₄d₂Fs)**

**y_o = (s_o + (a₁/(2Fs))·y_x) / (1 − a₂/(2Fs))**

**y_d = s_d + 2Fs·(d₁·y_o + d₂·y_x)**

where the coefficients a₁, a₂, b₁–b₄, d₁, d₂ derive from the circuit components:

- a₁ = 1/(C₁·R_f), a₂ = −(1/C₁)·(1/R_f + 1/R_α)
- b₁ = 1/(C₂·R_f), b₂ = −2/(C₂·R_f), b₃ = 1/(C₂·R_f), b₄ = C₃/C₂
- d₁ = a (feedback gain), d₂ = −1

After computing outputs, update all six states (s_d, s_x, s_o and their DF2T partners) and advance. **This model is stable under all physically reasonable parameter changes at any modulation rate.**

For nonlinear extension, replace the linear feedback gain a with a tanh saturator using instantaneous linearization (first-order Taylor around the previous sample's operating point):

**y_d = s_d + 2Fs·(d₁·[tanh(x_prev) + (y_o − x_prev)·(1 − tanh²(x_prev))] + d₂·y_x)**

### Alternative filter topologies and their tradeoffs

| Topology | Poles | Resonance | Cost (ops/sample) | Stability under modulation | Authenticity to 292 |
|---|---|---|---|---|---|
| TPT Buchla 292 (Parker) | 2 (non-coincident) | Yes (C₃ path) | ~15–20 | Excellent | Exact |
| Cytomic/Simper SVF | 2 (coincident) | Full Q control | ~10 | Excellent | Approximate |
| 1-pole TPT lowpass | 1 | No | ~5 | Excellent | Partial |
| Direct-form biquad (DF2T) | 2 | Yes | ~5 | **Poor under modulation** | N/A |

The **Cytomic SVF** is the best alternative when exact 292 replication is not required. Per-sample processing:

g = tan(π · f_c / f_s), k = 1/Q

a1 = 1 / (1 + g·(g + k)), a2 = g · a1, a3 = g · a2

v1 = a1·ic1eq + a2·(v0 − ic2eq) [bandpass]

v2 = ic2eq + a2·ic1eq + a3·(v0 − ic2eq) [lowpass output]

ic1eq = 2·v1 − ic1eq, ic2eq = 2·v2 − ic2eq [state updates]

Map vactrol resistance to cutoff via f_c = 1/(2π·R_f·C) and use the lowpass output. The SVF's coincident poles give a steeper −12 dB/oct slope compared to the 292's gentler non-coincident response, which is an audible difference at moderate cutoff values.

The **1-pole TPT** (g = tan(π·f_c/f_s), v = g/(1+g)·(input − state), output = v + state, state = output + v) is simplest, cheapest, and captures the basic timbral character of cutoff tracking amplitude. It misses the 292's second pole and any resonance, but is effective for many musical applications.

### Complete LPG signal flow for implementation

The per-sample processing order:

1. **Read CV input**; apply control circuit model (shelving filter, logarithmic amplifier with zener clamp) to compute I_LED. The Parker model uses Lambert W function (approximated as piecewise cubic spline) for the LED/zener diode equation.
2. **Apply vactrol slew model**: compute smoothed vactrol state using asymmetric one-pole with state-dependent time constant. Output is the "effective illumination."
3. **Map to resistance**: R_f = A / state^γ + B (power law).
4. **Compute filter coefficients** from R_f, R_α, C₁, C₂, C₃ (recompute a₁, a₂, b₁–b₄, d₁, d₂ and the delay-free-loop solution).
5. **Process audio sample** through the TPT filter: solve the 3-equation system, produce output y_o.
6. **Update all states** (6 filter states + vactrol envelope state + history state if modeled).

For mode switching, smoothly interpolate R_α between 5 MΩ and 5 kΩ, and C₃ between 0 and 4.7 nF, to avoid clicks. Use exponential smoothing with a ~5 ms time constant on these parameters.

---

## Cross-Cutting Implementation Concerns

### Numerical stability across the full time-constant range

At 96 kHz with τ_attack = 2.5 ms, the per-sample envelope coefficient is exp(−1/(96000 × 0.0025)) ≈ 0.99584. With τ_decay = 5 s, it is exp(−1/(96000 × 5)) ≈ 0.999998. Both are well within float32 precision, so **coefficient underflow is not a concern for the envelope ODE**. The real challenge is the audio filter: R_f spanning 600 Ω to 10 MΩ maps to cutoff frequencies from ~16 Hz to >20 kHz, giving g = tan(π·f_c/f_s) values from ~0.001 to ~0.65. The TPT/trapezoidal approach handles this entire range stably because the bilinear transform maps the entire left s-half-plane to the z-plane unit disk interior.

**Denormalized floats** are a real concern during long decay tails. As filter state variables approach zero, they can enter the denormalized range (below ~1.17×10⁻³⁸ for float32), causing **100×+ CPU slowdowns on x86**. Three mitigations:

- Set FTZ/DAZ processor flags at the start of the audio callback (SSE: _MM_FLUSH_ZERO_ON, _MM_DENORMALS_ZERO_ON; ARM NEON forces FTZ by default)
- Inject a DC offset of ~1×10⁻¹⁸ into the filter input (−360 dB, inaudible, keeps state variables normalized)
- Flush state variables to zero when |state| < 1×10⁻²⁰

For the LPG specifically, DC offset injection is cleanest because the vactrol naturally drives the filter toward silence during decay.

### Coefficient smoothing and the vactrol's built-in advantage

A critical insight: **the vactrol model itself is a lowpass filter on the control signal**. The asymmetric slew limiter with τ ≥ 2.5 ms rise and τ ≥ 35 ms fall means R_f cannot change faster than these rates. This inherently prevents zipper noise from audio-rate coefficient changes in the filter. The vactrol's sluggishness is not a limitation to work around — it is the core feature that makes the sound musical.

Coefficient smoothing is still needed for user-controlled parameters (offset, resonance) that bypass the vactrol model. Use exponential smoothing: smoothed += α · (target − smoothed), where α = 1 − exp(−2π · f_smooth / f_s) with f_smooth ≈ 20–50 Hz.

For the vactrol envelope itself, compute at **audio rate** (not control rate). The fast attack (potentially ≤2.5 ms = ~110 samples at 44.1 kHz) is too fast for typical control-rate block sizes of 32–64 samples without audible artifacts. The slow decay phase could theoretically run at control rate, but the implementation complexity of rate-switching rarely justifies the CPU savings.

Wishnick (DAFx-14) proved that the trapezoidal SVF is **time-varying BIBO stable** for all positive g and damping values, meaning you can safely update g every sample without stability concerns. The key constraint is that interpolated coefficient values must correspond to physically valid filter states — interpolate g linearly, not derived quantities like 1/(1 + g·(g + k)).

### Oversampling recommendations

Parker & D'Angelo recommend **2× oversampling** for the LPG model. The primary benefit is reducing bilinear-transform frequency warping: at 44.1 kHz, a 10 kHz analog cutoff maps to ~9.3 kHz digitally; at 88.2 kHz internal rate, it maps to ~9.8 kHz. Time-varying filter modulation aliasing is not a major concern because the vactrol limits the cutoff sweep rate to ≤400 Hz, well below Nyquist.

If the tanh nonlinearity is included in the resonance feedback path, **4× oversampling** is recommended to keep harmonic aliasing below −60 dB.

| OS factor | Internal rate (base 44.1k) | CPU multiplier | When to use |
|---|---|---|---|
| 1× | 44.1 kHz | 1× | Prototyping; acceptable if cutoffs stay below ~8 kHz |
| 2× | 88.2 kHz | ~2.5× (incl. halfband filter) | **Standard recommendation** for linear models |
| 4× | 176.4 kHz | ~5× | Required for nonlinear (tanh) feedback models |

The 2.5× CPU multiplier (not 2×) accounts for a 17-tap halfband anti-aliasing/reconstruction FIR filter (~8.5 multiply-adds per output sample).

### Computational cost budget

Per-sample cost for one complete LPG voice (no oversampling):

| Stage | Operations | Notes |
|---|---|---|
| Vactrol envelope (asymmetric 1-pole) | ~5 | 1 branch, 2 multiplies, 2 adds |
| Resistance mapping (power law) | ~5–10 | Fast approx or LUT with interpolation |
| Filter coefficient computation | ~8–12 | Fast tan() approx or LUT for g; 5 ops for auxiliary coefficients |
| Audio filter (TPT 292 topology) | ~15–20 | 3-equation system solve + 6 state updates |
| **Total** | **~35 ops/sample** | Without oversampling |
| With 2× OS + halfband FIR | **~87 ops/sample** | At base sample rate |

For the Cytomic SVF alternative: ~25 ops/sample total (simpler coefficient computation), ~62 ops/sample with 2× oversampling. For the 1-pole TPT: ~20 ops/sample total.

Memory per voice: 6 floats for filter states + 2 for envelope + ~8 for coefficient cache = **~64 bytes**, easily fitting in L1 cache for 128+ simultaneous voices.

**SIMD strategy**: IIR filters are inherently serial within a voice. Process 4 LPG voices in parallel using SSE float32×4 (or 8 with AVX). The tan() computation, power-law mapping, and halfband FIR all vectorize well. Realistic speedup: **3–4× throughput** with 4-voice batching.

### Discretization of the envelope ODE

Three approaches for converting the continuous vactrol dynamics dR/dt = (R_target − R)/τ to discrete time:

**Exact exponential (recommended)**: α = exp(−dt/τ), then R[n] = α·R[n−1] + (1−α)·R_target[n]. This is the exact solution of the first-order ODE assuming constant input over one sample period. Precompute α_attack = exp(−1/(f_s·τ_attack)) and α_decay = exp(−1/(f_s·τ_decay)) at initialization. No frequency warping, inherently stable, and sample-rate independent when τ is specified in seconds.

**Backward Euler**: R[n] = (R[n−1] + (dt/τ)·R_target[n]) / (1 + dt/τ). Unconditionally stable, simpler than exponential (avoids exp()), but slightly less accurate for very fast time constants.

**Trapezoidal rule**: More accurate but requires storing previous input. Generally unnecessary for the envelope smoother since the exact exponential is both cheap and maximally accurate.

For sample-rate independence, always specify time constants in seconds and recompute coefficients when the sample rate changes. The audio filter's g = tan(π·f_c/f_s) automatically adapts to sample rate through the bilinear transform.

### Published parameter lookup table for direct implementation

| Parameter | VTL5C3 | VTL5C3/2 (dual) | Units | DSP usage |
|---|---|---|---|---|
| τ_rise | 0.0025 | 0.012 | seconds | α_rise = exp(−1/(fs·τ)) |
| τ_fall | 0.035 | 0.250 | seconds | α_fall = exp(−1/(fs·τ)) |
| R_on (typical) | 1,000–4,000 | ~4,000 | Ω | Lower clamp for R_f |
| R_off (min) | 10,000,000 | 10,000,000 | Ω | Upper clamp for R_f |
| Power-law A | 3.464 | 3.464 | Ω·A^1.4 | R_f = A/I^1.4 + B |
| Power-law B | 1,136 | 1,136 | Ω | R_f = A/I^1.4 + B |
| Power-law γ | 1.4 | 1.4 | — | Exponent in R_f mapping |
| I_f minimum | 10×10⁻⁶ | 10×10⁻⁶ | A | Lower clamp |
| I_f maximum | 40×10⁻³ | 40×10⁻³ | A | Upper clamp (datasheet abs max) |
| C₁ (292 filter) | 1×10⁻⁹ | 1×10⁻⁹ | F | Filter pole 1 |
| C₂ (292 filter) | 220×10⁻¹² | 220×10⁻¹² | F | Filter pole 2 |
| C₃ (LP mode) | 4.7×10⁻⁹ | 4.7×10⁻⁹ | F | Resonance feedback |
| R_α (Combo) | 5×10⁶ | 5×10⁶ | Ω | Shunt / VCA divider |
| R_α (VCA) | 5×10³ | 5×10³ | Ω | VCA-only mode |

## Conclusion

Three architectural tiers emerge from this survey, each trading accuracy for computational cost. The **Parker & D'Angelo topology-preserving model** (DAFx-13) remains the gold standard for Buchla 292 emulation — it faithfully reproduces the circuit's coupled VCA/filter behavior, handles all three modes, and runs comfortably in real time with 2× oversampling at ~87 operations per output sample. Its heuristic vactrol model (asymmetric slew with state-dependent time constant) captures the essential character with minimal computation. The **Najnudel et al. port-Hamiltonian model** (DAFx-23) offers superior physical accuracy — the attack/decay asymmetry and history effect emerge from semiconductor physics rather than heuristic switching — but at higher computational cost suitable for offline or GPU-accelerated contexts. The **Mutable Instruments dual-decay approach** provides the cheapest musically convincing result: two parallel exponential decays at different rates, cascaded with asymmetric slew limiters, driving a simple one-pole or SVF filter.

The single most important implementation detail across all approaches is **topology-preserving filter discretization**. Direct-form biquads fail catastrophically under the rapid coefficient modulation inherent to LPG operation. The trapezoidal integrator, as formalized by Zavalishin and applied to the 292 by Parker & D'Angelo, solves this completely. The second most important detail is the vactrol's state-dependent time constant — without it, the model loses the characteristic acceleration of response at high illumination levels that gives real vactrols their dynamic, living quality.