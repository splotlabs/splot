# bitstream delta: decode-inter-header-shared-tail

Advances `AV2-5.18.2-FRAME-HEADER-INFO` (the non-intra control region) PAST
`InterStop::ReachedSharedTail` through the § 5.18.2 shared tail and inter-specific
arms to a new terminal `InterHeaderComplete`, for the verified minimal-tool
single-reference inter subset.

## MODIFIED Requirements

### Requirement: non-intra frame_header_info() shared-tail parse

The frame-header core parser SHALL, after the non-intra control region reaches
`InterStop::ReachedSharedTail` (§ 5.18.2 mirror :5183), continue into the § 5.18.2
shared structure cluster and the inter-specific coding-mode arms, reaching the
terminal `FrameHeaderParseStatus::InterHeaderComplete` for the modeled minimal-tool
single-reference inter subset. It SHALL reuse the intra shared-tail sub-parsers with
the inter inputs (`tile_info()` with `frame_is_intra == false`,
`quantization_params()`, `setup_qm_params()`, `delta_q_params()`, the per-segment
lossless / `allow_tcq` / `allow_parity_hiding` derivation, `deblocking_filter_params()`
with the inter `allow_df_sub_pu` arm, `gdf_params()`, `cdef_params()`, `lr_params()`,
`ccso_params()`, `read_tx_mode()`, `global_motion_params()`'s inter arm, and
`film_grain_config()`), plus the inter-specific `frame_reference_mode()`
`reference_select` f(1), `skip_mode_params()` `skip_mode_present` f(1), and the gated
`allow_bawp` / `allow_warpmv_mode` reads. The parsed inter-tail arms SHALL be
recorded on `FrameHeaderCore::inter_tail` and the shared-structure facts on the
shared `core` fields.

The parse SHALL be HONEST about the inter-specific arms the shared sub-parsers do
NOT model: it SHALL stop with the unsupported-coverage `UnsupportedUntilFeature`
(never a truncation) BEFORE reading any bit whose width or presence depends on an
unmodeled inter arm — specifically when `segmentation_enabled == 1` (the §5.18.7.1
`DerivedPrimaryRefFrame` arm), when restoration is enabled with `NumTotalRefs > 0`
(the §5.18.7.11 temporal-prediction arm), when CCSO is enabled (the §5.18.7.12
reuse arm), or when `global_motion_params()` reaches a cross-frame
`GlobalMotionStop` (`use_global_motion == 1` warp models). An
`Error::UnexpectedEof` inside the modeled tail SHALL be converted to the
facts-preserving `StoppedInsideInterControl`.

#### Scenario: minimal single-reference inter frame completes

- **WHEN** the `syn-2frame-inter-64x64.ivf` inter frame (single 64x64 zero-MV skip
  block, implicit reference map, `NumTotalRefs == 1`, restoration and CCSO disabled,
  `TipFrameMode == TIP_FRAME_DISABLED`, `!IsBridge`, `!bru_inactive`) is parsed with
  the post-key reference state
- **THEN** the frame header parses end-to-end to
  `FrameHeaderParseStatus::InterHeaderComplete` with the exact shared-tail and
  inter-tail values (single tile, `base_q_idx == 119`, segmentation off, deblocking
  off, `tx_mode == TX_MODE_LARGEST`, `reference_select == 0`,
  `skip_mode_present == 0`, `allow_bawp == 0`, `allow_warpmv_mode == 0`,
  `reduced_tx_set == 0`, `use_global_motion == 0`, `apply_grain == 0`)

#### Scenario: inter deblocking reads allow_df_sub_pu

- **WHEN** an inter frame has `enable_df_sub_pu == 1` and is not CodedLossless
- **THEN** `deblocking_filter_params()` reads `allow_df_sub_pu` f(1) BEFORE
  `apply_deblocking_filter[0]` (so the following apply bits land at the right
  position), whereas the intra / switch path reads no such bit

#### Scenario: enabled segmentation stops honestly

- **WHEN** an inter frame's shared tail reads `segmentation_enabled == 1`
- **THEN** the parser stops with `UnsupportedUntilFeature` (the §5.18.7.1
  `DerivedPrimaryRefFrame` arm is unmodeled), without reading any of the enabled
  block's bits, never a truncation

#### Scenario: enabled CCSO stops before any shared-tail bit

- **WHEN** an inter frame's sequence has `enable_ccso == 1`
- **THEN** the parser stops with `UnsupportedUntilFeature` at the admission gate
  BEFORE reading any shared-tail bit, so no possibly-mis-positioned `setup_qm` /
  `using_qmatrix` is ever exposed (the `syn-key-inter-64x64.ivf` inter frames take
  this path and stay `clean`)
