## MODIFIED Requirements

### Requirement: frame-header loop-restoration and CCSO writers

`splot-core` SHALL provide writers that are the exact inverse of the
`lr_params()` (§ 5.18.7.11) and `ccso_params()` (§ 5.18.7.12) parsers on the
accepted intra-path surface, plus the `tu(mx)` truncated-unary writer primitive
(§ 4.11.9) the CCSO writer needs. For every model the writer accepts, reparsing
the written bits with the corresponding parser and the same gating inputs SHALL
yield the original (`parse(write(x)) == x`). The writers SHALL be additive and
SHALL never panic: a model the writer cannot emit SHALL be rejected with a typed
writer error before any bit is written.

The parser can now model the fixed-coded frame-level Wiener NS bank
(`read_wienerns_filter()` with `readFrameFilters == 1`), but the writer does not
yet emit that syntax. The loop-restoration writer SHALL therefore reject any
`LrParams` plane with `frame_filter_bank` present, and SHALL keep rejecting
`frame_filters_on == true` until a later writer-support row implements the bank
syntax. The `frame_filters_on == false` surface (the `tool_index` reverse-lookup
and the `LoopRestorationSize` size-shift reversal) remains writable. The CCSO
writer SHALL reproduce the per-plane `ccso_offset_idx` loop byte-exactly from the
modeled values.

#### Scenario: modeled frame-filter bank is rejected

- **WHEN** a caller tries to write `lr_params()` with a plane carrying
  `frame_filter_bank`
- **THEN** the writer rejects the model before writing any bits
- **AND** the accepted no-bank loop-restoration surface still round-trips
