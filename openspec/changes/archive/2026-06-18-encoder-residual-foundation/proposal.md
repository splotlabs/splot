## Why

The encoder can now validate borrowed input frames and plan minimal headers, but
it has no typed way to compute the signed source-minus-prediction residual that
future transform and quantization stages will consume. This boundary should land
before forward transform work so arithmetic, geometry, zero-copy, and error
contracts are proven independently.

## What Changes

- Add an `ENC-RESIDUAL-FOUNDATION` matrix row for private encoder residual
  calculation.
- Add a private `splot-encode` residual module that computes checked row-major
  signed residual blocks from a borrowed input plane and caller-provided
  prediction samples for the current 8-bit YUV420 input surface.
- Validate plane/block geometry, prediction length, sample range, and output
  sizing before returning residual data.
- Use explicit signed intermediate/storage types and focused tests for zero,
  min/max, clipping-boundary, checkerboard, gradient, odd-edge, stride, and
  mismatch cases.
- Update the encoder roadmap/gap audit and generated matrix status views.
- Do not add forward transforms, quantization, coefficient syntax, packet output,
  CLI success, or a public encoder conformance claim.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `encoder-tools`: define the private residual-foundation contract for
  `ENC-RESIDUAL-FOUNDATION` as an encoder-policy arithmetic stage before forward
  transform, quantization, and packet output.

## Impact

- Affected crates: `crates/splot-encode`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature/status coverage docs, `docs/ENCODER-ROADMAP.md`, and
  `docs/ENCODER-GAP-AUDIT.md`.
- No new dependencies, no dependency graph change, no public packet output, no
  validator diagnostics, and no changes to `splot-core` or `splot-recon`.
