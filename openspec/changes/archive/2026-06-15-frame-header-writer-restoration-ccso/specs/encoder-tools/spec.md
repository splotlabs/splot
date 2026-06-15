# encoder-tools delta: frame-header-writer-restoration-ccso

## ADDED Requirements

### Requirement: frame-header loop-restoration and CCSO writers

`splot-core` SHALL provide writers that are the exact inverse of the `lr_params()` (§ 5.18.7.11)
and `ccso_params()` (§ 5.18.7.12) parsers on the intra path, plus the `tu(mx)` truncated-unary
writer primitive (§ 4.11.9) the CCSO writer needs. For every model the writer accepts, reparsing
the written bits with the corresponding parser and the same gating inputs SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility, a behavior-preserving tool-table extraction, and the new `write_tu`
primitive) and SHALL never panic: a model the parser could not have produced SHALL be rejected with
a typed writer error before any bit is written.

Because the frame-level Wiener-bank decode (`read_wienerns_filter()`) is unmodeled — the parser
*stops* before it rather than completing — a complete `lr_params()` model can never carry
`frame_filters_on == true`. The loop-restoration writer SHALL reject any such model and SHALL write
the `frame_filters_on == false` surface (the `tool_index` reverse-lookup and the `LoopRestorationSize`
size-shift reversal). The CCSO writer SHALL reproduce the per-plane `ccso_offset_idx` loop
byte-exactly from the modeled values.

#### Scenario: each restoration/CCSO structure round-trips across every branch

- **WHEN** a parsed `lr_params()` (a `Parsed` outcome) or `ccso_params()` structure is written with
  the same gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch (the
  disabled returns, the per-plane tool selection and `LoopRestorationSize` size signaling for each
  `SbSize`, the CCSO single-picture / frame-flag / `ccso_bo_only` / quant-step inferences, the
  `ccso_offset_idx` loop, and `NumPlanes` 1 vs 3).

#### Scenario: an unwritable or non-reproducible model is rejected before any bit

- **WHEN** an `lr_params()` model carries a plane with `frame_filters_on == true` (the unmodeled
  Wiener bank), a `LoopRestorationSize` shift unreachable for the frame `SbSize`, or a disabled
  restoration tool; or a `ccso_params()` model carries an `Option` present on the wrong branch, an
  out-of-domain index, or a `ccso_offset_idx` length that disagrees with `maxEdgeInterval² * maxBand`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
