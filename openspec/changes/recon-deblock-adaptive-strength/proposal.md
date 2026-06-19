## Why

The § 7.17.7.1 deblocking sample filter (`RECON-DEBLOCK-SAMPLE-FILTER`) takes the
threshold `q_thr` as a caller-resolved fact, and the § 7.17.3 width derivation
(`RECON-DEBLOCK-FILTER-MAX-WIDTH`) covers the widths. The § 7.17.5 adaptive filter
strength process — which derives `qThr` and the `side` threshold from the filter
level — is the remaining self-contained piece of the deblock filter's parameter
derivation.

## What Changes

- Add Feature ID `RECON-DEBLOCK-ADAPTIVE-STRENGTH`.
- Add `deblock_side_threshold_index(lvl, bit_depth) -> usize` (the § 7.17.5
  `qInd = Clip3(0, MAX_SIDE_TABLE - 1, lvl - 24 * (BitDepth - 8))`) and
  `deblock_adaptive_filter_strength(lvl, side_threshold, bit_depth) -> (qThr,
  side)` to `crates/splot-recon/src/deblock_filter.rs`.
- Compute `qThr = Round2(get_q(lvl, 0), QUANT_TABLE_BITS) >> 6` via the existing
  § 7.14.2 `quantizer_value`, and `side = Max(side_threshold + (1 << (12 -
  BitDepth)), 0) >> (13 - BitDepth)`.
- Take `lvl` (the § 7.17.6 filter level) and `side_threshold = Side_Thresholds[qInd]`
  as caller-resolved facts (`Side_Thresholds` lives in `splot-core`'s § 9.2
  tables, which `splot-recon` cannot reach; the caller indexes it via
  `deblock_side_threshold_index`).
- Keep the index helper a total `const fn` and the strength function total and
  panic-free (the quantizer lookup is total and the bit-depth shifts are
  positive); no new error variant.
- Preserve the current runtime `splot decode` behavior and all output bytes.
- Add tests: a `qInd` clip test over both bit depths and a strength test pinning
  the `side` arithmetic by hand and the `qThr` composition against
  `quantizer_value`.
- Update the implementation matrix, decoder support matrix, roadmap, generated
  status/coverage docs, the decoder-conformance-coverage group, and the crate and
  module `//!` docs.

Non-goals:

- No § 7.17.6 filter-level selection (segment/qindex state), no § 7.17.7.2 filter
  choice, no § 7.17 edge traversal, no other loop filters, no runtime wiring, no
  dependency-graph change.

## Capabilities

### Modified Capabilities

- `decoder-support`: add a supported row for the § 7.17.5 adaptive filter
  strength derivation.

## Impact

- Affected code: `crates/splot-recon/src/deblock_filter.rs`,
  `crates/splot-recon/src/lib.rs`, `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  status/coverage docs, and `xtask/src/decoder_conformance_coverage.rs`.
- Public API impact: two additive `pub` functions; no breaking changes.
- Diagnostics impact: none.
- Dependencies and licensing: no new dependencies and no licensing changes.
