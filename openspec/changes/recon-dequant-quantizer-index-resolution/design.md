## Context

`RECON-DEQUANT-QUANTIZER-LOOKUP` shipped the § 7.14.2 `get_q` lookup core in
`crates/splot-recon/src/dequant.rs` and explicitly deferred `get_qindex` and the
per-plane `get_dc_quant` / `get_ac_quant` composition. This change completes the
§ 7.14.2 surface. `splot-recon` is scheduler-free and may not depend on any other
`splot-*` crate, so every input is a recon-local plain type and library code is
panic-free.

## Goals / Non-Goals

Goals:

- Implement § 7.14.2 `get_qindex`, `get_dc_quant`, and `get_ac_quant` exactly,
  composing with the shipped `quantizer_value`.
- Keep the functions total and panic-free.

Non-Goals:

- Segmentation evaluation / `FeatureData` / `CurrentQIndex` maintenance, the
  § 7.14.4 dequantization process, quantizer-matrix weighting, the § 7.14.3
  reconstruct process, inverse transforms, residual addition, runtime decode, or
  reference refresh.

## Decisions

- **Lift segmentation and `delta_q` evaluation to the caller.** `get_qindex`'s
  `seg_feature_active_idx`, `FeatureData[ segmentId ][ SEG_LVL_ALT_Q ]`, and the
  running `CurrentQIndex` are frame/segment/tile state that `splot-recon` does
  not hold. `quantizer_index` therefore takes the already-resolved facts
  (`segment_alt_q_active`, `segment_alt_q_data`, `base_q_idx`,
  `current_q_index`, `delta_q_present`, `ignore_delta_q`) and reproduces the
  three § 7.14.2 branches. Only the segment-feature branch clamps
  (`Clip3(0, MaxQ, ...)`); the inactive branches return `current_q_index` or
  `base_q_idx` unclamped, exactly as the spec specifies, because the caller
  maintains those in range.
- **`QuantizerDeltas` carrier.** `get_dc_quant( plane )` / `get_ac_quant( plane )`
  select one of the per-plane delta sums the spec adds to the quantizer index.
  Modeling those five caller-resolved sums (`y_dc`, `u_dc`, `v_dc`, `u_ac`,
  `v_ac`; luma AC is 0 per spec) as a small recon-local struct keeps the
  `plane` parameter meaningful and the API faithful to the spec's named
  functions, while the per-plane delta derivation (`DeltaQ*` + `Base*DeltaQ`)
  stays a frame-quant concern owned by the caller.
- **Reuse `quantizer_value`.** `dc_quantizer` / `ac_quantizer` forward the
  selected delta and resolved index to the shipped `get_q` implementation, so
  there is one source of truth for the lookup and clamp.
- **Total, panic-free.** All index arithmetic uses `i64` intermediates before
  `Clip3`, so any caller value clamps rather than overflowing.

## Risks / Trade-offs

- Lifting segmentation/`delta_q` resolution means the caller must compute those
  facts. This is the correct boundary: it belongs to the frame/segment
  quantization rows, and the § 7.14.4 process row will compose this resolution
  with coefficient and quantizer-matrix handling.

## Migration Plan

Additive only. No existing API changes, no dependency-graph changes, and the
runtime `splot decode` behavior is unchanged.

## Open Questions

None.
