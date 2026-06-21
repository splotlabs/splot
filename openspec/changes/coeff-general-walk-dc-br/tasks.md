## 1. Implementation
- [x] 1.1 Add `coeff_br_lf_luma_context` mirroring the decoder `CoeffBrContext::ctx`
      via the first-`num` `SIG_REF_DIFF_OFFSET` neighbours.
- [x] 1.2 Emit the non-EOB DC `coeff_br` (interleaved, derived ctx, symbol mag-5);
      lift the non-EOB limit to 7; extend recovery.
- [x] 1.3 Add the routed `CoeffBrLf` ctx-1/ctx-3 CDF rows.

## 2. Tests
- [x] 2.1 `coeff_br_lf_luma_context` unit tests (specific Level[] -> ctx).
- [x] 2.2 Non-EOB DC magnitude > 4 emits the derived `coeff_br`; asymmetric roundtrip.
- [x] 2.3 Both coefficients carry `coeff_br`; magnitude 8 still rejected.

## 3. Tracking
- [x] 3.1 Add the `ENC-COEFF-GENERAL-WALK-DC-BR` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
