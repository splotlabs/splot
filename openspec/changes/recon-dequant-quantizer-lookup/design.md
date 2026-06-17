## Context

`splot-recon` is the scheduler-free reconstruction-primitive crate. It already
provides intra prediction primitives but no residual path. The documented next
reconstruction frontier is dequantization, inverse transforms, and residual
addition (`docs/DECODER-ROADMAP.md`). AV2 § 7.14.2 defines the quantizer-value
lookup that every dequantized coefficient depends on. It is pure, table-driven,
and independent of entropy decoding, so it is a clean first residual-path brick.

## Goals / Non-Goals

Goals:

- Implement the AV2 § 7.14.2 quantizer-value lookup core exactly:
  `Ac_Qlookup`, `qlookup`, the § 6.4.1 Table 6.3 `MaxQ`, and `get_q`.
- Keep the primitive total and panic-free for all inputs.
- Keep `splot-recon` scheduler-free and free of other `splot-*` dependencies.

Non-Goals:

- `get_qindex` (segment / `delta_q` index resolution), `get_dc_quant` /
  `get_ac_quant` composition, the § 7.14.4 dequantization process,
  quantizer-matrix weighting, the § 7.14.3 reconstruct process, inverse
  transforms, residual addition, runtime decode, or reference refresh.

## Decisions

- **Lift `get_qindex` to the caller.** The § 7.14.2 `get_qindex` process reads
  frame, segment, and `delta_q` state that `splot-recon` deliberately does not
  hold. `quantizer_value` therefore takes the already-resolved quantizer index
  (the `get_qindex` output, in `0..=MaxQ`) and the signed per-plane delta that
  `get_dc_quant` / `get_ac_quant` would add. This keeps the primitive a pure
  function over caller-resolved inputs, matching the existing intra-prediction
  primitives.
- **`MaxQ` from `BitDepth`.** AV2 § 6.4.1 Table 6.3 binds `MaxQ` to the bit
  depth: 8-bit → `MAXQ_8_BITS` (255), 10-bit → `MAXQ_10_BITS` (303);
  `bit_depth_idc` greater than 1 is reserved, so [`BitDepth`] already models
  only those two cases. `MAXQ_BITS` (351) is the bit-depth-agnostic
  segmentation-feature ceiling, not this clamp, so it is intentionally not used
  here.
- **Total, panic-free arithmetic.** `qlookup` is module-private and only ever
  called by `quantizer_value` with `q` already clamped to `1..=MaxQ` (at most
  303), so the `<<` shift (at most 12) cannot overflow `u32`. `quantizer_value`
  computes `qindex + delta` in `i64` and clamps to `1..=MaxQ` before the
  lookup, so even out-of-contract `qindex` / `delta` extremes clamp rather than
  overflow or panic. This satisfies the workspace no-panic library rule without
  a typed error path (there is no invalid input to reject — the spec clamps).

## Risks / Trade-offs

- Lifting `get_qindex` means callers must resolve the quantizer index before
  calling. This is the correct boundary: index resolution belongs to the
  frame/segment quantization rows, and the follow-on dequantization-process row
  will compose this lookup with that resolution.

## Migration Plan

Additive only. No existing API changes, no dependency-graph changes, and the
runtime `splot decode` behavior is unchanged.

## Open Questions

None.
