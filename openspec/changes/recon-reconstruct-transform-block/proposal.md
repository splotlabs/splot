## Why

The residual-math stack is complete as separate `splot-recon` primitives — the
§ 7.14.4 dequantization process, the § 7.15.4 inverse transform, and the § 7.14.3
residual-addition step — but a decoder runs them as a fixed three-step chain to
turn a transform block's decoded quantized coefficients into reconstructed
samples. The encoder closed loop already hand-composes exactly this sequence;
centralizing it as a tested `splot-recon` primitive gives the decoder the
residual *sink* (the consumption end of the coefficient pipeline) it currently
lacks, in the correct dependency order: the sink exists before the coefficient
loop has somewhere to send a decoded `Quant[]`.

## What Changes

- Add Feature ID `RECON-RECONSTRUCT-TRANSFORM-BLOCK`.
- Add `crates/splot-recon/src/reconstruct_block.rs` with
  `reconstruct_transform_block_residual<T: ReconSample>(prediction, quant,
  dequant_params, transform, dequant_scratch, residual_scratch, out)`.
- Compose the chain `out = Clip1(prediction + inverse_transform(dequant(quant)))`:
  § 7.14.4 `dequantize_block` → § 7.15.4 `inverse_transform_2d_outer` → § 7.14.3
  `reconstruct_add_residual`, over caller-resolved dequantization and transform
  parameters (resolve `transform` with `InverseTransform2dOuter::resolve`).
- Take caller-owned `dequant_scratch` (`adjW * adjH`) and `residual_scratch`
  (`origW * origH`) working buffers so the composition allocates nothing.
- Keep it total and panic-free: every buffer-length or geometry inconsistency is
  rejected by the underlying primitive's typed `ReconError` before `out` is
  mutated; no new error variant is added.
- Preserve the current runtime `splot decode` behavior and all hash/raw/Y4M
  output bytes (a pure `pub` composition with no runtime rewiring).
- Add focused tests: all-zero-preserves-prediction and uniform signed nonzero-DC
  residual at TX_4X4 and TX_64X64 (the latter exercising adjusted-to-original
  sample duplication), plus a fail-atomic inconsistent-buffer rejection.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate
  `//!` docs.

Non-goals:

- No coefficient entropy decode that produces `Quant`, no § 7.15.3 secondary
  transform, no § 7.14.4 `useQm` / shift derivation, no § 7.15.4 DPCM-direction
  selection, no prediction sample production, no wiring into the runtime decode
  path, no output, no reference refresh, no dependency-graph change, and no
  AVM/dav2d invocation.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the transform-block reconstruction
  residual chain.

## Impact

- Affected code: `crates/splot-recon/src/reconstruct_block.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `pub fn` in `splot-recon`; no breaking changes.
- Diagnostics impact: none; existing minimal runtime diagnostics and output bytes
  remain unchanged.
- Dependencies and licensing: no new dependencies and no licensing changes.
