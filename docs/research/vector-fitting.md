# Compact channel models for the SPICE export – vector fitting and the alternatives

Status: research note with a recommendation (25 October 2026, branch `feat/vector-fit`), written before the
implementation; section 8 records what the implementation changed (the lossless lines are LC ladders, not the
simulators' line elements). ROADMAP block 3, step 3 of the channel export (`channel-export.md` section 6). The
study script and its data: `validation/channel_export/compact_study.py`, `line_models.json`.

## Summary
- **Problem:** the export writes each line section as an RLGC ladder of π sections (20 per wavelength at the top
  frequency) with fitted Foster/Debye loss networks in every section. ThunderScope's 85 mm LVDS pair SYNC_OUT_P is
  about 38 k elements; a PRBS transient takes 6–15 minutes in ngspice and LTspice.
- **What a compact model must keep:** passivity (it is connected to arbitrary passive ends), causality, the exact
  delay, the DC resistance, and syntax that ngspice, Xyce and LTspice all read.
- **Methods reviewed:** vector fitting (VF) and its relaxed and fast variants, fixed-pole fitting with sign
  constraints (what `export::fit` does), delayed rational fitting (DVF), the method of characteristics with fitted
  characteristic admittance and propagation function (Branin, Marti, the universal line model, HSPICE's W element),
  Bergeron cells (lossless lines with lumped losses), Loewner and matrix-pencil identification, and passivity
  checks and enforcement (Hamiltonian test, residue perturbation by quadratic programming).
- **Study on kisi's own line models** (four models of the example boards, 10 and 70 mm, up to 25 GHz):
  - lossless-line cells – ideal delay lines carrying L∞ and C∞, the rest of Z(s) and Y(s) as lumped passive
    networks between them, 3 cells per (delay × f max) – follow the exact line within |ΔS| 0.004–0.010
    (exact lumps), better than today's ladder (0.03–0.11, its own discretisation);
  - the method of characteristics with Yc and the delay-free propagation function fitted (16 + 6…10 poles)
    reaches |ΔS| 0.004–0.04 in the band, but every fit is non-passive outside the band (largest singular value of
    S up to 4000 at 40–250 GHz) and its DC resistance is off by 10–700×, once negative: 1 − H(0) ≈ γl → 0 makes the
    DC resistance an ill-conditioned ratio of two fitted quantities.
- **SPICE facts that decide the realisation:** ngspice's lossless `T` element stalls in a cascade of short lines
  (breakpoint control: a 40-cell chain did not finish 20 ns of PRBS in 120 s; with relaxed breakpoint control it
  diverged to megavolts), while the SPICE3 lossy line LTRA (`O`) in its lossless case ran the same chain in 0.08 s
  and gives the exact line in AC/S-parameter analysis. LTspice reads LTRA too (and LTRA with R = 0 is the documented
  workaround for a `T` bug in LTspice 26.0.1). Xyce lists both line devices as not supported in `.AC` (known defect
  970-SON) – its S-parameter deck needs the ladder; its transient reads them.
- **Recommendation:** for the export's line sections, **lossless-line cells**: per line section a few cells
  (at least 3 per delay × f max), each a lossless line (L and C from the fit's high-frequency terms) with the
  section's fitted Foster and Debye networks (without their sL and sC terms) lumped once – passive by construction
  (positive R, L, C), the DC resistance exact, no controlled sources, no fit of a delayed response. Consecutive
  sections with the same model (the many sub-millimetre pieces of a coupled run) are merged first; vias, parts and
  stubs stay as they are; pairs keep their modal blocks (cells per mode). Vector fitting is not used for the
  board's lines; it stays the method for tabulated data (a Touchstone block behind a connector), where DVF with
  passivity enforcement would be needed.
- **As implemented** (section 8): the lossless line of each cell is an LC ladder (40 π sections per wavelength at
  f max, 2 elements each; phase within 0.1 % at f max) instead of an LTRA line – ngspice's LTRA bounds the time step
  to the shortest line delay and went unstable on a real lane, `T` stalls, Xyce runs neither in `.AC`. The loss
  networks (~40 elements per cell) dominate the count either way: 2.0–4.6 × fewer elements than the ladder at
  25 GHz (up to 5.9 × at 5 × Nyquist), PRBS benches 7–17 × faster in ngspice (SYNC_OUT_P 46 s instead of 480 s;
  24 s instead of 397 s at 5 × Nyquist), the same S-parameters as the ladder in the band
  (`channel-export.md` section 7).

## 1. The problem in numbers
| Lane | length | elements (R/L/C/sources) | ngspice | source |
|---|---|---|---|---|
| SYNC_OUT_P (LVDS pair) | 85 mm | 18753/7439/11448/200 | PRBS7 bench 348 s, LTspice ~6–15 min | `channel-export.md` 2b |
| PCIe lanes (pairs) | 30–60 mm | 12–25 k | S-parameters 15–73 s | `channel-export.md` 4 |

A lower top frequency saves only ~45 % (`--fmax profile`): the count stops falling below ~5 GHz because every
section keeps at least one ladder section with its full loss networks (`channel-export.md` 2b). An RLGC section
of the SYNC_OUT_P export holds about 40 elements: a Foster chain of 9–10 R∥L terms and 9–10 Debye R–C branches.

## 2. Rational fitting
- **Vector fitting** (Gustavsen and Semlyen 1999 [VF]): fits f(s) ≈ Σ rₖ/(s − pₖ) + d + s·e by iterating two
  linear least-squares problems – a scaling function σ(s) with the same poles relocates the poles to the zeros of σ,
  then the residues follow for fixed poles. Unstable poles are flipped, so the result is stable by construction;
  passivity is not. **Relaxed VF** (Gustavsen 2006 [RVF]) replaces σ(∞) = 1 by a mild normalisation and relocates
  poles faster and more robustly (also for noisy data and when a delay is being optimised). **Fast VF**
  (Deschrijver et al. 2008 [FVF]) solves the pole step of a multiport with common poles from QR-reduced blocks (60
  ports: 44 s instead of hours). scikit-rf implements VF with a passivity test and enforcement [SKRF].
- **Fixed poles with sign constraints:** with poles fixed on a logarithmic grid, the residue step alone is linear;
  constraining the residues to be non-negative (non-negative least squares) gives R∥L Foster and R–C Debye networks
  whose elements are all positive – passive and causal by construction. This is `export::fit` today (R(f), L(f)
  of the conductor, G(f), C(f) of the dielectric, per metre). It works because the per-metre impedance and
  admittance of a line are RL and RC functions (poles and zeros on the negative real axis); it cannot fit a
  response with a delay or resonances.
- **Loewner framework** (Mayo and Antoulas 2007 [LOE]): builds a state-space model directly from frequency samples
  by tangential interpolation; the order follows from the singular values of the Loewner pencil. No iteration and
  good for many ports, but neither stability nor passivity is built in (post-processing as for VF).
- **Matrix pencil / GPOF** (Hua and Sarkar 1990 [MP]): fits damped exponentials to time samples (an impulse
  response); same caveats, and band-limited data make the impulse response itself an approximation.

**Delays:** a response with a delay τ needs about two poles per ripple, i.e. ~2·τ·f max poles – the earlier study
fitted a 574 ps lane with 32 poles and could not make it passive (`channel-export.md` section 6). Taking the delay
out of S21 alone does not help a reflecting lane, because the delay sits in S11 as well (the round trip).

## 3. Delay-preserving models
- **Delayed rational fitting** (DVF/DSK, Chinea, Triverio and Grivet-Talocia 2010 [DVF]): each response as a sum
  of a few delayed terms e^{−sτᵢ} times rational functions with common poles; delays from a time-frequency (Gabor)
  analysis of the data; stable and causal by construction. Passivity is not enforced in that work (left for later);
  the SPICE synthesis uses RC cells and delayed controlled sources or matched ideal lines. Per-element or per-mode
  delays suit long multiport interconnects measured or simulated as a whole.
- **Method of characteristics** (Branin 1967 [BRA]): a line is, at each end, its characteristic admittance Yc
  in parallel with a current source fed by the wave arriving from the other end, delayed:
  I₁ = Yc·V₁ − H·(Yc·V₂ + I₂), I₂ = Yc·V₂ − H·(Yc·V₁ + I₁), with H = e^{−γl}. With frequency-dependent
  parameters Yc(s) and H(s) are fitted: **Marti** (1982 [MAR]) with real poles and a delay; the **universal line
  model** (Morched, Gustavsen and Tartibi 1999 [ULM]) in the phase domain with modal delays and VF; HSPICE's W element
  is built on the same idea (Kuznetsov and Schutt-Ainé [KUZ]). Size: per line section two Yc networks, two delays
  and two fitted H functions – about 100 elements, independent of the length. Yc is an RC-type admittance and can be
  fitted passively; H with its controlled sources cannot be made passive by construction.
- **Bergeron cells:** a lossless line (exact delay, exact characteristic impedance) with the loss lumped at its
  ends or middle – EMTP's constant-parameter model with R/4, R/2, R/4 (Dommel 1969 [DOM]; e.g. [MW]). With
  frequency-dependent lumped elements and the line split into cascaded cells this follows a frequency-dependent
  line over a wide band (Torrez Caballero, Costa and Kurokawa 2016 [BC]). Size grows with the electrical length
  (cells per delay × f max), not with the number of pieces.

## 4. Passivity checks and enforcement
- **Definitions** (Triverio et al. 2007 [TRI]): a passive model is stable and causal; for scattering data the
  condition is ‖S(jω)‖₂ ≤ 1 at every frequency (and analytic in the right half-plane), for admittances a
  positive-semidefinite Hermitian part. A model that is accurate but not passive can make a circuit of passive ends
  unstable (a pole pair moves into the right half-plane), and a passive model cannot follow non-passive data closer
  than the data's own violation.
- **Checks:** dense frequency sweeps miss narrow violations between samples and outside the band; the
  **Hamiltonian test** (Boyd, Balakrishnan, Kabamba; Grivet-Talocia 2004 [HAM]) finds all crossings of the singular
  values through 1 (or of the eigenvalues of Re Y through 0) as the imaginary eigenvalues of one matrix built from
  the state-space model – exact for rational models, but not directly applicable to models with delays.
- **Enforcement:** perturb the residues by quadratic programming so that the eigenvalues of Re Y are positive at
  the violating frequencies, iterating on a linearisation (Gustavsen and Semlyen 2001 [QP]); perturb the
  Hamiltonian's imaginary eigenvalues off the axis (Grivet-Talocia 2004 [HAM]); or perturb the residue matrices'
  eigenvalues (Gustavsen, fast enforcement [FPE]). All assume small violations and a good initial fit, enforce
  passivity only where they sample (QP) or only for rational models (Hamiltonian), and cost accuracy. scikit-rf's
  enforcement diverged on kisi's 60-pole lane fit (`channel-export.md` section 6).

## 5. What the SPICE engines can implement
| | ngspice | Xyce | LTspice |
|---|---|---|---|
| lossless line `T` | yes; breakpoint control stalls in cascades (below) | yes (TRA); not in `.AC` (970-SON) | yes; overshoot bug in 26.0.1 [LTB] |
| LTRA `O` (constant R, L, G, C; lossless case R = G = 0) | yes, exact in AC; time step ≤ 0.8 × the shortest delay, unstable on a lane with lumped losses (section 8) | yes; not in `.AC` (970-SON); LC case may run out of memory (1923-SRN) | yes (R = 0 is the workaround for the T bug [LTB]) |
| constant-parameter lossy lines (TXL, CPL, URC) | yes | partly | URC |
| frequency-dependent RLGC line (W element) | no | no | no |
| Laplace / s-domain source | XSPICE `s_xfer` code model | not documented | `Laplace=` on E/G sources [LTL] |
| delayed controlled source | no (a matched line) | `YDELAY` device [XRN] | in B sources |
| R, L, C, E, F, G, V | yes | yes | yes |

Sources: ngspice's devices as `ngspice -p` lists them (`devhelp`: Tranline, LTRA with steplimit/nosteplimit
options, TransLine (TXL), CplLines (CPL), URC, the XSPICE blocks) and the manual's chapter 6 (transmission lines)
[NG]; Xyce release notes 7.3–7.5 (defects 970-SON, 1923-SRN; the YDELAY device) [XRN]; LTspice [LTL], [LTB].
Consequences: a portable compact netlist uses only R, L, C, E, F and a line element; Laplace sources are not
portable; Xyce's `.AC` cannot run any line element, so an S-parameter deck for Xyce must use the ladder.

**ngspice's `T` in a cascade** (`compact_study.py --ngspice`, ngspice 42): 40 cells of an 11 ps line with lumped
losses, 20 ns of PRBS at 30 ps steps –

| delay element | run time | result |
|---|---|---|
| `T` (default breakpoint control) | not finished after 120 s | – |
| `T … REL=100 ABS=100` | 0.46 s | diverges (2·10⁶ V instead of 2 mV) |
| LTRA, lossless case (`O` with R = G = 0) | 0.08 s | correct |

LTRA's lossless case is also exact in ngspice's AC and S-parameter analysis (checked against e^{−jωτ}).

## 6. Study on kisi's line models
`validation/channel_export/compact_study.py`: four line models of the example boards (`line_models.json`, per-metre
R, L, G, C from `rf::LineModel::rlgc`, ThunderScope with formula impedances), 10 mm and 70 mm, against the exact
line (cosh/sinh of γl) up to 25 GHz, 50 Ω.

**Lossless-line cells** (exact lumped losses, cell = half line – z – y – half line; N = ⌈c · delay · f max⌉):

| line model | length | c = 1 | c = 2 | c = 3 | c = 4 | ladder, 20/λ |
|---|---|---|---|---|---|---|
| stripline odd mode (SYNC_OUT_P) | 10 mm | 0.079 | 0.017 | 0.0065 | 0.0027 | 0.035 |
| | 70 mm | 0.32 | 0.32 | 0.0086 | 0.0035 | 0.104 |
| stripline even mode (SYNC_OUT_P) | 70 mm | 0.38 | 0.33 | 0.0092 | 0.0038 | 0.106 |
| microstrip (PCIe_PET1_P) | 10 mm | 0.036 | 0.028 | 0.0062 | 0.0019 | 0.029 |
| | 70 mm | 0.097 | 0.069 | 0.0039 | 0.0017 | 0.112 |
| microstrip (REFINOUT2) | 70 mm | 0.105 | 0.067 | 0.0041 | 0.0018 | 0.114 |

The cells are periodic: their lumped losses reflect coherently at the Bragg frequency c · f max / 2, so c = 2
puts it at f max (the 0.32 above) and c = 3 at 1.5 × f max. One series z and one shunt y per cell is as good as the
symmetric z/2–y–z/2 (0.0069 vs 0.0065 on the 10 mm odd mode) with fewer elements; lumps at the cell ends instead
of the middle are 1.6× worse. The LC ladder's error is its own discretisation (the phase of the π ladder lags by
0.4 % at f max), which the cells do not have – the ideal lines carry the reactive part exactly.

**Method of characteristics** (Yc: 16-pole RC form by non-negative least squares; H e^{sτ}: relaxed VF):

| line model | length | Hm poles | max \|ΔS\| to 25 GHz | largest singular value of S, 1 Hz–250 GHz | R_dc model / line |
|---|---|---|---|---|---|
| stripline odd mode | 10 mm | 10 | 0.0097 | 4319 (62 GHz) | 11.2 Ω / 0.078 Ω |
| | 70 mm | 10 | 0.0038 | 0.9999 | 16.5 Ω / 0.54 Ω |
| stripline even mode | 70 mm | 10 | 0.0047 | 0.9999 | 9.0 Ω / 0.54 Ω |
| microstrip (PCIe) | 10 mm | 10, fitted from 1 kHz | 0.011 | 153 (64 GHz) | −16.7 Ω / 0.021 Ω |
| | 70 mm | 10 | 0.019 | 723 (187 GHz) | 4.2 Ω / 0.15 Ω |
| microstrip (REFINOUT2) | 70 mm | 10 | 0.018 | 43 (109 GHz) | 3.0 Ω / 0.14 Ω |

In the band the characteristic model is as accurate as the cells with c ≈ 3, but (1) outside the band the fitted
H has |H| > 1 somewhere and 1 − H² passes near zero – the 2-port is strongly non-passive at 40–250 GHz; (2) at DC
the series resistance is 2/[Yc(0)·(1 + H(0))/(1 − H(0))], where Yc(0) → 0 and 1 − H(0) → 0 as √f: a ratio of two
fitted small numbers, wrong by 10–700× and once negative. Both can be fought (fit to 10 × f max, constrain the
asymptotes and H(0), enforce passivity with a delayed-model test), but none of it is by construction, and an error
in either quantity can make a lane unstable with passive ends.

## 7. Recommendation for kisi's export (as written before the implementation)
**Lossless-line cells per line section**, as an export option (`--model compact`, the ladder stays the default
until the compact model is proven):
1. The line section's fits are the ladder's (`fit_z`, `fit_y` of kisi's `LineModel::rlgc` per metre). Their sL and
   sC terms become an ideal line per cell: L∞ = the Foster network's L, C∞ = the Debye network's C, Z = √(L∞/C∞),
   delay = length · √(L∞C∞) / N. The rest – R0 and the R∥L terms, G0 and the R–C (and microstrip's R–L–C)
   branches – is the lumped z and y of each cell, scaled to the cell length. Every element is a positive R, L or C or
   a lossless line: passive and causal by construction; the delay is that of the ideal lines plus what the lumps
   add (the internal inductance, the dispersion), as in kisi's model; the DC resistance is the sum of the R0.
2. N = ⌈3 · delay · f max⌉ cells (Bragg frequency at 1.5 × f max), laid out as half line – (z, y, line) × (N − 1) –
   z, y, half line.
3. Consecutive line sections of one line on one layer with (nearly) the same model – and consecutive coupled runs –
   are merged into one section with the length-weighted mean of their R(f), L(f), G(f), C(f): SYNC_OUT_P's coupled
   run consists of ~80 pieces of 0.01–3 mm with the same cross-section. (Merging matters for any compact method: the
   characteristic model would otherwise cost ~100 elements per piece.)
4. A section shorter than one cell (delay < 1/(3 f max)) stays a lumped π ladder, so that no line element shorter
   than a cell limits the simulator's time step (ngspice and LTspice keep the step near or below a line's delay).
5. The ideal line is written as an LTRA line in its lossless case: `O<n> a ref b ref <model>` with
   `.model <model> LTRA(R=0 L=… G=0 C=… LEN=1)` – the same line in ngspice, Xyce and LTspice, fast in ngspice. Xyce's
   `.AC` cannot run it (970-SON): the Xyce S-parameter deck of a compact export is not written; the README says so.
6. Vias, parts, stubs and pads unchanged (vias are short: their ladders hold their resonances and stubs); pairs keep
   their modal blocks, each mode with its own cells. Touchstone files stay kisi's S-parameters; the KiCad project
   uses the same library.

Expected size: SYNC_OUT_P's ~70 mm coupled run at 25 GHz becomes ~34 cells per mode of ~40 elements, about
2.7 k elements instead of ~30 k; at `--fmax profile` (2.5 GHz) 4 cells per mode. The vias (a few hundred elements
each) then dominate.

**Why not vector fitting here:** the per-metre networks are already fitted passively (fixed poles, positive
residues), and the cells add the delay exactly without any fit of a delayed response. Vector fitting of the whole
lane needs ~2·τ·f max poles and passivity enforcement that fails on kisi's lanes; the characteristic model
(Yc and H fitted per section) is compact but not passive by construction and ill-conditioned at DC (section 6).
**Where vector fitting will be needed:** for tabulated data – a connector or cable given as Touchstone, the channel
behind a connector as a SPICE block (ROADMAP block 3) – DVF with common poles (relaxed, fast VF), a passivity test
on a dense grid including far outside the band, and residue-perturbation enforcement are the state of the art
([DVF], [QP], [FPE]).

## 8. What the implementation changed
Building `--model compact` (`kisi_si::export`, `El::Cells`) and running it in ngspice on ThunderScope's lanes changed
three things:

1. **No simulator line element for the lossless part.** LTRA in its lossless case passed the AC/S-parameter
   checks and the 40-cell chain, but on a whole lane: (a) ngspice keeps the time step below 0.8 × the shortest
   line's delay – SYNC_OUT_P's PRBS bench at 25 GHz needed 65 618 time points (3.9 ps) and 255 s, hardly faster than
   the ladder; the model flag `NOSTEPLIMIT` lifts the bound but the waveform is wrong (stuck at 0.156 V after 10 ns);
   (b) with the cells of `--fmax profile` (2.5 GHz, ~100 ps per cell) the bench went unstable – "timestep too small"
   at 7.3–7.8 ns with 2·10⁷ V, with linear, quadratic or mixed interpolation, `NOCONTROL`, trapezoidal or Gear
   integration alike –, while the same circuit with the LTRA lines replaced by LC ladders ran cleanly; a plain chain
   of the same lines with simple lumps was stable, so it is the combination with the lane's loss networks and mode
   converters. `T` failed there too ("timestep too small" at 0.3 ns). A model that works in one run and diverges in
   the next cannot be exported. The cells' lossless lines are therefore **LC ladders**: 40 π sections per
   wavelength at f max (phase within 0.1 % there; the RLGC ladder has 20 and 0.4 %), 2 elements per section –
   cheap next to the ~40 elements of a cell's loss networks. Every simulator reads them (Xyce's `.AC` included),
   nothing bounds the time step, the S-parameter decks of all three simulators stay.
2. **Cell layout and count.** With no line element there is no reason to avoid half-length lines: the layout is
   half line – (z, y, line) × (N − 1) – z, y – half line, one lump per cell (as accurate as z/2–y–z/2, section 6).
   N = ⌈3 · delay · f max · max(1, √(r/0.03))⌉, where r is the lumps' size against the line's reactance and
   susceptance weighted by (f/f max)² (the discretisation error goes with r·(ωτ_cell)²): stripline lumps are
   r ≈ 0.04 (skin effect and Djordjevic-Sarkar), microstrip's 0.06–0.10, because a passive line can carry only the
   capacitance at infinite frequency and the dispersion's rise must be lumped (the fit's high-frequency C is ~10 %
   below C(f) on the test board's microstrip). Every section gets at least one cell (no ladder fallback).
3. **Passivity beyond the band** is checked on the circuit itself: kisi evaluates the exported circuit's S-matrix
   from 1 kHz to 10 × f max by star products of scattering matrices (cascade matrices of long ladders overflow far
   above their cut-off) – largest singular value ≤ 1 on every lane, ladder and compact alike – and `compare.py
   --passivity` repeats that in ngspice up to 5 × f max (beyond the RLGC ladder's cut-off near 6.4 × f max
   ngspice's own AC solution of an 800-section ladder breaks down numerically: singular values of 10⁴⁰ at 200 GHz
   for SYNC_OUT_P, where the star products give 0.99998). Asymptotically both are passive by construction (positive R, L, C, and
   the ideal mode converters are lossless).

Results on the example boards: `channel-export.md` section 7.

## Sources
- [VF] B. Gustavsen, A. Semlyen, “Rational approximation of frequency domain responses by vector fitting”, IEEE
  Trans. Power Delivery 14(3), 1052–1061, 1999 – https://www.sintef.no/globalassets/project/vectfit/vf_paper.pdf
- [RVF] B. Gustavsen, “Improving the pole relocating properties of vector fitting”, IEEE Trans. Power Delivery 21(3),
  1587–1592, 2006 – https://www.sintef.no/globalassets/project/vectfit/relaxed_vf.pdf
- [FVF] D. Deschrijver, M. Mrozowski, T. Dhaene, D. De Zutter, “Macromodeling of multiport systems using a fast
  implementation of the vector fitting method”, IEEE MWCL 18(6), 383–385, 2008 –
  https://www.sintef.no/globalassets/project/vectfit/fast_vf_paper.pdf
- [SKRF] scikit-rf `VectorFitting` – https://scikit-rf.readthedocs.io/en/v0.23.0/api/generated/skrf.vectorFitting.VectorFitting.html
- [LOE] A. J. Mayo, A. C. Antoulas, “A framework for the solution of the generalized realization problem”, Linear
  Algebra Appl. 425, 634–662, 2007 – https://portal.mardi4nfdi.de/entity/Q998198
- [MP] Y. Hua, T. K. Sarkar, “Matrix pencil method for estimating parameters of exponentially damped/undamped
  sinusoids in noise”, IEEE Trans. ASSP 38(5), 814–824, 1990
- [DVF] A. Chinea, P. Triverio, S. Grivet-Talocia, “Delay-based macromodeling of long interconnects from
  frequency-domain terminal responses”, IEEE Trans. Adv. Packag. 33(1), 246–256, 2010 –
  https://iris.polito.it/bitstream/11583/1857935/2/jnl-2010-tadvp-skdelays-ieeexplore.pdf
- [BRA] F. H. Branin, “Transient analysis of lossless transmission lines”, Proc. IEEE 55(11), 2012–2013, 1967
- [MAR] J. R. Marti, “Accurate modelling of frequency-dependent transmission lines in electromagnetic transient
  simulations”, IEEE Trans. PAS-101(1), 147–157, 1982 – https://circle.ubc.ca/handle/2429/23403
- [ULM] A. Morched, B. Gustavsen, M. Tartibi, “A universal model for accurate calculation of electromagnetic
  transients on overhead lines and underground cables”, IEEE Trans. Power Delivery 14(3), 1032–1038, 1999
- [KUZ] D. Kuznetsov, J. E. Schutt-Ainé, R. Mittra, “Optimal transient simulation of distributed lines”, IEEE
  Multi-Chip Module Conf., 164–169, 1995 – https://experts.illinois.edu/en/publications/optimal-transient-simulation-of-distributed-lines/
- [DOM] H. W. Dommel, “Digital computer solution of electromagnetic transients in single- and multiphase networks”,
  IEEE Trans. PAS-88(4), 388–399, 1969; [MW] its lumped-loss Bergeron line as implemented in Simscape Electrical –
  https://au.mathworks.com/help/sps/powersys/ref/distributedparametersline.html
- [BC] P. Torrez Caballero, E. C. Marques Costa, S. Kurokawa, “Frequency-dependent line model in the time domain for
  simulation of fast and impulsive transients”, Int. J. Electr. Power Energy Syst. 80, 179–189, 2016 –
  https://repositorio.unesp.br/items/2b9c5518-3fbf-405f-be74-f904fd64aeb8/full
- [TRI] P. Triverio, S. Grivet-Talocia, M. S. Nakhla, F. G. Canavero, R. Achar, “Stability, causality, and passivity
  in electrical interconnect models”, IEEE Trans. Adv. Packag. 30(4), 795–808, 2007 –
  https://iris.polito.it/retrieve/handle/11583/1662802/48869/jnl-2007-tadvp-Fundamentals-ieeexplore.pdf
- [HAM] S. Grivet-Talocia, “Passivity enforcement via perturbation of Hamiltonian matrices”, IEEE TCAS-I 51(9),
  1755–1769, 2004 – https://iris.polito.it/bitstream/11583/1401334/2/jnl-2004-cas-Hamiltonian-ieeexplore.pdf
- [QP] B. Gustavsen, A. Semlyen, “Enforcing passivity for admittance matrices approximated by rational functions”,
  IEEE Trans. Power Systems 16(1), 97–104, 2001 – https://www.sintef.no/globalassets/project/vectfit/references/passivity.pdf
- [FPE] B. Gustavsen, “Fast passivity enforcement of rational macromodels by perturbation of residue matrix
  eigenvalues” – https://www.sintef.no/globalassets/project/power-systems/publications/fast_passivity_enforcement_of_rational_macromodels_by_perturbation_of_residue_matrix_eigenvaluesic1.pdf
- [NG] ngspice manual, chapter 6 (transmission lines: T, LTRA, URC, TXL, CPL), section 12.2.18 (s-domain transfer
  function) – https://ngspice.sourceforge.io/docs/ngspice-html-manual/manual.xhtml (the device list and parameters
  as `ngspice -p` / `devhelp` of ngspice 42 prints them)
- [XRN] Xyce release notes 7.3–7.5: known defects 970-SON (TRA and LTRA not in AC), 1923-SRN (LC lines out of
  memory), the YDELAY device – https://xyce.sandia.gov/files/xyce/Release_Notes_7.3.pdf,
  https://www.sandia.gov/app/uploads/sites/155/2025/07/Release_Notes_7.5.pdf
- [LTL] Analog Devices, “Model transfer functions by applying the Laplace transform in LTspice” –
  https://www.analog.com/en/resources/technical-articles/model-transfer-functions-by-applying-the-laplace-transform-in-ltspice.html
- [LTB] LTspice 26.0.1: overshoot of a terminated lossless line, LTRA with R = 0 as the workaround –
  https://ez.analog.com/design-tools-and-calculators/ltspice/f/q-a/602625/ltspice26-0-1-bug-terminated-tl-y2-produces-overshoot/595207
