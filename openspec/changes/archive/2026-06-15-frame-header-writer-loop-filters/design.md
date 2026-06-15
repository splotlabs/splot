# Design: frame-header-writer-loop-filters

## Context

The frame loop-filter cluster has three parsers on the intra path:
`deblocking_filter_params()` (§ 5.18.5.2) reads the `apply_deblocking_filter` flags (or copies
them from the resolved MFH) and the per-index `DfDeltaQ` offsets; `gdf_params()` (§ 5.18.7.9)
reads `gdf_frame_enable`, the geometry-gated `gdf_per_block`, and two `f(2)` indices;
`cdef_params()` (§ 5.18.7.10) reads the damping / strengths counts, the skip-txfm arm, and the
per-set Y/UV primary/secondary strengths. All three sit behind `CodedLossless` / enable-flag
disabled returns and single-picture inferences.

## Decisions

- **Additive — derivations and canonicalization, not a model extension.** The principled split
  from #4b / #4c: those surfaced bits that affect *layout / downstream parsing* and were not
  recoverable. Every read-but-not-stored value here is instead reconstructible from what the
  model retains plus the same gating inputs the parser used:
  - `DfDeltaQ[i] = df_delta_q[i] - (1 << (dfParBits - 1))` is reversible: the writer recovers
    `raw = DfDeltaQ[i] + half` and range-checks it against the `f(dfParBits)` domain. The
    absent-`df_delta_q` inference (`DfDeltaQ[1] = DfDeltaQ[0]`, else `0`) is re-derived and
    validated.
  - The `gdf_per_block` coded/inferred gate depends on `gdfBlkSize`, derived from the parsed
    `tile_info()` geometry. To avoid a second copy of that derivation drifting from the parser,
    it is extracted into `pub(crate) gdf_per_block_is_coded(filter, geometry)` that both the
    parser and the writer call.
  - `cdef_y_pri_zero` / `cdef_uv_pri_zero` is a redundant encoding of a zero strength; the
    writer emits the canonical zero-flag form (semantic round-trip universal, byte-exact on the
    canonical subset — like the quant writer's `read_delta_q == 0`).
  - The `cdef_*_sec_strength` `3 -> 4` parser remap is reversed (`4 -> 3`); the stored domain
    `{0, 1, 2, 4}` is validated (a stored `3` is impossible and rejected).
- **Reject-before-write for every gated/inferred field.** Each writer runs a full
  `check_*_encodable` before the first `write_bit`, so a model the parser could not have
  produced (an `apply_deblocking_filter` that disagrees with the MFH copy, a `Some`/`None`
  Option on the wrong branch, a strength count mismatch, an out-of-domain index) is rejected
  with a typed `WriteError` and `bit_len() == 0`.
- **No panic on constructed models.** Every subtraction (`CdefDamping - 3`, `CdefStrengths -
  1`) is guarded by a prior range check; the `DfDeltaQ` math is done in `i64`; the `1 <<
  (dfParBits - 1)` shift is valid because `dfParBits >= 2` after the disabled return; an
  over-wide `dfParBits` (`> 32`) is rejected with `BitWidthTooLarge` before any write, mirroring
  the parser; every `f(n)` value is domain-checked before the write.

## Testing

Round-trip via the public parsers across every branch (the disabled returns, the MFH-update vs
direct deblocking arms, the single-picture inferences, the `gdf_per_block` coded-vs-inferred
gate, each `CdefOnSkipTxfm` arm, the `cdef` zero-flag and sec-strength-remap edges, num_planes
1 vs 3). One reject test per `NonCanonicalFrameHeader` path plus the `BitWidthTooLarge` path
(asserting `bit_len() == 0`), including the constructed-model panic edges. A round-trip property
test per parser (drive the parser on random bits + gating, re-emit, reparse) plus a
never-panics-on-arbitrary-constructed-models proptest per structure.
