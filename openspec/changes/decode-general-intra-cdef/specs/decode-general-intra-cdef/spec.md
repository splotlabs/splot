## ADDED Requirements

### Requirement: General intra § 7.18 CDEF orchestration
The decoder SHALL apply the AV2 § 7.18 CDEF (Constrained Directional Enhancement
Filter) in place over the reconstructed general intra frame, AFTER the § 7.17
deblocking pass (reading the deblocked `CurrFrame`) and before the frame is
frozen, for the verified subset: an 8-bit 4:2:0 intra key frame with
`cdef_frame_enable == 1`, `CdefStrengths == 1` (so § 5.20.10.1 `read_cdef` reads
no per-block symbol — `cdef_idx[r][c] == 0` for the whole frame),
`cdef_on_skip_txfm_frame_enable == 1` (so the § 7.18.1 `skip` is `0` with no
`Skips` / `LosslessArray` lookup), a single tile, and segmentation disabled. The
orchestration SHALL snapshot the deblocked frame before filtering (§ 7.18 filters
`CurrFrame` into `CdefFrame`, so every tap reads a pre-CDEF sample regardless of
raster write order), iterate the § 7.18 8x8 blocks (`step4 = 2` MI units), and per
block run the § 7.18.2 direction search over the deblocked luma 8x8, derive the
§ 7.18.1 luma `priStr` (`(priStr * (4 + Min(FloorLog2(var >> 6), 12)) + 8) >> 4`
when `var` is nonzero, else `0`), `secStr`, `dir` (`priStr == 0 ? 0 : yDir`), and
`damping` (`CdefDamping + coeffShift`), then the chroma `priStr` / `secStr` /
`dir` (`Cdef_Uv_Dir[SubsamplingX][SubsamplingY][yDir]`) / `damping`
(`CdefDamping + coeffShift - 1`), and for each output sample fetch the six
§ 7.18.3 directional taps via `cdef_get_at` with the § 5.20.9.3
`is_inside_filter_region` (single-tile → `is_inside_frame`: the candidate luma MI
must lie inside the frame MI grid) availability check, apply the § 7.18.3
constrain / primary-secondary tap filter, and write the deringed sample back. A
CDEF-off frame (`cdef_frame_enable == 0`) SHALL skip the pass so the
reconstruction is byte-identical.

This requirement SHALL NOT claim a multi-strength (`CdefStrengths > 1`) frame and
its per-block `cdef_index0` / `cdef_index_minus_1` symbols,
`cdef_on_skip_txfm_frame_enable == 0` skip handling, lossless / segmentation
`skipChroma`, a 10-bit CDEF-active frame, multiple tiles, the other in-loop
filters, inter frames, or any AVM / dav2d invocation. A CDEF-active frame outside
this admitted subset SHALL be rejected with a structured
`decode/unsupported-feature` diagnostic before any caller-visible output. The
8-bit CDEF-off and 10-bit reconstruction paths SHALL remain byte-identical.

#### Scenario: CDEF-active intra frames decode to the oracle
- **WHEN** `splot decode` is given a committed CDEF-active intra key frame —
  `syn-2sb-cdef-intra-128x64-q130.ivf` (`CdefDamping 5`, y_pri 1 / y_sec 4),
  `syn-2sb-cdef-intra-128x64-q120.ivf` (`CdefDamping 4`, y_pri 2 / y_sec 4), each
  with `cdef_frame_enable == 1`, `CdefStrengths == 1`,
  `cdef_on_skip_txfm_frame_enable == 1`, two 64x64 superblocks split into all-DC
  32x32 blocks
- **THEN** the general intra path reconstructs the frame, applies the § 7.18 CDEF
  pass in place after deblocking, and succeeds
- **AND** the `--output-format raw` bytes equal the avmdec and dav2d raw outputs
  exactly (raw md5 `192e3935f9892345a14e02cb4baf4ba5` and
  `2319a8f00af1ebb919a52ba18d90f4a1` respectively)
- **AND** the § 7.18.2 direction search drives a real per-block dering (`yDir`
  varies 0/2/4/6, `var` positive), changing thousands of luma samples from the
  CDEF-off reconstruction

#### Scenario: deblock-and-CDEF fixture pins the filter order
- **WHEN** `splot decode` is given the committed
  `syn-2sb-cdefdeblock-intra-128x64-q100.ivf`, which has BOTH
  `apply_deblocking_filter == [false, true, true, true]` (§ 7.17 deblocking) AND
  `cdef_frame_enable == 1` with `CdefStrengths == 1`, `CdefDamping 4`, y_pri 1 /
  y_sec 4 (§ 7.18 CDEF) active
- **THEN** it reconstructs bit-exactly to the avmdec and dav2d raw outputs (raw
  md5 `472d95801ce2a112160bcdfee93957d5`), with both filters changing samples and
  only their composition in spec order (deblock THEN CDEF) matching the oracle

#### Scenario: CDEF-off frame stays byte-identical
- **WHEN** the existing general intra fixtures whose `cdef_frame_enable == 0` are
  decoded after the CDEF pass is added
- **THEN** each reconstructs to the same bytes as before, because the § 7.18 pass
  is skipped entirely when CDEF is frame-disabled

#### Scenario: CDEF leaf-math primitives are unit-pinned
- **WHEN** the § 7.18.2 direction search runs over a flat block, a row-varying
  block, and the § 7.18.3 constrain / tap-filter primitives run over their branch
  table
- **THEN** the flat block yields `(yDir 0, var 0)`, the row-varying block selects
  direction 2 with positive `var`, the constrain returns `0` for a zero threshold
  / large diff and passes small diffs through both signs, and one available bright
  primary tap pulls the center toward the neighbour by a hand-computed amount,
  deterministically pinning the leaf math independent of the scheduler

#### Scenario: multi-strength and 10-bit CDEF-active reject
- **WHEN** a CDEF-active frame has `CdefStrengths > 1`,
  `cdef_on_skip_txfm_frame_enable == 0`, or is 10-bit
- **THEN** the decoder rejects it before any caller-visible output with a
  structured `decode/unsupported-feature` diagnostic, because no oracle fixture
  pins the per-block `read_cdef` symbols, the skip handling, or the 10-bit pass
