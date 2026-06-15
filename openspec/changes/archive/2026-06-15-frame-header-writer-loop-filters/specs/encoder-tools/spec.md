# encoder-tools delta: frame-header-writer-loop-filters

## ADDED Requirements

### Requirement: frame-header loop-filter writers

`splot-core` SHALL provide writers that are the exact inverse of the three frame loop-filter
parsers — `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9), and
`cdef_params()` (§ 5.18.7.10). For every model the writer accepts, reparsing the written bits
with the corresponding parser and the same gating inputs SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility and a behavior-preserving gate extraction on the filtering parser) and
SHALL never panic: a model the parser could not have produced SHALL be rejected with a typed
writer error before any bit is written.

Where a value has more than one parser-reachable encoding or is derived rather than stored — a
zero `cdef_*_pri_strength` (the `cdef_*_pri_zero` form), the `cdef_*_sec_strength` `3 <-> 4`
remap, the `DfDeltaQ[i]` offset (recovered as `df_delta_q[i] - (1 << (dfParBits - 1))`), and the
`gdf_per_block` coded/inferred gate (re-derived from the `GdfGeometry`) — the writer MAY emit the
canonical encoding and re-derive the inferred value; the round-trip is then semantic universally
and byte-exact on the canonical subset.

#### Scenario: each loop-filter structure round-trips across every branch

- **WHEN** a parsed deblocking / GDF / CDEF structure is written with the same gating inputs and
  reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch (the
  `CodedLossless` / enable-flag disabled returns, the deblocking MFH-update vs direct arms and
  the `DfDeltaQ` present/absent inferences, the single-picture `gdf`/`cdef` inferences, the
  `gdf_per_block` coded-vs-inferred gate, each `CdefOnSkipTxfm` arm, the `cdef` zero-flag and
  sec-strength remap, and `NumPlanes` 1 vs 3).

#### Scenario: a non-reproducible loop-filter model is rejected before any bit

- **WHEN** a model carries a value outside its descriptor domain (a `gdf` `f(2)` index, a
  `CdefDamping` / `CdefStrengths` outside its coded range, an over-wide `dfParBits`, a
  `cdef_*_sec_strength` of `3`, a `cdef_*_pri_strength` `>= 16`), an inferred field that
  disagrees with its gate (an `apply_deblocking_filter` not matching the MFH copy, a
  `gdf_per_block`/single-picture inference, an `Option` present on the wrong enabled/disabled
  branch), or a `strengths` length that disagrees with `CdefStrengths`
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
