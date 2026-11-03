# Pilot: lossy port modes with the frequency domain solver (`--lossy --fd`)

CST 2025.5, frequency domain solver, tetrahedral mesh, `ModesOnly`, `ConsiderPortLossesTet`; copper
5.8e7 S/m, εr and tanδ at 1 GHz. About 5 s per frequency. Checks at 1 GHz:

| case | α CST | expected | Z, εeff against the time domain (lossless port modes) |
|---|---|---|---|
| `anchor_forsten_suite_4` (copper only, tanδ 0) | 0.073 Np/m (0.64 dB/m) | αc 0.083 Np/m (Wheeler's incremental inductance with kisi's field solver, strip and ground) | Z +0.24 %, εeff +0.8 % |
| `ms_u2_er4.4` (tanδ 0.02) | 0.468 Np/m (4.07 dB/m) | αc 0.150 (Wheeler) + αd 0.338 (closed form) = 0.488 Np/m | Z +0.3 %, εeff +1.2 % |

The shift of Z and εeff is mostly the conductors' internal inductance, absent with lossless port modes:
ΔL/L = R/(ωL) ≈ 0.5 % and 0.8 %, i.e. Z +0.24 % / +0.4 %, εeff +0.5 % / +0.8 %; about 0.3 % in εeff
remains. α is 4–12 % below the Wheeler estimate (port mesh at the strip edges, to be checked).
