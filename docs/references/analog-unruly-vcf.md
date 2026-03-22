# DSP emulation of unruly analog VCFs

Three analog filters — the Korg MS-20, Polivoks, and EDP Wasp — define a class of "misbehaved" VCFs whose musical character arises from nonlinearities that most filter designers would call defects: diode clipping inside feedback loops, slew-rate saturation of programmable op-amps operated far outside spec, and CMOS digital logic gates abused as analog amplifiers. Faithfully reproducing these behaviors in software demands more than a standard biquad with a `tanh()` tacked on. This report provides the circuit-level understanding, mathematical models, and DSP implementation strategies an engineer needs to build accurate real-time emulations of all three filters.

The core technical challenge is identical across all three targets: **nonlinear elements sit inside the filter's feedback loop**, creating implicit equations that cannot be solved with a simple closed-form expression at each sample. Every major DSP methodology — zero-delay feedback (ZDF), wave digital filters (WDF), and state-space/DK methods — offers a different tradeoff for handling this, and the choice depends on the specific filter topology and your CPU budget.

---

## Part I: The three filters and their nonlinear souls

### The Korg MS-20's two distinct filter revisions

The MS-20 contains a **highpass and lowpass in series** (HPF → LPF → VCA), each second-order. Two circuit revisions exist with fundamentally different nonlinear behavior, and any accurate emulation must distinguish between them.

**Revision 1 — The Korg-35 (discrete transistor Sallen-Key).** The early filter uses a proprietary potted module containing five transistors and six resistors. Timothy Stinchcombe's definitive 2006 analysis revealed it as a **Sallen-Key topology with a deliberate 3:1 capacitor ratio** (C₁ = 3.3 nF, C₂ ≈ 1.1 nF). Two transistors (2SC1623) operate in reverse saturation mode as current-controlled variable resistors, with equivalent resistance R = Vₜ / (I_B · β_R) where β_R = 4.21. The unequal capacitors shift the self-oscillation threshold from k₁k₂ = 3 (equal-value Sallen-Key) down to **k₁k₂ = 7/3 ≈ 2.33**, which falls within the circuit's achievable gain range of ≈ 2.2. The lowpass transfer function is:

$$H_{\text{LP}}(s) = \frac{k_1}{s^2/\omega_c^2 + (7/3 - k_1 k_2)\,s/\omega_c + 1}$$

The critical nonlinearity: **diode clipping sits in the forward path**, inside the high-gain amplifier stage (gain ≈ 58). A pair of back-to-back 1N4148 silicon diodes conduct at input levels as low as ≈ 8 mV (0.5 V / 58). Below the conduction threshold, the full gain of 58 applies; above it, gain drops to unity. This means **all frequencies in the passband experience progressive harmonic distortion** as signal level increases — not just the resonance peak. Additionally, the signal itself modulates the transistor currents in the exponential converter, producing **asymmetric resonance**: Stinchcombe's SPICE analysis shows the negative half-cycle resonates at ≈ 2.0 kHz while the positive half resonates at ≈ 1.7 kHz at matched settings. This asymmetry is a signature of the Korg-35 that the OTA revision lacks entirely.

**Revision 2 — The LM13700 OTA filter.** The later revision, mounted on a separate KLM-307 daughterboard, uses two OTAs from an LM13600 or LM13700 IC. Stinchcombe showed this is **not a Sallen-Key filter** despite widespread claims — a unity-gain buffer between stages eliminates the inter-stage loading that defines Sallen-Key. It is instead two cascaded, buffered first-order sections. The transfer function becomes:

$$H_{\text{LP}}(s) = \frac{-k_1}{s^2/\omega_c^2 + (2 - k_1 k_2)\,s/\omega_c + 1}$$

Self-oscillation occurs at k₁k₂ = 2. The crucial difference: **three back-to-back diodes are placed in the feedback path** (lower gain stage, gain ≈ 4), conducting only at ≈ 525 mV input. This means only frequencies near the cutoff/resonance frequency are distorted — passband content remains relatively clean. Each OTA contributes its own tanh saturation: I_out = (I_abc/2) · tanh(V_diff / 2Vₜ) where Vₜ ≈ 26 mV.

**The HPF anomaly.** In both revisions, the highpass mode is created by grounding the normal LP input and feeding signal into the lifted end of C₂. Stinchcombe proved the result is a **6 dB/octave bandpass in parallel with a 12 dB/octave highpass**, with the BPF term dominating below cutoff. The effective HPF rolloff is **6 dB/oct, not 12 dB/oct** — a widely misreported specification that any emulation must reproduce.

### The Polivoks: a capacitorless state-variable filter

