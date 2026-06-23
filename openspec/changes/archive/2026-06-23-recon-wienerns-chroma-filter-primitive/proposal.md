## Why

The repository already has the AV2 §7.20.3 luma Wiener NS filter primitive and
the §7.20.2 source-sample helpers, but chroma Wiener NS filtering remains
explicitly unsupported. The ac0ej3 loop-restoration path reaches chroma
Wiener NS unit syntax, so the next safe reconstruction brick is the
scheduler-free chroma sample math before any runtime wiring or output claim.

## What Changes

- Add Feature ID `RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE` to the implementation
  and decoder support matrices.
- Extend `splot-recon` with an additive AV2 §7.20.3 chroma Wiener NS primitive
  over caller-supplied chroma source samples, caller-supplied luma source
  samples, and caller-resolved coefficients.
- Transcribe the AV2 §7.20.3 `Wiener_Ns_Config_Uv` tap table and the
  `Wiener_Filters_420` luma downsampling table used by `get_luma_sample`.
- Keep §7.20 traversal, §7.20.2 frame source reads, coefficient selection,
  temporal/reference Wiener state, runtime wiring, and ac0ej3 output out of
  scope.

## Capabilities

### New Capabilities

- `recon-wienerns-chroma-filter-primitive`: AV2 §7.20.3 chroma non-separable
  Wiener per-block/per-sample filter primitive, including the chroma tap loop
  and the luma-tap contribution over caller-resolved source callbacks.

### Modified Capabilities

- `decoder-support`: Track `RECON-WIENERNS-CHROMA-FILTER-PRIMITIVE` as partial
  loop-restoration reconstruction progress without claiming runtime decode
  wiring, full Wiener NS filtering, or successful ac0ej3 decode.

## Impact

Affected areas: `crates/splot-recon` Wiener NS filtering, recon error/tests,
public `splot-recon` exports, implementation/decoder support matrices, and
generated status/coverage docs. No dependency graph, licensing, encoder, CLI,
runtime output, oracle fixture, or successful decode claim changes.
