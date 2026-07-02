## MODIFIED Requirements

### Requirement: First-inter-frame frontier flexible-MV-resolution subset
The decoder SHALL decode single-reference inter frames of sequences with
`enable_flex_mvres = 1`: for a NEWMV-family block outside the adaptive-MVD
path it SHALL read `use_most_probable_precision` over
`TileUseMostProbablePrecisionCdf[ctx]` (ctx = the count of `NPos` neighbours
with `UseMostProbablePrecisions` set) and, when zero, `pb_mv_precision` over
`TilePbMvPrecisionCdf[ctx][FrameMvPrecision - MV_PRECISION_HALF_PEL]` (ctx =
any `NPos` neighbour with `MvPrecisions < FrameMvPrecision`), derive
`MvPrecision` via `Max(MV_PRECISION_ONE_PEL, FrameMvPrecision - 2) -
pb_mv_precision` with the `<= MV_PRECISION_TWO_PEL` decrement, decode the
§ 5.20.7.20 MV shell at that precision (including the `MV_PRECISION_EIGHT_PEL`
and `MV_PRECISION_FOUR_PEL` shell-class banks), and apply § 5.20.7.13
`lower_mv_precision` to the predictor when `MvPrecision <
MV_PRECISION_HALF_PEL`. The § 8.3.2 contexts for `is_inter`, `skip_flag`,
`use_amvd`, `comp_mode`, and `single_ref` SHALL be derived over the
unrestricted `NPosBuf` neighbour list (with `count_refs` counting both
reference lists), while `interp_filter` and the precision contexts use the
superblock-row-restricted `NPos` list. For a frame enabling the INTERINTRA
motion mode, the § 5.20.7.14 SIMPLE-path `inter_intra` flag SHALL be read for
single-reference non-warp blocks of 8x8..=64x64 and a set flag SHALL reject
with a structured diagnostic before any output. The committed
`syn-2frame-inter-64x64-10bit.ivf` and `syn-grid-inter-128x128-q80.ivf`
fixtures SHALL decode byte-identical to `avmdec --i420 --rawvideo`.

#### Scenario: Flex-mvres 10-bit inter fixture decodes bit-exact
- **GIVEN** `syn-2frame-inter-64x64-10bit.ivf` (`enable_flex_mvres = 1`,
  `allow_bawp = 1`, 10-bit, one NEARMV skip=0 inter block)
- **WHEN** the stream is decoded
- **THEN** both frames match the pinned avmdec-verified hashes

#### Scenario: Interintra-flagged block defers
- **GIVEN** an inter frame enabling the INTERINTRA motion mode whose block
  decodes `inter_intra = 1`
- **WHEN** the block's motion mode is read
- **THEN** decode rejects with `inter_interintra_unimplemented` before output