The Polivoks VCF, designed by Vladimir Kuzmin in 1982, is a **second-order state-variable filter with no discrete capacitors in the filter core** — a unique topology that produces its famous aggressive, squelchy character.

The "trick" exploits the **K140UD12 programmable op-amp** (a Soviet clone of the Fairchild μA776/LM4250). These ICs have an external current-programming pin (I_set) that controls the internal bias current, gain-bandwidth product, and slew rate simultaneously. By operating at very low I_set currents (0.2–13.76 μA), the gain-bandwidth product is pushed down into the audio range, and the **internal frequency compensation capacitor** (≈ 30 pF) acts as the integrating element. The op-amp becomes a voltage-controlled integrator: cutoff frequency ω_c = g_m / C_comp = I_set / (2Vₜ · C_comp).

The state equations for the filter are:

$$\frac{dx_1}{dt} = \omega_c \cdot f_{\text{nl}}(a \cdot u - x_1 - k \cdot x_2) \quad \text{(bandpass state)}$$

$$\frac{dx_2}{dt} = \omega_c \cdot f_{\text{nl}}(x_1) \quad \text{(lowpass state)}$$

where x₁ = bandpass output, x₂ = lowpass output, u = input, k = resonance feedback gain, and f_nl is the nonlinear function.

**The dominant nonlinearity is slew-rate saturation**, not amplitude clipping. When the signal demands a rate of change exceeding the op-amp's maximum slew rate (set by I_set), the output becomes a linear ramp rather than following the signal. This is fundamentally a **rate limiter**, modeled as:

$$f_{\text{nl}}(x) = SR \cdot \tanh\!\left(\frac{x}{V_{\text{sat}}}\right)$$

where SR is the slew rate limit (proportional to I_set) and V_sat is the input differential pair saturation voltage. The outputs also hard-clip at the supply rails (±12.5 V original, ±15 V in clones).

This rate-limiting mechanism produces the Polivoks's distinctive **trapezoidal self-oscillation waveform** at high frequencies — the slew limiting linearizes the rising and falling edges while rail clipping flattens the peaks. At lower frequencies where the slew rate is adequate, the waveform approaches a distorted sine. The original circuit includes **no explicit amplitude-limiting elements** in the resonance path; self-oscillation goes rail-to-rail, which is why the resonance sounds so aggressive. Some clone designs add clamping diodes for a "soft resonance" option.

The musical "squelch" arises from the interaction of several effects: slew-rate distortion adds odd harmonics that scale with both signal amplitude and frequency; the nonlinearity is *inside* the integration loop, creating intermodulation products that track with the cutoff frequency; and crossover distortion at low I_set adds graininess.

**No dedicated academic paper on Polivoks virtual analog modeling exists** — this is a confirmed gap in the DAFx/AES/ICMC literature. The closest relevant work is D'Angelo and Välimäki's general op-amp/diode emulation techniques and Köper/Holters' state-space methods for similar topologies.

### The EDP Wasp: digital logic gates as analog amplifiers

Chris Huggett's 1978 Wasp filter is a **second-order multimode state-variable filter** that substitutes CD4069UBE unbuffered CMOS inverters for operational amplifiers and uses CA3080 OTAs for frequency control. The filter simultaneously provides lowpass, bandpass, and highpass outputs.

The signal flow is: input → AC coupling → IC1 (CMOS summing amplifier, HP output) → voltage divider → IC2 (CA3080 OTA, controls cutoff) → IC3 (CMOS integrator, BP output) → voltage divider → IC4 (CA3080 OTA) → IC5 (CMOS integrator, LP output) → global feedback to IC1 via R₂/C₂; and intermediate bandpass feedback to IC1 via resonance network with 1N4148 diode limiters.

**The CMOS inverter nonlinearity** arises from using a digital logic gate in its linear transition region. When biased at V_DD/2 via a DC feedback resistor, the inverter operates on the steep portion of its S-shaped voltage transfer characteristic (VTC), functioning as a high-gain inverting amplifier. The VTC is sigmoidal but differs from a BJT-based OTA's tanh() curve — it arises from the **quadratic (square-law) MOSFET I-V relationship** rather than exponential BJT characteristics. The transition may be asymmetric due to NMOS/PMOS threshold voltage mismatch, introducing even harmonics alongside the predominantly odd-harmonic distortion.

Köper and Holters (DAFx-2022) developed the definitive model using extended MOSFET equations where the gain factor α and threshold voltage V_T are polynomial functions of the gate-source voltage:

$$i_D = \frac{\alpha(v_{GS})}{2} \cdot (v_{GS} - V_T(v_{GS}))^2 \cdot (1 + \lambda \cdot v_{DS}) \quad \text{(saturation region)}$$

