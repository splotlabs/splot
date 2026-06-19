## Why

The finite-q golomb tail covers luma DC magnitude 8..=17 (the `q < cMax` path).
Magnitude 18+ uses the §5.20.7.28 `read_quant` golomb-*prefix* path (`q == cMax`):
the q_length unary saturates at `cMax` zeros, then an exp-golomb `golomb_length`
unary selects the `coeff_rem` width. This change completes the single-coefficient
luma DC magnitude vocabulary.

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` as a private `splot-encode`
  encoder-tool feature.
- Add a parameterized `compose_intra_dc_golomb_prefix_block_trace(magnitude,
  negative)` covering magnitude 18..=525 (golomb `length` 2..=8): the mode prefix,
  the fixed golomb level tokens, the luma `dc_sign` CDF token, then the
  golomb-prefix bypass bits — `cMax` (5) `q_length` zeros, the `golomb_length`
  unary (`golomb_zeros` zeros + a terminating 1, `length = golomb_zeros + k`), and
  `coeff_rem` as one `L(length)` literal — then all-zero U/V `txb_skip`.
- Encode `x = magnitude - maxLevel` (`x >= 10`): `length = GetMsb(x - 6)`,
  `golomb_zeros = length - k`, `coeff_rem = (x - 6) - (1 << length)`,
  `xBase = 6 + (1 << length)`.
- Reject magnitudes outside 18..=525 with the typed
  `BlockSymbolTraceGolombMagnitudeOutOfRange` error (a runtime check, not a
  release-stripped assertion).
- Prove the trace roundtrips through one §8.2 coder and that the decoded
  golomb-prefix bits reconstruct each encoded magnitude via the decoder's
  `read_quant` golomb-prefix arithmetic.
- Preserve the current no-packet-output invariant.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the §5.20.7.28 golomb-prefix tail of a
  coded luma DC coefficient.

## Impact

- Affected code: `crates/splot-encode` internals and tests (`block_symbol_trace`).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; the module is crate-private and not re-exported.
- Dependency impact: none.
- Validator/CLI impact: none; no coded packets or public encoder success path.
