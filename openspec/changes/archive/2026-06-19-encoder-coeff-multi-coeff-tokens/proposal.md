## Why

The eob > 1 trace brick composes a coded block from per-token accessors, but a few
needed tokens lack standalone accessors: the *coded* `all_zero` (symbol 0), a
parameterized `eob_pt_16` (eob = 2 needs symbol 1), and a parameterized
low-frequency `coeff_base_eob` (the eob = 2 AC uses context 1, not the single-DC
context 0). This change adds those reusable accessors so the consuming trace brick
stays small.

## What Changes

- Add `ENC-COEFF-MULTI-TOKENS` as a private `splot-encode` encoder-tool feature.
- Add a `coefficient_tokenization/multi_coeff.rs` submodule (to keep the parent file
  under the 1000-line source budget) with `pub(crate)`:
  - `coded_luma_all_zero_token(coeff_cdf_q_ctx)` — the coded luma `all_zero` (0).
  - `eob_pt_16_token(coeff_cdf_q_ctx, eob_ctx, symbol)` — a parameterized
    `eob_pt_16` (symbol 1 selects eob = 2).
  - `coeff_base_lf_eob_token(coeff_cdf_q_ctx, ctx, level)` — a parameterized
    low-frequency `coeff_base_eob` (symbol = level − 1).
- Wire the eob = 2 AC `coeff_base_eob` context (1) row into the generic
  `CoefficientTokenCdfRows` roundtrip router so the new tokens roundtrip through the
  in-tree AV2 § 8.2 coder.
- The accessors are available but not yet composed into a trace (the eob > 1 trace
  brick does that). No packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the multi-coefficient token accessors.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` (generic
  router + module wiring), `crates/splot-encode/src/coefficient_tokenization/multi_coeff.rs`
  (new), `crates/splot-encode/src/coefficient_tokenization_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, and
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none.
- Validator/CLI impact: none.