with α(v_GS) = c_{α,2} v_{GS}^2 + c_{α,1} v_{GS} + c_{α,0} and V_T(v_GS) similarly polynomial. The **optimized parameters** (fitted to measurements at 12 V supply) for the NMOS are: c_{α,0} = 0.0022, c_{α,1} = −1.05×10⁻⁴, c_{α,2} = 3.13×10⁻⁶, c_{V_T,0} = 0.8655, c_{V_T,1} = 0.2212, c_{V_T,2} = 0.0149, λ = 10⁻³.

**The extended OTA model** is equally critical. The standard model i_OTA = α·i_bias·tanh(β·v_OTA / 2V_TH) fails to capture the dramatic output current changes when the output voltage approaches the supply rails. Köper et al. proposed:

$$i_{\text{OTA}} = f_M + f_H + f_L$$

where f_M is the standard mid-range tanh model, f_H models the high-rail transition via tanh(γ_H(v_out − ΔV_H)), and f_L models the low-rail transition. Without the rail model, self-oscillation simulations diverge; with it, they correctly reproduce the bounded, clipped self-oscillation observed in hardware. The measured OTA parameters are α ≈ 0.8635 and β ≈ 0.9408 for the CA3080.

The combined integrator cutoff frequency is ω_c = (i_bias / 2C₃V_TH) · R₆/(R₈R₇/(R₈+R₇) + R₆ + R₅) ≈ 9.855 × 10⁸ · i_bias, with integrating capacitors C₃ = C₄ = 330 pF in the Doepfer A-124 version.

---

## Part II: DSP emulation strategies in depth

### Zero-delay feedback and the topology-preserving transform

The VA/ZDF approach, formalized by Vadim Zavalishin in *The Art of VA Filter Design* and made accessible by Will Pirkle's books and application notes, is the most widely used method for real-time filter emulation. The core idea: replace each analog integrator with a **trapezoidal-rule discrete integrator** that preserves the circuit's feedback topology.

The trapezoidal integrator discretizes y(t) = ∫x(t)dt as:

$$y[n] = g \cdot x[n] + s[n], \qquad s[n+1] = g \cdot x[n] + y[n]$$

where **g = tan(ω_c T/2)** is the frequency-warped gain coefficient (ω_c = cutoff angular frequency, T = sample period). This is the bilinear transform's frequency warping, and the resulting structure is the Transposed Direct Form II (TDF2) integrator.

For a **linear 2-pole SVF** (applicable to the Wasp and Polivoks topologies), the zero-delay loop resolves algebraically. Andy Simper's widely-used Cytomic formulation gives:

$$y_{\text{HP}} = \frac{x - 2R \cdot y_{\text{BP}} - y_{\text{LP}}}{1 + 2Rg + g^2}, \quad y_{\text{BP}} = g \cdot y_{\text{HP}} + s_1, \quad y_{\text{LP}} = g \cdot y_{\text{BP}} + s_2$$

where R = 1/(2Q) is the damping factor and state updates are s₁ ← 2y_BP − s₁, s₂ ← 2y_LP − s₂.

For the **Korg-35**, Pirkle's TPT implementation resolves the Sallen-Key feedback loop analytically in the linear case, with the self-oscillation threshold at K = 2.0 (OTA version) or K ≈ 2.33 (original Korg-35). A nonlinear processing (NLP) block models the diode saturation as y = tanh(σ · x), where σ controls saturation severity.

**When nonlinear elements sit inside the feedback loop** — the core challenge for all three filters — the implicit equation at each sample becomes:

$$u = x - k \cdot f_{\text{nl}}(y_4(u))$$

This has no closed-form solution. The standard approaches are:

- **Newton-Raphson iteration**: u_{k+1} = u_k − F(u_k)/F'(u_k), with **quadratic convergence** — typically 2–4 iterations suffice at audio rates. Requires computing the Jacobian (derivative of the nonlinear function) at each iteration.
- **Mystran's linearization** (from KVR DSP forums): evaluate tanh(x)/x at the previous sample's operating point, use this as a linear gain coefficient to solve the feedback equation analytically, then update. This effectively performs one Newton step with a very good initial guess.
- **Fixed-point/Picard iteration**: u_{k+1} = f(u_k) — no derivatives needed, but only linear convergence and only converges when |K · f'(v)| < 1 (contraction mapping condition). Often adequate for moderate resonance at high sample rates.

