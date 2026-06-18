## Why

The encoder now has private residual, forward-transform, and fixed-quantization
stages, but it still cannot turn quantized coefficients into the ordered entropy
token facts needed by the future tile-body writer. This change adds the next
private, non-emitting bridge so coefficient syntax can be tested before packet
output exists.

## What Changes

- Add `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` as a private `splot-encode`
  encoder-tool feature.
- Add a minimal coefficient-tokenization module for the existing top-left
  neutral-spatial-context 4x4 DCT_DCT DC-only quantized-coefficient subset.
- Derive scan order, EOB, begin-position metadata, sign/magnitude token facts,
  coefficient CDF q-context, and ordered entropy-token records for AV2
  §5.20.7.27 / §5.20.7.28.
- Prove token-to-range-byte-to-symbol-decode roundtrips through the in-tree AV2
  §8.2 `splot-core` symbol encoder/decoder using scoped default CDF rows,
  including the low-frequency EOB base CDF for the DC coefficient.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for minimal private coefficient
  tokenization over the current top-left 4x4 DCT_DCT DC-only encoder subset.

## Impact

- Affected code: `crates/splot-encode` internals and tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none.
- Dependency impact: none; this uses existing `splot-core` and `splot-recon`
  dependencies.
- Validator/CLI impact: none; no coded packets or public encoder success path.
