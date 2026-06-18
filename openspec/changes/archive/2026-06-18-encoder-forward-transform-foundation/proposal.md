## Why

The encoder can now compute checked signed residual blocks, but it has no typed
forward-transform handoff for later quantization and closed-loop reconstruction.
The first transform slice should prove the arithmetic boundary without taking on
full transform selection, coefficient tokenization, or packet output.

## What Changes

- Add an `ENC-FORWARD-TRANSFORM-FOUNDATION` matrix row for a private encoder
  forward-transform primitive.
- Add a private `splot-encode` forward-transform module for the current minimal
  4x4 DCT_DCT DC-only subset over uniform residual blocks.
- Use explicit checked arithmetic and a documented no-op quant/dequant test
  path through `splot-recon` inverse transform APIs.
- Reject unsupported non-uniform input and wrong block sizes with typed encoder
  errors instead of silently producing partial coefficients.
- Update the encoder roadmap/gap audit and generated matrix status views.
- Do not add a direct `splot-tables` dependency, broad transform families,
  quantization policy, coefficient tokenization, tile-body emission, packet
  output, CLI success, or Baseline Encoder Profile v1 claims.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `encoder-tools`: define the private forward-transform foundation contract for
  `ENC-FORWARD-TRANSFORM-FOUNDATION` as an encoder-policy arithmetic stage after
  residual calculation and before quantization.

## Impact

- Affected crates: `crates/splot-encode`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature/status coverage docs, `docs/ENCODER-ROADMAP.md`, and
  `docs/ENCODER-GAP-AUDIT.md`.
- No new dependencies, no dependency graph change, no public packet output, no
  validator diagnostics, and no changes to `splot-core`, `splot-recon`,
  `splot-decode`, `splot-validate`, or `splot-cli`.