**Practical guidance for the three filters:**
- **Korg-35 (forward-path diodes):** The nonlinearity is in the forward path, not the feedback loop, so it can often be handled with a simple `tanh()` applied to the high-gain stage output without iterative solving. The feedback loop itself remains approximately linear.
- **MS-20 OTA (feedback-path diodes):** The diode nonlinearity in the feedback path creates a true implicit equation. Newton-Raphson with 2–3 iterations or Mystran's linearization works well.
- **Polivoks (slew-rate limiting inside integrators):** Each integrator's rate is bounded, making both integrator stages nonlinear. A per-integrator `tanh()` applied to the integrator input models the differential-pair saturation. For the ZDF formulation, iterate on the full two-integrator system.
- **Wasp (CMOS + OTA nonlinearities):** Multiple nonlinear elements at every stage. The CMOS inverter sigmoid at each amplifier stage plus OTA tanh at each frequency-control stage demands either iterative solving of the full system or a simplification that concentrates the dominant nonlinearities.

### Wave digital filter modeling

WDFs, rooted in Alfred Fettweis's 1970s theory, model circuits using **wave variables** a = v + Ri (incident) and b = v − Ri (reflected) instead of voltages and currents. Each circuit element has a scattering relation: a capacitor adapted with R = T/(2C) becomes a simple unit delay b[n] = −a[n−1]; a resistor matched to its port resistance gives b = 0 (complete absorption).

Elements connect via **adaptors** (series or parallel) that enforce KVL/KCL. The modularity is elegant: each component is a self-contained scattering block, and the tree structure propagates waves upward to a root where the nonlinearity is resolved, then reflects waves back down.

Kurt Werner's 2016 Stanford dissertation expanded WDFs to handle **arbitrary topologies** (non-series/parallel circuits, operational amplifiers, and multiple simultaneous nonlinearities) via:

- **R-type adaptors:** For circuits that cannot be decomposed into series/parallel trees, Werner derives scattering matrices from Modified Nodal Analysis (MNA). This handles the bridge and feedback topologies found in state-variable filters.
- **Grouped nonlinearities:** Multiple nonlinear elements are collected at the WDF tree root and solved jointly. For a single nonlinearity (e.g., one diode), the Shockley equation can be solved explicitly using the **Lambert W function**: b = a + 2RI_s − 2V_t · W(RI_s · exp((a + 2RI_s)/(2V_t)) / V_t). For multiple nonlinearities, the K-method or Newton-Raphson iteration is required.
- **OTA modeling in WDF:** Bogason and Werner (DAFx-17, Best Paper Award) showed how to incorporate OTAs as voltage-controlled current sources in the WDF framework — directly relevant to the MS-20 OTA revision and the Wasp's CA3080s.

**Advantages of WDF:** Inherent passivity guarantees stability for passive circuits; modular structure mirrors circuit topology; bilinear-transform discretization preserves energy properties. **Disadvantages:** Complex topologies require R-type adaptors with matrix inversions; multiple nonlinearities lose the explicit computation advantage; can be more computationally expensive than direct TPT implementations for simple circuits; less intuitive for engineers unfamiliar with wave-variable theory.

For the three target filters: WDF has been applied to the **Korg MS-50** (closely related to MS-20, by Rest/Parker/Werner, DAFx-17) but not published for the Polivoks or Wasp specifically. The Wasp's SVF topology with its multiple CMOS nonlinearities would require either grouped nonlinear solving at the root or a simplification.

### State-space and nodal analysis methods

The **DK method** (Yeh, Abel, Smith, IEEE TASLP 2010) provides an automated pipeline from circuit schematic to real-time DSP code. Starting from Modified Nodal Analysis, the circuit's nonlinear ODE is expressed in K-method form:

$$\dot{x} = Ax + Bu + Ci, \qquad i = f(v), \qquad v = Dx + Eu + Fi$$

where x = state variables (capacitor voltages, inductor currents), u = inputs, i = nonlinear device currents, v = controlling voltages, and f() is the nonlinear device characteristic. Discretizing with the trapezoidal rule and algebraic manipulation yields:

$$0 = f(p[n] + K \cdot i[n]) - i[n]$$

where **K = DHC + F** is the "K-matrix" and p[n] depends on previous states and current input. If this implicit function has a unique solution, it can be **pre-computed as a lookup table** g(p), making runtime execution explicit with zero iteration overhead.

For a **single nonlinearity** (1D K-matrix), the LUT is trivial to precompute. For two or three independent nonlinearities, 2D–3D tables are feasible. Beyond that, **runtime Newton-Raphson** is necessary, using the Jacobian J = (∂f/∂v) · K − I.

**Holters and Zölzer's generalized method** (EUSIPCO 2015) extends the DK approach with greater flexibility for arbitrary element models — directly applicable to the unusual components in these filters (programmable op-amps, CMOS inverters). Their **ACME.jl Julia framework** was used for the definitive Wasp VCF model (Köper et al., DAFx-2022), demonstrating the approach handles both extended MOSFET models and rail-dependent OTA behavior.

