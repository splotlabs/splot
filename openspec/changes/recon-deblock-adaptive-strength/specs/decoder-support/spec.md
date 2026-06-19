## ADDED Requirements

### Requirement: Deblocking adaptive filter strength derivation

The repository SHALL provide scheduler-free `splot-recon` derivations for the AV2
§ 7.17.5 adaptive filter strength process, tracked by
`RECON-DEBLOCK-ADAPTIVE-STRENGTH`. The `deblock_side_threshold_index(lvl,
bit_depth) -> usize` function SHALL return `Clip3(0, MAX_SIDE_TABLE - 1, lvl - 24 *
(BitDepth - 8))` (with `MAX_SIDE_TABLE = 296`) as a total `const fn`, and the
`deblock_adaptive_filter_strength(lvl, side_threshold, bit_depth) -> (qThr, side)`
function SHALL return `qThr = Round2(get_q(lvl, 0), QUANT_TABLE_BITS) >> 6` (via
the § 7.14.2 quantizer-value lookup) and `side = Max(side_threshold + (1 << (12 -
BitDepth)), 0) >> (13 - BitDepth)`, where `lvl` is the § 7.17.6 filter level and
`side_threshold` is the caller-resolved `Side_Thresholds[qInd]`. Both SHALL be
total and panic-free and SHALL NOT implement the § 7.17.6 filter-level selection,
the § 7.17.7.2 filter choice, the § 7.17 edge traversal, the other loop filters,
or runtime decode wiring.

#### Scenario: Adaptive filter strength matches the spec

- **WHEN** `cargo test -p splot-recon deblock_filter --locked` runs
- **THEN** the test suite covers `deblock_side_threshold_index` clipping over both
  bit depths, the hand-pinned `side` arithmetic, and the `qThr` composition
  against the independently-tested `quantizer_value`
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation
