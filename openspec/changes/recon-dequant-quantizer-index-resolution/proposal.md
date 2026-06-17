## Why

`RECON-DEQUANT-QUANTIZER-LOOKUP` shipped the AV2 § 7.14.2 quantizer-value
lookup core (`get_q`) but deliberately deferred the rest of § 7.14.2:
`get_qindex` quantizer-index resolution and the per-plane `get_dc_quant` /
`get_ac_quant` composition. Those are small, pure, and scheduler-free, and they
complete the § 7.14.2 surface so the later § 7.14.4 dequantization process only
has to supply coefficients and quantizer-matrix weighting. This is the
documented next residual-path step.

## What Changes

- Add Feature ID `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`.
- Extend `crates/splot-recon/src/dequant.rs` with:
  - `quantizer_index` implementing § 7.14.2 `get_qindex( ignoreDeltaQ, segmentId )`
    over caller-resolved frame and segment facts.
  - `QuantizerDeltas`, a carrier for the caller-resolved per-plane DC and AC
    delta sums.
  - `dc_quantizer` and `ac_quantizer` implementing § 7.14.2 `get_dc_quant( plane )`
    and `get_ac_quant( plane )` by selecting the plane delta and applying the
    existing `quantizer_value`.
- Keep all functions total and panic-free with `i64` clamp intermediates,
  reading no frame, segment, or tile state.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No segmentation evaluation (`seg_feature_active_idx`, `FeatureData` array) or
  `CurrentQIndex` maintenance — those stay with the caller.
- No § 7.14.4 dequantization process, quantizer-matrix weighting, or § 7.14.3
  reconstruct process.
- No inverse transforms, residual addition, tile-syntax decode, runtime decode
  output, hashes, Y4M, or reference refresh.
- No `splot-decode -> splot-recon` dependency change and no scheduler state in
  `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.14.2 quantizer-index
  resolution and per-plane quantizer composition while broader reconstruction
  (the § 7.14.4 dequantization process, inverse transforms, and residual
  addition) remains partial.

## Impact

- `crates/splot-recon/src/dequant.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-ROADMAP.md`
- `openspec/specs/decoder-support/spec.md`