**The port-Hamiltonian approach** (Danish, Bilbao, Ducceschi, DAFx-21) applies to the Korg-35 specifically, guaranteeing zero-input stability via Lyapunov analysis and providing a **non-iterative** discrete-time scheme using discrete gradients and change-of-state variables — potentially cheaper than Newton-Raphson methods.

**Practical recommendation:** For the Wasp, the state-space method (Holters/Köper approach) with the extended MOSFET and rail-OTA models is the most rigorous published approach. For the Korg-35, Pirkle's TPT implementation or the port-Hamiltonian method offer good accuracy-to-cost ratios. For the Polivoks, a state-space SVF with slew-rate-limited integrators is the natural starting point, though no published model exists to validate against.

### White-box versus black-box: when to model the circuit and when to model the sound

**White-box (circuit-level) modeling** — WDF, state-space/DK, TPT/ZDF — requires the full schematic and component values. It provides physically meaningful parameter control (turn a virtual potentiometer and the correct circuit behavior follows), deterministic behavior, and handles component variation naturally. The cost is development complexity and potentially high CPU load: a complex nonlinear circuit with Runge-Kutta integration at 48 × 256 kHz sampling has been reported for some diode-heavy circuits.

**Black-box (data-driven) modeling** — LSTM/GRU networks (Wright et al., DAFx-19), temporal convolutional networks, WaveNet architectures — requires only input/output recordings. Parker, Esqueda, and Bergner (DAFx-19) demonstrated a deep neural network embedded in a discrete-time state-space framework using the **Korg MS-20 lowpass filter** as a test case, measured at 192 kHz. Black-box models excel at capturing the aggregate character of complex circuits from measurements alone, but they offer limited parameter control (each knob setting requires separate training or conditioning), unpredictable behavior outside training distribution, and no physical interpretability.

**Grey-box (hybrid) approaches** offer the most promising tradeoff for these three filters. **Differentiable white-box modeling** (Esqueda, Kuznetsov, Parker, DAFx-21) implements a white-box circuit model in a differentiable framework (PyTorch), then uses gradient descent to optimize component values against hardware measurements. This was the exact approach used for the Wasp VCF model's parameter fitting. **Wiener-Hammerstein models** (linear filter → static nonlinearity → linear filter) can capture the essential character with very low CPU cost, though they miss the interaction between nonlinearity and feedback that defines these filters' sound.

For an engineer choosing an approach: **start with white-box TPT/ZDF for the Korg-35 and Polivoks** (well-understood topologies, moderate nonlinearity), and **use the state-space method for the Wasp** (multiple interacting nonlinearities requiring systematic treatment). Reserve black-box/neural approaches for final "character matching" or for situations where the analog hardware is available for measurement but the circuit is too complex for analytical modeling.

### Oversampling and anti-aliasing for nonlinear filters

Every nonlinear operation inside a filter generates harmonics that extend beyond Nyquist and fold back as inharmonic aliasing. The severity depends on the nonlinearity's harshness: a soft tanh at moderate drive might generate harmonics rolling off at roughly 1/f, while the Polivoks's rail-to-rail clipping or the Korg-35's diode hard-clipping in the forward path generates much slower rolloff.

**Typical oversampling ratios in practice:**

- **2× oversampling + ADAA**: Adequate for soft saturation (tanh at moderate drive), Moog-style ladder with gentle limiting. Marginal for the MS-20 or Wasp.
- **4× oversampling**: The practical minimum for the Korg-35 (forward-path diode clipping) and Wasp (CMOS distortion at every stage). Good enough for most musical purposes.
- **8× oversampling**: Standard for high-quality emulations of aggressive filters. Developer reports from the plugin community ("pretty much lay waveforms on top of each other" at 8×) confirm this as the sweet spot for accuracy vs. cost.
- **16× oversampling**: For extreme accuracy or when the self-oscillation waveform's harmonic structure must be preserved precisely. Rarely necessary in production code.

**Antiderivative anti-aliasing (ADAA)**, introduced by Parker, Zavalishin, and Le Bivic (DAFx-16), can dramatically reduce aliasing from **memoryless** nonlinearities (like a standalone tanh waveshaper) without oversampling. First-order ADAA computes:

$$y[n] = \frac{F_1(x[n]) - F_1(x[n-1])}{x[n] - x[n-1]}$$

where F₁(x) = ∫f(x)dx is the first antiderivative of the nonlinear function. Second-order ADAA (Bilbao et al., IEEE SPL 2017) uses the second antiderivative for steeper spectral rolloff.

