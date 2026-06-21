## 1. Implementation
- [x] 1.1 Emit interleaved EOB `coeff_br` (constant ctx 0/7, symbol mag-5) for EOB
      magnitude 1..=7; non-EOB stays 1..=4 (position-aware validation).
- [x] 1.2 Extend `recover_quant_from_tokens` to read the interleaved `coeff_br`.
- [x] 1.3 Add the routed `CoeffBrLf` ctx-7 + `CoeffBaseLf` 4x4 ctx-3 CDF rows and a
      reusable `coeff_br_lf_token`.

## 2. Tests
- [x] 2.1 eob=1 DC magnitude 5/6/7 matches the existing single-DC tokens (ctx 0).
- [x] 2.2 eob=2 AC (EOB) magnitude > 4 emits `coeff_br` ctx 7; asymmetric roundtrip
      + recover == input.
- [x] 2.3 Reject EOB magnitude > 7 and non-EOB magnitude > 4.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-COEFF-BR` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
