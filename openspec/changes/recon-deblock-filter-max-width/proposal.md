## Why

The § 7.17.7.1 deblocking sample filter (`RECON-DEBLOCK-SAMPLE-FILTER`) takes the
per-side widths `maxWidthNeg` / `maxWidthPos` as caller-resolved facts. The
§ 7.17.3 filter-maximum-width process that derives those widths from the filter
size, plane, and super-block-edge flag is a self-contained, table-free branching
derivation — the natural companion that completes the sample filter's width
inputs.

## What Changes

- Add Feature ID `RECON-DEBLOCK-FILTER-MAX-WIDTH`.
- Add `deblock_filter_max_width(filter_size, is_chroma, sb_edge) -> (max_width_neg,
  max_width_pos)` to `crates/splot-recon/src/deblock_filter.rs`.
- Implement § 7.17.3: `maxWidthPos` is `1` for `filter_size <= 4`, `3` for `8`,
  `is_chroma ? 4 : 6` for `16`, and `is_chroma ? 4 : 8` otherwise; `maxWidthNeg`
  is `Min(maxWidthPos, is_chroma ? 2 : 6)` at a super-block edge and `maxWidthPos`
  otherwise.
- Take `filter_size` (the § 7.17.4 max filter size, a caller-computed `Min` of
  the boundary transform dimensions) and `is_chroma` (the spec `plane != 0`) as
  caller-resolved scalars.
- Keep it a total `const fn` with no error path; add a module-level
  `const`-evaluated spec contract.
- Preserve the current runtime `splot decode` behavior and all output bytes.
- Add a branch-coverage test over every `filter_size` bucket, both planes, and
  the super-block-edge cap.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate and
  module `//!` docs.

Non-goals:

- No § 7.17.4 filter size (a caller-side `Min`), no § 7.17.5/§ 7.17.6 adaptive
  filter strength (segment/qindex state), no § 7.17 edge traversal, no other loop
  filters, no runtime wiring, no dependency-graph change.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.17.3 filter-maximum-width
  derivation.

## Impact

- Affected code: `crates/splot-recon/src/deblock_filter.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: one additive `pub const fn`; no breaking changes.
- Diagnostics impact: none.
- Dependencies and licensing: no new dependencies and no licensing changes.
