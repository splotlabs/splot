## Why

The block-symbol trace has covered single-coefficient blocks (eob = 1) across the
whole luma DC magnitude vocabulary. This change composes the first MULTI-coefficient
block (eob = 2), combining the three merged building blocks (the §8.3.2 `coeff_base`
low-frequency context derivation, the non-EOB `coeff_base` token, and the
multi-coefficient token accessors) into one §8.2-roundtrip-proven trace — the first
block whose `coeff_base` CDF context is data-dependent (derived from a neighbour's
`Level[]`).

## What Changes

- Add `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` as a private `splot-encode` encoder-tool
  feature.
- Add `compose_minimal_intra_two_coeff_block_trace()` in `block_symbol_trace`: the
  §5.20.5.3 mode prefix, then the coded luma `residual()` for an eob = 2 block (one
  nonzero AC of level 1 at scan pos 1, a zero DC at scan pos 0): `all_zero=0`,
  `eob_pt_16=1` (eob 2), the base pass (AC `coeff_base_eob` at context 1, DC
  `coeff_base` at the `Level[]`-derived §8.3.2 low-frequency context 1), the AC
  `sign_bit` bypass, then all-zero U and V `txb_skip`. The DC context is derived via
  `coeff_base_lf_luma_context` (not hard-coded).
- Add the eob = 2 AC `coeff_base_lf_eob` (context 1) and DC `coeff_base_lf`
  (context 1, TCQ-off) rows to `BlockSymbolTraceCdfRows` with their routing.
- Prove the ten-token trace `[0,0,0, 0, 1, 0, 0, 0, 1, 1]` roundtrips through one
  in-tree AV2 §8.2 coder and that the DC token carries the derived context.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the first multi-coefficient (eob = 2) block
  trace.

## Impact

- Affected code: `crates/splot-encode/src/block_symbol_trace.rs` (+ sibling tests).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none.
- Validator/CLI impact: none; no coded packets.
