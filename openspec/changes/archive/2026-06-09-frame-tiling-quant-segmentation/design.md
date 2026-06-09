# Design: frame header tiling, quantization, and segmentation

## Context

`parse_frame_header_core` (`crates/splot-core/src/headers/frame/info.rs`) parses the
intra path of `frame_header_info()` through `disable_cdf_update` and stops with
`FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation`. The § 5.18.2 call
order after that point (mirror: `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`)
is:

```text
tile_info( )                  § 5.18.7.2
quantization_params( )        § 5.18.6.1   (read_delta_q: § 5.18.6.3)
set_primary_ref_frame_and_ctx( 1 )          (no bits)
segmentation_params( )        § 5.18.7.1   (seg_info: § 5.4.9, already implemented)
setup_qm_params( )            § 5.18.6.2
delta_q_params( )             § 5.18.7.8
<lossless/QM derivation loop> § 5.18.2     (per-segment qm_index reads)
allow_tcq / allow_parity_hiding § 5.18.2
deblocking_filter_params( )   § 5.18.5.2   <- new stop point
```

The § 5.18.7.3 `tile_params()` helper and its tables already exist in
`crates/splot-core/src/tile.rs` (`AV2-5.18.7.3-TILE-PARAMS`, `done`) and are used by
the sequence-header tile config; `seg_info()` exists in
`crates/splot-core/src/segment.rs`. The validator already tracks quantizer-matrix OBU
availability state (used for the § 6.17.6.2 checks).

## Goals / Non-Goals

Goals: parse the structures above on the intra path with exact spec gating; expose
typed fields; add locally-decidable § 6.17.6 / § 6.17.7 diagnostics; surface fields in
`splot inspect`; keep the matrix honest.

Non-goals: everything listed in the proposal's Non-goals (filtering and deeper
structures, non-intra paths beyond existing stop points, tile-group payload, decoder
state).

## Decisions

1. **One submodule per structure family.** Add
   `crates/splot-core/src/headers/frame/tiling.rs` (`tile_info`),
   `quant.rs` (`quantization_params`, `read_delta_q`, `setup_qm_params`,
   `delta_q_params`, lossless/QM derivation), and `segmentation.rs`
   (`segmentation_params`). Alternative — growing `info.rs` — rejected: `info.rs` is
   already ~1200 lines and the repo's sequence-header precedent splits child syntax
   into focused modules.

2. **Extend `CoreSeqView` instead of passing the whole sequence header.** The new
   structures need sequence-derived inputs (`BitDepth`, `NumPlanes`,
   `separate_uv_delta_q`, `equal_ac_dc_q`, the `*_delta_q_enabled` flags, base DC/AC
   quantizer offsets, `seq_seg_info_present_flag` / `seq_allow_seg_info_change` /
   `enable_ext_seg` and stored sequence feature data, the sequence tile layout
   (`SeqUniformTileSpacingFlag`, `SeqTileColsLog2`, `SeqTileRowsLog2`, `SeqSbCols`,
   `SeqSbRows`, start arrays), `allow_tile_info_change`, `SbSize` /
   `get_seq_sb_size()`, `enable_avg_cdf` / `avg_cdf_type`, `choose_tcq_per_frame` /
   `enable_tcq`, `enable_parity_hiding`, `MaxSegments`). This keeps the explicit
   state-dependency principle from `frame-header-core-foundation`: every external
   input is a named field, not an implicit lookup.

3. **MFH-dependent branches stop explicitly rather than guess.** `tile_info()` and
   `segmentation_params()` consult multi-frame-header state when `cur_mfh_id > 0`.
   The existing core parser already leaves `cur_mfh_id > 0` frame dimensions unknown.
   Where a read is gated on MFH fields we do not model
   (`mfh_seg_info_present_flag`, `mfh_ext_seg_flag`, `mfh_allow_seg_info_change`,
   deblocking update flags), the parser stops with
   `FrameHeaderParseStatus::UnsupportedUntilFeature` naming the blocking Feature ID —
   the same pattern used for bridge/inter paths today. The fully-parsed path is
   `cur_mfh_id == 0` (direct sequence reference), which is what every test fixture
   exercises.

4. **`MiCols`/`MiRows` derived from the parsed frame size.** `tile_info()` needs
   them; derive per § 6.17.4.4 `compute_image_size()` from
   `FrameWidth`/`FrameHeight`. When the frame size is unknown (`cur_mfh_id > 0`),
   tile_info cannot be evaluated — covered by decision 3.

5. **`get_qindex(1, segmentId)` implemented minimally.** The lossless derivation
   needs only the "ignore delta-q, use segment SEG_LVL_ALT_Q feature" form of
   `get_qindex` (spec § 7 quantizer selection; implementers must cite the exact
   section from the mirror). It is a pure function of `base_q_idx` and parsed
   segmentation feature data — no decoder state.

6. **Status enum: replace, don't accumulate.** `FrameHeaderParseStatus` is
   `#[non_exhaustive]`; remove `StoppedBeforeFilteringQuantSegmentation` and add
   `StoppedBeforeDeblockingFilterParams` (label
   `stopped_before_deblocking_filter_params`). Pre-1.0 breaking change is acceptable
   and keeps the enum honest: no path produces the old value anymore. Inspector
   snapshots update accordingly.

7. **Diagnostics live in `splot-validate`, parsing in `splot-core`.** Range/
   relational checks that are bitstream-conformance requirements (§ 6.17.7.2 tile
   bounds, `context_update_tile_id`; § 6.17.6.2 QM plane-count) become validator
   diagnostics with stable ids under the existing frame-header rule-id namespace;
   the parser itself only fails on structural errors (EOF, invalid descriptor).
   New rule ids are registered in `docs/VALIDATOR-DIAGNOSTICS.md`
   (`cargo xtask check-diagnostic-registry` gates this).

## Risks / Trade-offs

- [Spec-gating mistakes in deeply nested conditions] → Every read cites the mirror
  line in a comment; tests encode bit-exact fixtures built from the spec text;
  property tests guarantee no panic/overread; AVM remains the future oracle
  (`CONF-AVM-DIFF-HARNESS`).
- [Breaking enum change ripples through validate/cli] → Compiler-driven: exhaustive
  matches fail to build until updated; snapshot tests catch label drift.
- [Sequence state plumbing widens `CoreSeqView`] → Fields are grouped per structure
  (quant/seg/tile sub-views) so unrelated callers don't churn.
- [Lossless derivation subtly wrong (BaseYDcDeltaQ offsets)] → Unit tests pin the
  exact § 5.18.2 formula with hand-computed vectors from the mirror text.

## Migration Plan

Single PR; no data or API migration. Inspector JSON gains fields (additive) and the
status label changes (documented in the PR). Matrix rows move
`AV2-5.18.6-QUANTIZATION` and `AV2-5.18.7-SEGMENTATION-TILING` forward with proof;
umbrella rows stay `partial`.

## Open Questions

- None blocking. If the mirror's `get_qindex` definition turns out to require
  decoder-side state beyond parsed segmentation data, the lossless loop's `qm_index`
  reads cannot be gated exactly; in that case stop before the derivation loop with an
  explicit status and a `TODO(spec: AV2-5.18.6-QUANTIZATION)` instead of guessing.
