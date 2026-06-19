## Why

Multi-coefficient blocks (eob > 1) require the §8.3.2 `coeff_base` low-frequency
context, which is derived from a neighbour-sum of the already-decided `Level[]`
magnitudes — unlike the position-only `coeff_base_eob` context used by the
single-DC bricks. This change adds the encoder-side `coeff_base_lf` LF luma
context derivation as an isolated, unit-tested primitive (the hardest piece of the
multi-coefficient path), so the eob > 1 trace brick that consumes it stays small.

## What Changes

- Add `ENC-COEFF-BASE-LF-CONTEXT` as a private `splot-encode` encoder-tool feature.
- Add `pub(crate)` `coeff_base_lf_luma_context(...)` in `coefficient_tokenization`,
  mirroring the decoder's `CoeffBaseContext` low-frequency luma branch
  (`SIG_REF_DIFF_OFFSET` neighbour-sum with the LF `magLimit`, `ctx = (mag+1)>>1`,
  and the §8.3.2 LF luma context mapping). It imports the shared
  `splot_core::tables::conversion::SIG_REF_DIFF_OFFSET` table.
- The primitive is total and panic-free (saturating geometry, slice-bounds-guarded
  reads contribute 0), scoped to low-frequency LUMA coefficients (parity-hidden DC
  and chroma are out of scope and documented).
- It is loaded but unread (no caller yet); the eob > 1 trace brick will consume it.
- No new CDF, no token emission, no packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the §8.3.2 `coeff_base` low-frequency
  luma context derivation.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` (+ sibling
  tests).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none new; imports an existing `splot-core` § 9 table.
- Validator/CLI impact: none.
