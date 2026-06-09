# Delta spec: bitstream — frame header tiling, quantization, and segmentation

Mirror citations: `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6`,
`#s-5-18-7-1`, `#s-5-18-7-2`, `#s-5-18-7-8`, and the § 5.18.2 intra-path tail
(`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).

## ADDED Requirements

### Requirement: Frame tile info parsing
The intra-path frame-header parser SHALL parse `tile_info()` per AV2 v1.0.0
§ 5.18.7.2 immediately after `disable_cdf_update`, deriving `sbCols`/`sbRows` from
the active sequence's superblock size and frame dimensions, evaluating the
`reuse_tile_info` eligibility condition exactly as specified (including
`uniform_eligible()` for uniform sequence tile spacing and exact `SeqSbCols`/
`SeqSbRows` match otherwise), and reading `reuse_tile_info` only when
`haveTileParams` and the eligibility condition hold and `allow_tile_info_change`
is set. When `reuse_tile_info` is 0 the parser SHALL invoke the existing
§ 5.18.7.3 `tile_params()` helper with frame dimensions; when 1 it SHALL reuse
the sequence tile layout via `reuse_tile_params()`. The parser SHALL read
`context_update_tile_id` (width `TileRowsLog2 + TileColsLog2`, gated on the
`enable_avg_cdf`/`avg_cdf_type` condition) and `tile_size_bytes_minus_1` only
when `(TileCols > 1 || TileRows > 1) && !IsBridge && TipFrameMode !=
TIP_FRAME_AS_OUTPUT`, and SHALL expose the resulting tile layout
(`TileCols`, `TileRows`, `TileColsLog2`, `TileRowsLog2`, `MiColStarts`,
`MiRowStarts`, `TileSizeBytes`, `context_update_tile_id`) as typed fields.

#### Scenario: Single-tile frame skips context-update fields
- **WHEN** a key-frame header parses `tile_info()` and the derived layout has
  `TileCols == 1 && TileRows == 1`
- **THEN** the parser does not read `context_update_tile_id` or
  `tile_size_bytes_minus_1`, reports `context_update_tile_id = 0`, and continues
  at the correct bit position

#### Scenario: Explicit tile params on the frame path
- **WHEN** `reuse_tile_info` is 0 and the payload encodes a valid multi-tile
  uniform layout via `tile_params()`
- **THEN** the parsed `TileCols`/`TileRows`/start arrays match the § 5.18.7.3
  derivation and `tile_size_bytes_minus_1` yields `TileSizeBytes`

#### Scenario: Truncated tile info is a typed error
- **WHEN** the payload ends inside `tile_info()`
- **THEN** the parser returns a typed EOF error and does not panic

### Requirement: Frame quantization params parsing
The intra-path frame-header parser SHALL parse `quantization_params()` per AV2
v1.0.0 § 5.18.6.1: `base_q_idx` with bit width `n = BitDepth == 8 ? 8 : 9`, the
gated `DeltaQYDc` read (`TipFrameMode != TIP_FRAME_AS_OUTPUT &&
y_dc_delta_q_enabled`), and the chroma block gated on `NumPlanes > 1` and the
sequence `uv_ac_delta_q_enabled`/`uv_dc_delta_q_enabled` flags, including
`diff_uv_delta` (only when `separate_uv_delta_q`), the `equal_ac_dc_q`
assignments, and the V-plane copy when `diff_uv_delta` is 0. `read_delta_q()`
SHALL be implemented per § 5.18.6.3 (`delta_coded` f(1), then `delta_q` su(7)).
All resulting quantizer values (`base_q_idx`, `DeltaQYDc`, `DeltaQUDc`,
`DeltaQUAc`, `DeltaQVDc`, `DeltaQVAc`) SHALL be exposed as typed fields.

#### Scenario: Monochrome stream reads no chroma deltas
- **WHEN** the active sequence has `NumPlanes == 1`
- **THEN** only `base_q_idx` (and `DeltaQYDc` when enabled) are read and all
  chroma deltas are 0

#### Scenario: Shared UV delta
- **WHEN** `separate_uv_delta_q` is 0 and chroma delta reads are enabled
- **THEN** `diff_uv_delta` is not read, and parsed V deltas equal the U deltas

#### Scenario: 10-bit base_q_idx width
- **WHEN** the active sequence bit depth is greater than 8
- **THEN** `base_q_idx` is read as f(9)

### Requirement: Frame setup QM params parsing
The intra-path frame-header parser SHALL parse `setup_qm_params()` per AV2
v1.0.0 § 5.18.6.2: `using_qmatrix` f(1); when set, `pic_qm_num_minus_1` f(2)
only when `segmentation_enabled` (else inferred 0), then per-index `qm_y` f(4)
and, when `NumPlanes > 1`, `qm_uv_same_as_y` f(1) with `qm_u`/`qm_v` reads
gated on `separate_uv_delta_q` exactly as specified. Parsed QM levels SHALL be
exposed as typed fields. Note § 5.18.2 places `setup_qm_params()` after
`segmentation_params()`; the parser SHALL follow that call order.

#### Scenario: QM disabled reads nothing further
- **WHEN** `using_qmatrix` is 0
- **THEN** no `pic_qm_num_minus_1` or `qm_*` fields are read

#### Scenario: Multiple QM sets with segmentation
- **WHEN** `using_qmatrix` is 1 and `segmentation_enabled` is 1 and
  `pic_qm_num_minus_1` is 2
- **THEN** three `qm_y` entries are parsed with their chroma companions per the
  `qm_uv_same_as_y`/`separate_uv_delta_q` gating

### Requirement: Frame segmentation params parsing
The intra-path frame-header parser SHALL parse `segmentation_params()` per AV2
v1.0.0 § 5.18.7.1: `segmentation_enabled` f(1); when enabled, derive
`haveSegParams`/`allowChange`/`mfhId` from the multi-frame-header
(`mfh_seg_info_present_flag`, `mfh_ext_seg_flag`, `enable_ext_seg`,
`mfh_allow_seg_info_change`) or sequence (`seq_seg_info_present_flag`,
`seq_allow_seg_info_change`) state, read `reuse_seg_info` only when
`allowChange`, reuse stored feature data when `reuse_seg_info` is 1, and
otherwise parse `seg_info(MaxSegments)` with the existing § 5.4.9 helper. On
the intra path `DerivedPrimaryRefFrame == PRIMARY_REF_NONE`, so
`segmentation_update_map` SHALL be inferred 1 and
`segmentation_temporal_update` inferred 0 without reading bits. The parser
SHALL derive `SegIdPreSkip` and `LastActiveSegId` from the resulting feature
enables and expose them as typed fields.

#### Scenario: Segmentation disabled clears features
- **WHEN** `segmentation_enabled` is 0
- **THEN** all feature enables/data are zero and no further segmentation bits
  are read

#### Scenario: Fresh seg_info on intra frame
- **WHEN** `segmentation_enabled` is 1 and no sequence/MFH segment info is
  reusable
- **THEN** `seg_info(MaxSegments)` is parsed, no
  `segmentation_update_map`/`segmentation_temporal_update` bits are read, and
  `LastActiveSegId`/`SegIdPreSkip` reflect the parsed feature enables

### Requirement: Frame quantizer index delta parsing and lossless derivation
The intra-path frame-header parser SHALL parse `delta_q_params()` per AV2
v1.0.0 § 5.18.7.8 (`delta_q_present` f(1) only when `base_q_idx > 0`;
`delta_q_res` f(2) only when present), then execute the § 5.18.2 per-segment
lossless/QM derivation loop: compute `LosslessArray`/`CodedLossless`/
`HasLosslessSegment` from `get_qindex(1, segmentId)` and the parsed quantizer
deltas plus the sequence base DC/AC offsets, and when `using_qmatrix` read
`qm_index` f(CeilLog2(pic_qm_num_minus_1 + 1)) for each non-lossless segment.
It SHALL then read `allow_tcq` f(1) only when `!CodedLossless &&
choose_tcq_per_frame` (else inferred per spec) and `allow_parity_hiding` f(1)
only when `!(CodedLossless || !enable_parity_hiding || allow_tcq)`.

#### Scenario: Zero base_q_idx skips delta-q
- **WHEN** `base_q_idx` is 0
- **THEN** `delta_q_present` is not read and is 0, and `delta_q_res` is 0

#### Scenario: Lossless segment forces QM level 15
- **WHEN** `using_qmatrix` is 1 and a segment is lossless per `LosslessArray`
- **THEN** no `qm_index` is read for that segment and its QM levels are 15

### Requirement: New frame-header stop point
After `allow_parity_hiding`, the intra-path parser SHALL stop with a new
explicit `FrameHeaderParseStatus` value indicating it stopped before
`deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2). The
`StoppedBeforeFilteringQuantSegmentation` status SHALL no longer be produced on
this path, and no full-payload trailing-bits conformance SHALL be inferred from
the new partial status.

#### Scenario: Status reports the deeper stop point
- **WHEN** a valid intra frame header parses through the new structures
- **THEN** the parse status is the new stopped-before-deblocking value and
  `consumed_bits` covers exactly the parsed prefix

### Requirement: New frame parsers never panic
All new frame-header parsing paths (tile info, quantization, QM setup,
segmentation, delta-q, lossless derivation) SHALL return typed errors on
truncated or malformed input and SHALL be covered by property tests over
arbitrary byte slices, with positive, negative, and EOF unit tests for each
structure.

#### Scenario: Property test over arbitrary input
- **WHEN** the frame-header core parser runs over arbitrary bytes and arbitrary
  sequence-state inputs
- **THEN** it never panics and never reads past the payload bounds