**The challenge for filter feedback loops:** ADAA introduces fractional-sample delay (0.5 samples for first-order, 1 sample for second-order), which shifts the resonant frequency of feedback systems. Holters (DAFx-19) proposed ADAA for stateful systems via global modification of state-space coefficient matrices. Albertini, Bernardini, and Sarti (DAFx-20) integrated ADAA into nonlinear WDFs. The most practical approach for these three filters is **ADAA applied to any memoryless nonlinearities outside the feedback loop** (e.g., input/output waveshapers) combined with **moderate oversampling (4×–8×) for the feedback loop's internal nonlinearities**. This synergistic combination is highly effective: ADAA steepens the spectral rolloff, making the oversampling's lowpass filter much more effective.

**Anti-aliasing filter design:** Use polyphase FIR filters for up/downsampling — they are SIMD-friendly and avoid the recursive dependencies of IIR filters. A moderate transition band with some treble rolloff is the practical compromise; razor-sharp cutoffs introduce Gibbs ringing.

### Nonlinear solver strategies for real-time constraints

The implicit equation at each sample — whether from ZDF feedback, K-method formulation, or WDF root scattering — must be solved within the audio callback's time budget. The choice of solver directly affects both accuracy and CPU cost.

**Newton-Raphson** provides quadratic convergence: the number of correct significant digits roughly doubles each iteration. For the scalar case (single nonlinearity):

