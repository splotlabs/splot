## ADDED Requirements

### Requirement: Deblocking filter maximum width derivation

The repository SHALL provide a scheduler-free `splot-recon` derivation for the
AV2 § 7.17.3 deblocking filter-maximum-width process, tracked by
`RECON-DEBLOCK-FILTER-MAX-WIDTH`. The `deblock_filter_max_width(filter_size,
is_chroma, sb_edge) -> (max_width_neg, max_width_pos)` function SHALL set
`max_width_pos` to `1` for `filter_size <= 4`, `3` for `filter_size == 8`,
`is_chroma ? 4 : 6` for `filter_size == 16`, and `is_chroma ? 4 : 8` otherwise,
and SHALL set `max_width_neg` to `Min(max_width_pos, is_chroma ? 2 : 6)` when
`sb_edge` and to `max_width_pos` otherwise. It SHALL take `filter_size` (the
§ 7.17.4 maximum filter size) and `is_chroma` (the spec `plane != 0`) as
caller-resolved facts and SHALL be a total `const fn` with no error path. It SHALL
NOT implement the § 7.17.4 filter size, the § 7.17.5/§ 7.17.6 adaptive filter
strength, the § 7.17 edge traversal, the other loop filters, or runtime decode
wiring.

#### Scenario: Maximum width covers every spec branch

- **WHEN** `cargo test -p splot-recon deblock_filter --locked` runs
- **THEN** the test suite asserts `max_width_pos` for every `filter_size` bucket
  and both planes, and the super-block-edge `max_width_neg` cap, matching § 7.17.3
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation
