## Why

The encoder now has private residual, forward-transform, fixed-quantization, and
coefficient-tokenization stages, but it has never composed them into a single
decoder-visible closed loop. Nothing yet proves that the encoder's own
quantized decisions reconstruct to exactly the samples a conforming AV2 decoder
would produce. That closed loop is the core correctness invariant for every
later coded frame, so it must exist and be evidenced before any tile-body or
packet path is allowed to publish output.

## What Changes

- Add `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` as a private `splot-encode`
  encoder-tool feature.
- Add a minimal closed-loop reconstruction module for the existing top-left
  8-bit luma 4x4 DCT_DCT DC-only uniform-block subset.
- Compose the decoder-visible pipeline using `splot-recon` for all
  decoder-visible math: AV2 §7.13.2.10 DC intra prediction (no-neighbor
  midpoint), §7.14.4/§7.14.2 dequantization, §7.15.4 inverse transform, and
  §7.14.3 reconstruct (residual addition with clip), then freeze the result into
  a `splot-recon` current-frame workspace and compute its decoded-frame hash.
- Keep the encoder-policy stages (residual, forward transform, quantization) in
  `splot-encode` and never duplicate decoder-visible math in the encoder.
- Add an independent structured test proving the emitted coefficient decisions
  (the existing tokenization roundtrip) decode back to the exact quantized
  coefficient the closed loop reconstructs from, and that for the lossless
  qindex-zero flat subset the reconstruction equals the source.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for a minimal private closed-loop
  reconstruction over the current top-left 8-bit luma 4x4 DCT_DCT DC-only
  encoder subset.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none; this uses the existing `splot-core` and `splot-recon`
  dependencies only.
- Validator/CLI impact: none; no coded packets or public encoder success path.