$$i_{k+1} = i_k - \frac{f(p + K \cdot i_k) - i_k}{f'(p + K \cdot i_k) \cdot K - 1}$$

Using the **previous sample's solution as the initial guess** dramatically reduces iteration count due to temporal continuity. Typical counts at 4× oversampled 44.1 kHz:

- Mild nonlinearity (moderate resonance, moderate drive): **1–2 iterations**
- Strong nonlinearity (high resonance, heavy saturation): **3–5 iterations**
- Self-oscillation with signal present: **4–8 iterations** at transitions

**Fixed-point iteration** (i_{k+1} = f(p + K · i_k)) requires no derivative computation but converges only linearly. It converges only when |K · f'(v)| < 1 — a condition that can fail at high resonance. Useful as a fallback or for very mild nonlinearities.

**Lookup tables** exploit the DK method's structure: if the K-matrix is scalar (single nonlinearity), precompute g(p) for finely spaced p values and use cubic interpolation at runtime. For two independent nonlinearities, a 2D table is feasible (e.g., 1024 × 1024 with bilinear interpolation). Beyond 2–3 dimensions, memory grows exponentially and LUTs become impractical. A hybrid approach — **LUT for initial guess + 1 Newton refinement** — combines the best of both.

**Fast tanh() approximations** matter because tanh evaluations often dominate the per-sample cost. The most practical options:

- **Padé rational approximation**: tanh(x) ≈ x(27 + x²)/(27 + 9x²) for |x| < 3, clamped to ±1 outside. Max error ≈ 2.6%. Fully SIMD-vectorizable.
- **Reciprocal-sqrt sigmoid** (Andy Simper/Cytomic): f(x) = x / √(1 + x²) using hardware RSQRTSS/VRSQRTE with one Newton refinement. Within 2×10⁻⁴ of true tanh. ≈ 1 ns/sample on modern x86. Smooth, no clamping discontinuity.
- **Exponential-based**: Compute a = 6 + x(6 + x(3 + x)), then tanh ≈ (a − 6)/(a + 6). Uses third-order Taylor expansion of exp(x).

**When solvers fail to converge:** Audible clicks/pops from state discontinuities, buzzy artifacts from oscillation between two states, or runaway values leading to NaN. Practical mitigations: cap iterations at a fixed maximum (8 is typical), use the previous sample's solution as fallback, add a small damping factor to the Jacobian, and hard-limit state variable magnitudes.

### Faithfully reproducing self-oscillation

Self-oscillation occurs when loop gain exceeds unity at the resonant frequency. In analog circuits, thermal noise provides the initial excitation, and circuit nonlinearities provide amplitude limiting. Digital systems present two challenges: **zero self-noise** (no excitation source) and **solver stability** at the transition to oscillation.

**Excitation strategies:**

- Inject low-level noise at **−120 dBFS or below** into the filter input or feedback path — this mimics thermal noise and triggers self-oscillation naturally
- Inject a single-sample impulse on note-on events
- Initialize filter states to small nonzero values
- The ZDF/TPT approach has a natural advantage: the instantaneous feedback path responds correctly to self-oscillation without the half-sample phase error that plagues direct-form implementations (Stilson's observation that a 1-sample delay in a 4-pole ladder creates effectively a 5-pole filter)

**Amplitude limiting by filter type:**

- **Korg-35 (Rev 1)**: Forward-path diodes limit at ≈ 0.5 V / gain. Self-oscillation waveform is **soft-clipped with significant harmonics** — "a nice creamy fuzz, like a tube amp breaking up." Model with tanh(σ·x) in the high-gain stage.
- **MS-20 OTA (Rev 2)**: Three feedback-path diodes limit at ≈ 2.1 V. Cleaner self-oscillation, more whistling/howling character. Model with three series Shockley diode equations or a composite tanh with adjusted saturation voltage.
- **Polivoks**: No explicit limiting — rail clipping is the only mechanism. Self-oscillation goes **rail-to-rail producing trapezoidal/near-square waves** at high frequencies. Model with hard clip at ±V_rail after the integrator output. The trapezoidal shape (linear ramps from slew limiting + flat tops from rail clipping) is the defining signature.
- **Wasp**: OTA rail-dependent current behavior limits oscillation. The extended OTA model (f_M + f_H + f_L) from Köper et al. is essential — the simple tanh model produces unbounded oscillation. Model with the three-component OTA expression including rail transitions.

**Input-oscillation interaction** is a key musical feature: when an input signal is present during self-oscillation, the oscillation frequency can be **pulled** by strong harmonics near cutoff, amplitude modulation occurs between oscillation and signal, and intermodulation products appear. For the Korg MS-20 specifically: "the VCF will alter its resonance as the input amplitude varies — this was a 'flaw' in the original design, but is the main reason for the sound." This behavior emerges naturally from correct nonlinear modeling — it is not something that needs to be added separately.

---

## Part III: Practical implementation roadmap

### Recommended approach for each filter

**Korg-35 (Rev 1):** Start with Pirkle's TPT implementation using Zavalishin's Sallen-Key derivation with the **7/3 self-oscillation threshold**. Place tanh(σ·x) at the high-gain amplifier output to model forward-path diode clipping. Add the asymmetric resonance by modulating the effective integrator cutoff frequencies differentially based on signal polarity — a subtle refinement that captures the transistor interaction. The HPF variant must output the 6 dB/oct response (BPF ∥ HPF), not a true 12 dB/oct highpass. Oversample at **4×–8×** due to the forward-path distortion affecting all frequencies.

**MS-20 OTA (Rev 2):** TPT implementation with **self-oscillation at k₁k₂ = 2**. Place the diode model in the feedback path: either three series Shockley equations solved via Lambert W, or a composite tanh(x/V_composite) where V_composite ≈ 3 × 26 mV ≈ 78 mV equivalent. Add tanh saturation at each OTA input to model the differential-pair limiting. Newton-Raphson with 2–3 iterations resolves the feedback nonlinearity. Oversample at **4×**.

**Polivoks:** Implement as a ZDF SVF with **rate-limited integrators**. Each integrator's input is passed through a bounded function: either tanh(x/V_sat) scaled by SR (slew rate), or a hard-clipped ramp. Add rail clipping at ±V_rail on each state variable output. The I_set control parameter scales both ω_c and SR proportionally (since both depend on the programming current). For the aggressive "hard resonance" mode, omit additional amplitude limiting. For "soft resonance" (clone-style), add antiparallel diodes modeled with Shockley equation at the integrator outputs. Oversample at **4×–8×** due to the harsh clipping at high resonance.

**Wasp:** The most complex target. Use the **generalized state-space method** (Holters/Köper framework) with extended MOSFET models for all three CMOS inverter stages and extended rail-OTA models for both CA3080s. This requires solving a system with **5+ nonlinear elements simultaneously** at each sample — Newton-Raphson iteration on the full system, or a reduced-order approximation that concentrates the dominant nonlinearities. A simplified approach: standard ZDF SVF with sigmoidal (polynomial or tanh-like) waveshaping at each integrator input/output, plus diode limiting in the resonance path. The simplified version captures the gritty character without the full MOSFET-level accuracy. Oversample at **4×–8×**. The unipolar supply (signals centered at V_DD/2) must be accounted for in the signal scaling.

### CPU budget estimation and optimization priorities

At 44.1 kHz with 4× oversampling (effective 176.4 kHz), the per-sample budget for a single filter voice is roughly **5.7 μs**. With modern x86 at ≈ 3–4 GHz, this is approximately **17,000–23,000 clock cycles**. The dominant costs are:

- **tanh evaluations**: 5–8 per sample for a nonlinear SVF, ≈ 3–5 ns each with fast approximation → **15–40 ns**
- **Newton-Raphson iterations**: 2–4 per sample at ≈ 10–20 ns each → **20–80 ns**
- **Oversampling FIR filters**: 32–64 tap polyphase at ≈ 1–2 ns/tap → **32–128 ns**
- **State update and output mixing**: ≈ 5–10 ns

Total per voice: roughly **70–260 ns at 4× oversampling**, leaving ample headroom for polyphony. The bottleneck is typically the **oversampling filter**, not the nonlinear core. Optimize the FIR first (SIMD vectorization, minimum-phase design), then the tanh approximation, then the solver iteration count.

### Key papers every implementer should read

The essential reading list, in recommended order:

1. **Zavalishin, "The Art of VA Filter Design" (rev 2.1.2, 2020)** — The foundational text. Free PDF from Native Instruments. Covers TPT theory, SVF, Sallen-Key (Korg-35), ladder filters, and nonlinear extensions.

2. **Stinchcombe, "A Study of the Korg MS10 & MS20 Filters" (2006)** — 46-page definitive circuit analysis of both MS-20 revisions with transfer functions, SPICE validation, and the HPF 6 dB/oct proof.

3. **Köper, Holters, Esqueda, Parker, "A Virtual Analog Model of the EDP Wasp VCF" (DAFx-2022)** — The definitive Wasp model with extended MOSFET and rail-OTA equations, measured parameters, and validation.

4. **Simper, "SvfLinearTrapOptimised2" (Cytomic technical papers)** — The practical implementation reference for TPT/ZDF SVF with all outputs. Pair with "NonLinearStateSvfFilter.pdf" for nonlinear extensions.

5. **Yeh, Abel, Smith, "Automated Physical Modeling of Nonlinear Audio Circuits" (IEEE TASLP, 2010)** — The DK method for systematic circuit-to-code derivation.

6. **Werner, "Virtual Analog Modeling of Audio Circuitry Using Wave Digital Filters" (Stanford PhD, 2016)** — Comprehensive WDF theory including R-type adaptors and multiple nonlinearities.

7. **Parker, Zavalishin, Le Bivic, "Reducing the Aliasing of Nonlinear Waveshaping Using Continuous-Time Convolution" (DAFx-16)** — ADAA theory for anti-aliasing nonlinear functions.

8. **Pirkle, "Modeling the Korg35 Lowpass and Highpass Filters" (AES e-Brief 103, 2013)** — Direct implementation guide for MS-20 filter with C++ code available.

9. **Bogason, Werner, "Modeling Circuits with OTAs Using Wave Digital Filters" (DAFx-17)** — WDF OTA modeling directly applicable to the MS-20 OTA revision and Wasp.

10. **Danish, Bilbao, Ducceschi, "Port Hamiltonian Methods for the KORG35 and Moog 4-Pole VCF" (DAFx-21)** — Non-iterative, guaranteed-stable Korg-35 implementation.

### What measurement data tells us about character tuning

The subjective differences between these filters arise from measurable phenomena that should guide parameter tuning:

**The Korg-35 vs. OTA revision** difference reduces to *where* clipping occurs. Forward-path clipping (Korg-35) produces broadband harmonic enrichment ("creamy fuzz, like a tube amp") because all signal frequencies pass through the nonlinearity. Feedback-path clipping (OTA) produces frequency-selective distortion only near resonance ("clean howl, whistling"). This is the single most important distinction to capture, and it's trivially different in the DSP structure: place your tanh/diode model *before* or *after* the feedback summation node.

**The Polivoks's trapezoidal self-oscillation** is caused by slew-rate limiting, not amplitude clipping. If your self-oscillation waveform is a soft-clipped sine (rounded peaks), your model is wrong — it should show **linear ramps** (constant-slope edges) with **flat tops** (rail clipping). This is the telltale sign that the integrator model is correct. Test by setting resonance to maximum with no input and examining the output waveform on a scope: it should look trapezoidal at high cutoff frequencies and progressively more sinusoidal as cutoff decreases.

**The Wasp's "gritty dirt"** comes from the CMOS inverter nonlinearity being present at **every amplifier stage** (summing amp + two integrators), not just in the feedback path. The distortion onset is gradual — audible well below the clipping point, because the sigmoidal VTC introduces curvature at any signal level. If your Wasp model sounds clean at low levels and only distorts at high levels, the CMOS nonlinearity is likely modeled as a hard clipper rather than a polynomial sigmoid. The correct behavior is **always-present, level-dependent coloration** with no clean-to-dirty threshold.

## Conclusion

The three filters in this report represent three distinct categories of analog nonlinearity: **forward-path clipping** (Korg-35), **rate-limiting inside feedback** (Polivoks), and **distributed polynomial distortion** (Wasp). The DSP toolbox for handling them is mature — TPT/ZDF methods handle the first two elegantly with modest Newton-Raphson iteration, while the state-space/DK method provides the systematic framework the Wasp demands. The critical insight across all three is that the nonlinearity's *location* in the signal flow matters more than its *shape*: a tanh in the forward path sounds fundamentally different from the same tanh in the feedback path, and both sound different from a rate limiter inside an integrator. Getting the topology right — not just the nonlinear function — is what separates a convincing emulation from a generic "analog-style" filter.