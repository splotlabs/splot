# bitstream Specification

## Purpose

The AV2 bitstream model and parsers in `splot-core`. Normative reference: AV2
v1.0.0. This capability never panics on malformed input — every failure is a typed
`Error`.

Tracked by Feature IDs: `AV2-4.11.6-LEB128`, `AV2-5.2.2-OBU-HEADER`,
`AV2-5.2.1-OBU-TYPE`, `AV2-B-ANNEXB-OBU-ENVELOPE`, `AV2-4.11.7-SU`,
`AV2-5.4.9-SEGMENT-INFO`, `AV2-5.8-LAYER-CONFIG-RECORD`,
`AV2-5.9-ATLAS-SEGMENT`, `AV2-5.15-CONTENT-INTERPRETATION`,
`AV2-5.18.7.3-TILE-PARAMS`,
`AV2-5.18.1-FRAME-HEADER-GENERAL`, `AV2-5.18.2-FRAME-HEADER-INFO`,
`AV2-5.19-TILE-GROUP`, `AV2-IVF-CONTAINER`, plus the not-yet-parsed header/syntax rows in
`docs/IMPLEMENTATION-MATRIX.toml`.
## Requirements
### Requirement: LEB128 decoding

The reader SHALL decode `leb128()` per AV2 v1.0.0 § 4.11.6: byte-aligned, at most
8 bytes, value bounded to `(1 << 32) - 1`, non-minimal encodings permitted.

#### Scenario: minimal and non-minimal encodings

- **WHEN** decoding `0x00` or `0x80 0x00`
- **THEN** both yield `0`, recording the number of bytes consumed

#### Scenario: overflow or overlong

- **WHEN** a value exceeds `u32` or uses more than 8 bytes
- **THEN** an `Error` is returned, never a panic

#### Scenario: bit-reader descriptor path

- **WHEN** the bit reader decodes the `leb128()` element `0x80 0x01`
- **THEN** it yields `128` and advances by two bytes
- **AND** a truncated or overlong code returns an `Error`, never a panic

### Requirement: Signed integer descriptor `su(n)`

`splot-core` SHALL expose a panic-free `su(n)` reader that reads `n` bits MSB-first
and sign-extends per AV2 v1.0.0 §4.11.7 (`signMask = 1 << (n - 1)`; if the sign bit is
set, `value -= 2 * signMask`). It SHALL reject `n == 0` and `n` greater than the
supported fixed-width maximum with a structured error rather than a panic.

#### Scenario: single-bit signed values

- **GIVEN** a one-bit `su(1)` field
- **WHEN** the descriptor reads a `0` bit and then a `1` bit
- **THEN** it SHALL return `0` for the `0` bit
- **AND** `-1` for the `1` bit.

#### Scenario: negative multi-bit value

- **GIVEN** an `n`-bit `su(n)` field whose top bit is set
- **WHEN** the descriptor reads the field
- **THEN** it SHALL return the sign-extended negative value `f(n) - 2 * (1 << (n - 1))`.

#### Scenario: truncated input

- **GIVEN** a `su(n)` field with fewer than `n` bits remaining
- **WHEN** the descriptor reads the field
- **THEN** it SHALL return a structured end-of-input error
- **AND** SHALL NOT panic.

### Requirement: Rice-Golomb descriptor

`splot-core` SHALL provide a panic-free `rg(n)` descriptor reader (AV2 v1.0.0
§ 4.11.10) for use by the content-interpretation OBU.

#### Scenario: well-formed rg(n) value

- **GIVEN** a bit reader positioned at an `rg(n)` code with a unary prefix that
  terminates within 32 bits
- **WHEN** the descriptor is read
- **THEN** it SHALL return `(q << n) + remainder`, where `q` is the number of
  leading one bits and `remainder` is the `n`-bit suffix.

#### Scenario: non-terminating rg(n) prefix

- **GIVEN** a bit reader whose next 32 bits are all one
- **WHEN** the descriptor is read
- **THEN** it SHALL return a typed error (the spec requires the descriptor never
  return a value less than 0) and SHALL NOT panic.

### Requirement: AV2 OBU header

The reader SHALL parse the AV2 OBU header per AV2 v1.0.0 § 5.2.2 — the AV2 layout
(`obu_header_extension_flag`, `obu_type`, `obu_tlayer_id`, and the optional
`obu_mlayer_id`/`obu_xlayer_id`), NOT the AV1 OBU header. There is no
`obu_forbidden_bit`, `obu_has_size_field`, or AV1 OBU type table.

#### Scenario: inferred xlayer

- **WHEN** parsing an `OBU_MSDO` or `OBU_TEMPORAL_DELIMITER` without the extension
- **THEN** `obu_xlayer_id` is inferred to `GLOBAL_XLAYER_ID`

### Requirement: Annex B envelope

The reader SHALL parse the Annex B length-delimited envelope per AV2 v1.0.0
Annex B: each OBU is a `leb128()` length followed by `open_bitstream_unit(...)`.
Header parsing SHALL be bounded to the declared OBU size.

#### Scenario: conformant Annex B stream

- **WHEN** a conformant Annex B stream is parsed
- **THEN** each OBU envelope, header, and payload is recovered with correct offsets

#### Scenario: bounded header parse

- **WHEN** an OBU header signals an extension byte beyond its declared size
- **THEN** parsing fails within that OBU rather than reading into the next one

#### Scenario: malformed input

- **WHEN** truncated or out-of-range input is parsed
- **THEN** a typed `Error` is returned and any parseable prefix is retained

### Requirement: Sequence-header child parser coverage

`splot-core` SHALL provide bounded parsers for implemented `sequence_header_obu()` child structures mapped in `docs/IMPLEMENTATION-MATRIX.toml`.

#### Scenario: implemented child syntax is parsed

- **GIVEN** an Annex B bitstream containing an `OBU_SEQUENCE_HEADER`
- **AND** a child structure whose matrix row has `parse = done`
- **WHEN** the OBU is dispatched by `open_bitstream_unit(sz)`
- **THEN** the child syntax SHALL be parsed into typed Rust fields
- **AND** the parser SHALL not read past the declared OBU payload.

#### Scenario: child syntax is intentionally not implemented

- **GIVEN** an Annex B bitstream containing an `OBU_SEQUENCE_HEADER`
- **AND** a child structure whose matrix row is still `todo` or `partial`
- **WHEN** the parser reaches that feature boundary
- **THEN** the parser SHALL return a bounded unimplemented payload status or typed unimplemented error with the owning Feature ID
- **AND** it SHALL NOT silently skip unknown syntax bits.

### Requirement: Segment information `seg_info(numSegments)`

`splot-core` SHALL expose a reusable `seg_info(numSegments)` parser (AV2 v1.0.0
§5.4.9) that, for each segment and each of the `SEG_LVL_MAX` features, reads
`feature_enabled` and, when enabled, the feature value using the exact
`Segmentation_Feature_Bits`, `Segmentation_Feature_Signed`, and
`Segmentation_Feature_Max` tables - signed features via `su(1 + bitsToRead)` clipped to
`[-limit, limit]`, unsigned features via `f(bitsToRead)` clipped to `[0, limit]`. It
SHALL support the 8- and 16-segment paths and zero-initialize unused feature slots.

#### Scenario: all-disabled segment info

- **GIVEN** a `seg_info(numSegments)` where every `feature_enabled` bit is 0
- **WHEN** the parser runs
- **THEN** every feature SHALL be disabled with data 0
- **AND** exactly `numSegments * SEG_LVL_MAX` bits SHALL be consumed.

#### Scenario: signed quantizer feature

- **GIVEN** a segment whose quantizer feature (`SEG_LVL_ALT_Q`) is enabled
- **WHEN** the parser reads the feature value
- **THEN** it SHALL read `su(1 + Segmentation_Feature_Bits[0])`
- **AND** clip the result to `[-Segmentation_Feature_Max[0], Segmentation_Feature_Max[0]]`.

### Requirement: Sequence segment config parses `seg_info()`

`splot-core` SHALL parse `seg_info(MaxSegments)` inside `sequence_segment_config()`
(AV2 v1.0.0 §5.4.4) when `seq_seg_info_present_flag` is 1, with
`MaxSegments = enable_ext_seg ? 16 : 8`, and SHALL no longer leave the call bounded.

#### Scenario: sequence segment info present

- **GIVEN** a sequence header with `seq_seg_info_present_flag` equal to 1
- **WHEN** the sequence header is parsed
- **THEN** the parsed segment config SHALL contain the parsed `seg_info()`
- **AND** the sequence header SHALL report itself fully parsed (no segment-info bound).

### Requirement: Sequence tile config parses `tile_params()`

`splot-core` SHALL implement the `tile_params()` helper (AV2 v1.0.0 §5.18.7.3 with the
§5.18.7.5 `uniform_spacing` and §5.18.7.7 `tile_log2` helpers and the §9.3 / level-tier
conversion tables) and call it from `sequence_tile_config()` (AV2 v1.0.0 §5.4.2) when
`seq_tile_info_present_flag` is 1, with the parsed sequence frame dimensions,
superblock size, tier, and level. Valid (non-reserved-level) sequence headers SHALL
parse the tile config fully, with no bounded tile-params status.

#### Scenario: uniform sequence tile config

- **GIVEN** a sequence header with `seq_tile_info_present_flag` equal to 1 and
  `uniform_tile_spacing_flag` equal to 1
- **WHEN** the sequence header is parsed
- **THEN** the parsed tile params SHALL record uniform spacing with the derived tile
  columns and rows
- **AND** the sequence header SHALL report itself fully parsed.

#### Scenario: non-uniform sequence tile config

- **GIVEN** a sequence header with `seq_tile_info_present_flag` equal to 1 and
  `uniform_tile_spacing_flag` equal to 0
- **WHEN** the sequence header is parsed
- **THEN** the parser SHALL read each tile width and height via `ns()`
- **AND** record the resulting tile column and row counts.

### Requirement: HLS payload foundation

`splot-core` SHALL parse temporal delimiter, MSDO, and multi-frame-header payload syntax to the extent recorded in the matrix.

#### Scenario: MSDO local syntax is malformed

- **GIVEN** an MSDO OBU whose local syntax violates an implemented parser bound
- **WHEN** the bitstream is parsed
- **THEN** the parser SHALL return a structured error or invalid payload status
- **AND** the validator SHALL convert it to a diagnostic rather than panicking.

### Requirement: Content interpretation OBU parser

`splot-core` SHALL parse `content_interpretation_obu()` (AV2 v1.0.0 § 5.15) into
typed fields, reaching `timing_info()` when `ci_timing_info_present_flag` is set,
and SHALL be dispatched from `open_bitstream_unit(sz)`.

#### Scenario: content interpretation with timing is parsed

- **GIVEN** an Annex B bitstream containing an `OBU_CONTENT_INTERPRETATION`
- **AND** `ci_timing_info_present_flag` equal to 1 with valid timing
- **WHEN** the OBU is dispatched by `open_bitstream_unit(sz)`
- **THEN** the syntax SHALL be parsed into typed Rust fields including the present
  `timing_info()`
- **AND** the parser SHALL NOT read past the declared OBU payload.

#### Scenario: content interpretation optional branches

- **GIVEN** an `OBU_CONTENT_INTERPRETATION` with any combination of
  `ci_color_description_present_flag`, `ci_chroma_sample_position_present_flag`, and
  `ci_aspect_ratio_info_present_flag` set
- **WHEN** the OBU is parsed
- **THEN** each present branch (including the `rg(2)` color-description id, the
  chroma-sample-position UVLC fields, and the extended `ci_sar_width`/
  `ci_sar_height` path when `ci_aspect_ratio_idc == 255`) SHALL be read into typed
  fields without skipping unknown bits.

#### Scenario: content interpretation truncated mid-field

- **GIVEN** an `OBU_CONTENT_INTERPRETATION` whose payload ends inside the fixed
  header or inside `timing_info()`
- **WHEN** the OBU is parsed
- **THEN** the parser SHALL return a structured error
- **AND** the validator SHALL convert it to a diagnostic rather than panicking.

### Requirement: Multi-frame header parses `seg_info()`

`splot-core` SHALL parse `seg_info(mfh_ext_seg_flag ? 16 : 8)` inside
`multi_frame_header_obu()` (AV2 v1.0.0 §5.7) when `mfh_seg_info_present_flag` is 1, and
SHALL no longer mark the multi-frame header bounded at `seg_info()`.

#### Scenario: multi-frame header segment info present

- **GIVEN** a multi-frame header with `mfh_seg_info_present_flag` equal to 1
- **WHEN** the multi-frame header is parsed
- **THEN** the parsed result SHALL contain the parsed `seg_info()`
- **AND** SHALL NOT report a `seg_info()` bound.

### Requirement: Layer configuration record OBU parsing

`splot-core` SHALL parse `layer_config_record_obu()` (AV2 v1.0.0 § 5.8) into typed
syntax, dispatching on `obu_xlayer_id` to `lcr_global_info()` or
`lcr_local_info(obu_xlayer_id)`, reading the full nested syntax (including the
length-bounded `lcr_global_payload()`), never skipping payload bits and never reading
past the OBU boundary, and retaining reserved-zero fields rather than rejecting them.

#### Scenario: minimal global record

- **GIVEN** a global LCR (`obu_xlayer_id == GLOBAL_XLAYER_ID`) with no optional sections
- **WHEN** the parser reads it
- **THEN** it SHALL return a global record exposing `lcr_global_config_record_id` and
  `lcr_xlayer_map`.

#### Scenario: global payload remaining bits and overflow

- **GIVEN** a global LCR with `lcr_global_payload_present_flag` set
- **WHEN** the parser reads the payload
- **THEN** it SHALL consume exactly `lcr_data_size * 8` bits including the trailing
  `lcr_remaining_payload_bit` bits
- **AND** parsed content exceeding `lcr_data_size * 8` SHALL return a structured error.

#### Scenario: truncated record

- **GIVEN** a layer configuration record OBU that ends mid-field
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured end-of-input error and SHALL NOT panic.

### Requirement: Atlas segment info OBU parsing

`splot-core` SHALL parse `atlas_segment_info_obu()` (AV2 v1.0.0 § 5.9) into typed
syntax for all five `ats_atlas_segment_mode_idc` modes plus `ats_label_segment_info()`,
never skipping payload bits and never reading past the OBU boundary, and SHALL
range-check the mode and the segment/region counts before any loop.

#### Scenario: single-mode atlas

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc == SINGLE_ATLAS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a record with `num_segments == 1` and the nominal
  dimensions.

#### Scenario: out-of-range mode or count

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc` greater than 4, or a segment
  count reaching `MAX_NUM_ATLAS_SEGMENTS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured atlas-segment error before iterating, and SHALL
  NOT panic.

### Requirement: Frame-header prefix parser exposes activation/reference fields

`splot-core` SHALL expose a prefix-only frame-header representation containing at
least `cur_mfh_id`, the direct `seq_header_id_in_frame_header` when present, the
derived referenced sequence-header id when resolvable, the OBU-type context, the
consumed bit count, and a status indicating that full §5.18 parsing is not implied
(AV2 v1.0.0 §5.18.1 / §5.18.2). It SHALL stop after the activation/reference fields
and SHALL NOT consume the rest of `frame_header_info()`.

#### Scenario: direct sequence-header id path

- **GIVEN** a frame-header prefix where `cur_mfh_id` is 0
- **WHEN** the parser reads the activation/reference fields
- **THEN** the prefix result SHALL contain `seq_header_id_in_frame_header`
- **AND** the prefix status SHALL indicate that full §5.18 parsing is not implied.

#### Scenario: bridge frame infers cur_mfh_id

- **GIVEN** an `OBU_BRIDGE_FRAME` frame header
- **WHEN** the parser reads the activation/reference fields
- **THEN** `cur_mfh_id` SHALL be inferred to 0 without consuming a `uvlc`
- **AND** the prefix result SHALL contain `seq_header_id_in_frame_header`.

#### Scenario: MFH id path

- **GIVEN** a frame-header prefix where `cur_mfh_id` is greater than 0
- **WHEN** the parser reads the activation/reference fields
- **THEN** the prefix result SHALL contain `cur_mfh_id`
- **AND** sequence-header resolution SHALL be deferred to validator state using
  multi-frame-header availability records.

### Requirement: Tile-group prefix parser exposes frame-header presence

`splot-core` SHALL expose a prefix-only tile-group representation containing
`is_first_tile_group`, `frame_header_present_flag`, an optional frame-header prefix,
and the consumed bit count (AV2 v1.0.0 §5.19). It SHALL stop before tile payload
syntax.

#### Scenario: first tile group reaches the frame header

- **GIVEN** a tile-group OBU whose first bit signals `is_first_tile_group` equal to 1
- **WHEN** the tile-group prefix parser runs
- **THEN** it SHALL infer `frame_header_present_flag` equal to 1
- **AND** parse the frame-header prefix
- **AND** stop before tile payload syntax.

#### Scenario: non-first tile group does not parse a header copy

- **GIVEN** a tile-group OBU with `is_first_tile_group` equal to 0 and
  `frame_header_present_flag` equal to 1
- **WHEN** the tile-group prefix parser runs
- **THEN** it SHALL record that a frame header is present
- **AND** SHALL NOT parse the `frame_header_copy()` bits as activation fields.

### Requirement: Prefix parse errors are structured

`splot-core` SHALL return a typed `Error` for EOF or invalid descriptors encountered
before a required prefix field, without panicking.

#### Scenario: EOF before cur_mfh_id

- **GIVEN** a frame-header payload that ends before the prefix parser can read
  `cur_mfh_id`
- **WHEN** the prefix parser runs
- **THEN** it SHALL return a structured parse error
- **AND** no library panic, unwrap, or unreachable state SHALL occur.

### Requirement: Parsers never panic on arbitrary input

`splot-core` SHALL parse `su(n)`, `seg_info()`, and `tile_params()` without panicking
on arbitrary or truncated input, returning structured errors instead.

#### Scenario: arbitrary input to the new parsers

- **GIVEN** arbitrary byte input
- **WHEN** `read_su`, `parse_seg_info`, or `parse_tile_params` is run over it
- **THEN** no library panic, unwrap, or unreachable state SHALL occur.

### Requirement: SVLC descriptor parsing

`splot-core` SHALL parse AV2 `svlc()` descriptors using the AV2 v1.0.0 § 4.11.4 mapping
from `uvlc()` values to signed integers.

#### Scenario: zero value

- **WHEN** `uvlc()` returns `0`
- **THEN** `svlc()` SHALL return `0`.

#### Scenario: alternating signed values

- **WHEN** `uvlc()` returns `1`, `2`, `3`, or `4`
- **THEN** `svlc()` SHALL return `1`, `-1`, `2`, or `-2` respectively.

#### Scenario: truncated code

- **WHEN** the `uvlc()` prefix is truncated or has `leadingZeros >= 32`
- **THEN** `read_svlc()` SHALL return the typed parser error from `read_uvlc()` without
  panicking.

### Requirement: User-defined QM helper parsing

`splot-core` SHALL parse AV2 `user_defined_qm(level, t, plane)` (§ 5.4.11) as a shared
helper used by quantizer-matrix syntax, covering the three fundamental transform shapes
`Fundamental_Tx_Size[3] = { TX_8X8, TX_8X4, TX_4X8 }`.

#### Scenario: plane copy

- **WHEN** `plane > 0` and `qm_copy_from_previous_plane` is set
- **THEN** the parser SHALL copy the previously parsed plane matrix and return without
  reading new coefficient deltas.

#### Scenario: 4x8 transpose

- **WHEN** the `TX_4X8` matrix signals `qm_4x8_is_transpose_of_8x4`
- **THEN** the parser SHALL fill it as the transpose of the same plane's `TX_8X4` matrix.

#### Scenario: user-defined coefficients

- **WHEN** a matrix is neither copied nor transposed
- **THEN** the parser SHALL read coefficient deltas with `svlc()` in AV2 2D diagonal
  scan order and apply coefficient-repeat behavior when `quant2 == 0`.

### Requirement: Quantizer Matrix OBU parsing

`splot-core` SHALL parse `OBU_QUANTIZATION_MATRIX` payloads using AV2
`quantizer_matrix_obu()` syntax (§ 5.13) and dispatch them from `open_bitstream_unit()`.

#### Scenario: reset/default QM OBU

- **WHEN** `qm_bit_map == 0`
- **THEN** the parser SHALL record the reset/default path and SHALL NOT read per-level
  matrix payloads.

#### Scenario: user-defined QM level

- **WHEN** a level bit is set and `qm_is_default_flag == 0`
- **THEN** the parser SHALL read all `user_defined_qm(level, t, plane)` structures for
  the selected plane count.

### Requirement: Film Grain OBU parsing

`splot-core` SHALL parse `OBU_FILM_GRAIN` payloads using AV2 `film_grain_obu()` syntax
(§ 5.14) and dispatch them from `open_bitstream_unit()`.

#### Scenario: updated film-grain slot

- **WHEN** bit `i` of `fgm_update_flags` is set
- **THEN** the parser SHALL read one `film_grain_model(monochrome, subX, subY)` and
  associate it with slot `i`.

### Requirement: Film grain model parsing

`splot-core` SHALL parse `film_grain_model()` syntax (§ 5.18.10.2), preserving the
fields needed for inspection and future frame-reference checks.

#### Scenario: scaling points and AR coefficients

- **WHEN** a model has non-zero luma/chroma scaling points and a non-zero
  `ar_coeff_lag`
- **THEN** the parser SHALL read the cumulative scaling points and the de-biased AR
  coefficient arrays for the derived position counts.

### Requirement: Padding OBU parsing

`splot-core` SHALL parse `padding_obu()` (AV2 v1.0.0 § 5.16) into typed syntax using the
§ 5.16 / § 6.15 rule that the last non-zero payload byte begins `trailing_bits()`,
surfacing it as `ParsedObu::Padding`. The padding parser SHALL consume the whole payload
(padding bytes plus its own trailing bits); dispatch SHALL NOT additionally run the
shared OBU trailing-bits logic for `OBU_PADDING`.

#### Scenario: empty padding payload

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 0`
- **WHEN** `parse_padding_obu()` reads it
- **THEN** it SHALL return a padding length of 0 and a trailing length of 0.

#### Scenario: one-byte trailing-only payload

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 1` whose single byte is valid
  `trailing_bits()`
- **WHEN** the parser reads it
- **THEN** it SHALL return a padding length of 0 and a trailing length of 1.

#### Scenario: arbitrary padding bytes

- **GIVEN** an `OBU_PADDING` whose payload is arbitrary non-zero `obu_padding_byte`
  values followed by valid `trailing_bits()`
- **WHEN** the parser reads it
- **THEN** it SHALL accept the padding bytes and parse the trailing bits from the last
  non-zero byte.

#### Scenario: all-zero payload rejected

- **GIVEN** a non-empty `OBU_PADDING` payload whose bytes are all zero
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured `padding/all-zero-payload` error
- **AND** SHALL NOT panic.

### Requirement: Metadata OBU parsing

`splot-core` SHALL parse `metadata_short_obu()` (AV2 v1.0.0 § 5.17.2) and
`metadata_group_obu()` (§ 5.17.3) into typed syntax, reading the 1-byte metadata unit
header fields, `metadata_type` (`leb128()`, retaining `Leb128Bytes`), and the shared
`metadata_unit()` (§ 5.17.1), surfacing them as `ParsedObu::MetadataShort` /
`ParsedObu::MetadataGroup` finished with `trailing_bits()` only (metadata OBUs are not
extensible). It SHALL never read past the OBU boundary and SHALL never panic on
arbitrary input.

#### Scenario: cancelled short metadata

- **GIVEN** a `metadata_short_obu()` with `muh_cancel_flag == 1`
- **WHEN** `parse_metadata_short()` reads it
- **THEN** it SHALL return after `metadata_type` with no metadata unit
- **AND** SHALL leave the reader positioned for the OBU `trailing_bits()`.

#### Scenario: short payload size underflow

- **GIVEN** a `metadata_short_obu()` whose `obuPayloadSize` is smaller than
  `2 + Leb128Bytes`
- **WHEN** the parser computes `metadataPayloadSize`
- **THEN** it SHALL return a `metadata/unit-payload-underflow` error rather than
  underflowing.

#### Scenario: group unit count too large

- **GIVEN** a `metadata_group_obu()` with `metadata_unit_cnt_minus_1 >= 16383`
- **WHEN** the parser reads it
- **THEN** it SHALL return a `metadata/group-unit-count-too-large` error.

#### Scenario: group header underflow

- **GIVEN** a non-cancelled group unit whose `muh_header_size` is too small to account
  for `Leb128Bytes`, the fixed header fields, and the layer maps
- **WHEN** the parser decrements `headerRemainingBytes`
- **THEN** it SHALL return a `metadata/group-header-underflow` error rather than
  underflowing.

### Requirement: Bounded metadata unit parsing

`splot-core` SHALL parse `metadata_unit(metadataPayloadSize)` (AV2 v1.0.0 § 5.17.1)
bounded to exactly `metadataPayloadSize` bytes via a sub-reader, parsing the typed
§ 5.17.4-§ 5.17.13 child payload selected by `metadata_type`, preserving reserved /
unknown / private types as raw (length only), and treating `metadata_unit_remaining_bit`
as ignorable padding (any value).

#### Scenario: child syntax bounded to the declared size

- **GIVEN** a `metadata_unit()` whose `metadataPayloadSize` is smaller than its child
  syntax needs
- **WHEN** the parser reads the child payload
- **THEN** it SHALL return a `metadata/unit-payload-underflow` error rather than reading
  into the OBU trailing bits or the next unit.

#### Scenario: unknown metadata type preserved as raw

- **GIVEN** a `metadata_unit()` with a reserved or private `metadata_type`
- **WHEN** the parser reads it
- **THEN** it SHALL preserve the raw payload length and SHALL NOT return
  `Unimplemented`.

#### Scenario: remaining bits any value

- **GIVEN** a `metadata_unit()` whose child payload is shorter than `metadataPayloadSize`
  and whose `metadata_unit_remaining_bit` bits are non-zero
- **WHEN** the parser reads it
- **THEN** it SHALL accept the unit (the remaining bits are ignorable).

### Requirement: Dispatch and inspect padding and metadata

`open_bitstream_unit` dispatch SHALL route `OBU_PADDING`, `OBU_METADATA_SHORT`, and
`OBU_METADATA_GROUP` to their parsers and remove them from the unimplemented branch, and
`splot inspect --json` SHALL surface the parsed payloads, summarizing raw payload lengths
rather than dumping bytes.

#### Scenario: inspector surfaces padding and metadata

- **GIVEN** a bitstream containing an `OBU_PADDING`, an `OBU_METADATA_SHORT`, and an
  `OBU_METADATA_GROUP`
- **WHEN** `splot inspect --json` reads it
- **THEN** the output SHALL include a `padding` view with padding/trailing lengths and
  `metadata_short` / `metadata_group` views with the header fields and per-unit metadata
  types and payload sizes
- **AND** SHALL NOT dump unbounded raw metadata payload bytes.

### Requirement: Frame header parse modes

The bitstream parser SHALL expose a mode that preserves the existing activation-prefix parse and a mode that parses additional frame-header core fields.

#### Scenario: Activation-prefix mode is compatible

- **GIVEN** a frame-bearing OBU payload
- **WHEN** the parser is called in activation-prefix mode
- **THEN** it SHALL read only the activation/reference fields currently consumed by the existing parser
- **AND** it SHALL return a status indicating activation-only coverage.

### Requirement: Frame-header core status

Every frame-header parse result SHALL carry an explicit parse status.

#### Scenario: Parser stops before deep frame syntax

- **GIVEN** a frame header that reaches unimplemented filtering, quantization, segmentation, tiling, transform, global-motion, or frame-film-grain syntax
- **WHEN** the parser stops before that syntax
- **THEN** the result SHALL indicate a partial status
- **AND** callers SHALL NOT infer that full trailing bits were validated.

### Requirement: Explicit state dependencies

The frame-header core parser SHALL receive active sequence, MFH, temporal-unit, and reference-state inputs explicitly.

#### Scenario: Required state is missing

- **GIVEN** a syntax branch requiring unavailable reference-frame state
- **WHEN** the parser reaches that branch
- **THEN** it SHALL return a typed unsupported/partial status or typed error
- **AND** it SHALL NOT guess reference counts, order hints, frame sizes, or validity.

### Requirement: Frame-size helper foundation

The parser SHALL model frame size as a typed value when dimensions can be derived exactly from parsed state.

#### Scenario: Frame size exceeds active sequence maximum

- **GIVEN** a parsed frame size
- **AND** an active sequence maximum frame size
- **WHEN** the frame size exceeds the sequence maximum
- **THEN** the validator SHALL be able to report a structured diagnostic.

### Requirement: Operating point set OBU parsing

`splot-core` SHALL parse `operating_point_set_obu()` (AV2 v1.0.0 § 5.10) and its
`operating_point_payload()` children (§ 5.11, § 5.11.1-§ 5.11.5) into typed syntax,
dispatching on `obu_xlayer_id` to the global and local branches, reading the full
nested syntax (no skipped bits), never reading past the OBU boundary, and retaining
reserved-zero fields. It SHALL surface an `operating_point_set_obu()` as
`ParsedObu::OperatingPointSet` finished with the extensible OBU tail.

#### Scenario: reset-only OPS

- **GIVEN** an OPS OBU with `ops_cnt == 0`
- **WHEN** `parse_operating_point_set()` reads it
- **THEN** it SHALL return a record with no operating point payloads
- **AND** SHALL NOT read the optional header fields.

#### Scenario: payload size accounting

- **GIVEN** an `operating_point_payload()` that declares `ops_data_size`
- **WHEN** the parser reads it
- **THEN** it SHALL preserve the declared `ops_data_size` and the computed `opsBytes`
  measured from after `ops_data_size` through the closing `byte_alignment()`.

#### Scenario: reserved values retained

- **GIVEN** a local OPS with a non-zero `ops_reserved_2bits`, or a global OPS with
  `ops_mlayer_info_idc == 3`
- **WHEN** the parser reads it
- **THEN** it SHALL retain the value for the validator rather than returning an error.

#### Scenario: truncated input

- **GIVEN** an OPS OBU truncated mid-syntax
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured error
- **AND** SHALL NOT panic.

### Requirement: Buffer removal timing OBU parsing

`splot-core` SHALL parse `buffer_removal_timing_obu()` (AV2 v1.0.0 § 5.12) into typed
syntax in both forms selected by `br_ops_dependent_flag`, surfacing it as
`ParsedObu::BufferRemovalTiming` finished with `trailing_bits()` only (the OBU is not
extensible).

#### Scenario: extended-layer BRT

- **GIVEN** a BRT OBU with `br_ops_dependent_flag == 0`
- **WHEN** `parse_buffer_removal_timing()` reads it
- **THEN** it SHALL parse a single `br_time` and no per-operating-point records.

#### Scenario: OPS-dependent BRT

- **GIVEN** a BRT OBU with `br_ops_dependent_flag == 1`
- **WHEN** the parser reads it
- **THEN** it SHALL parse `br_ops_id`, `br_ops_cnt`, and each per-operating-point
  present flag with its optional `br_time_op`.

### Requirement: Dispatch and inspect OPS and BRT

`open_bitstream_unit` dispatch SHALL route `OBU_OPERATING_POINT_SET` and
`OBU_BUFFER_REMOVAL_TIMING` to their parsers, and `splot inspect --json` SHALL surface
the parsed payloads.

#### Scenario: inspector surfaces OPS and BRT

- **GIVEN** a bitstream containing an `operating_point_set_obu()` and a
  `buffer_removal_timing_obu()`
- **WHEN** `splot inspect --json` reads it
- **THEN** the output SHALL include an `operating_point_set` view and a
  `buffer_removal_timing` view with the key parsed fields.

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

### Requirement: Intra-path frame-header stop point
The intra-path parser SHALL read the § 5.18.2 lossless / `allow_tcq` /
`allow_parity_hiding` tail and the loop-filter cluster
`deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2), `gdf_params()` (§ 5.18.7.9),
`cdef_params()` (§ 5.18.7.10), `lr_params()` (loop restoration, § 5.18.7.11), and
`ccso_params()` (§ 5.18.7.12), then continue into the § 5.18.2 tail (see the
*complete intra frame-header parsing* requirement) rather than stopping at this
point. When an `lr_params()` plane signals a frame-level Wiener filter, the parser
SHALL instead stop before the unmodeled `read_wienerns_filter()` frame-level Wiener
bank decode, naming the missing coverage. No full-payload trailing-bits conformance
SHALL be inferred from a partial status.

#### Scenario: Wiener bank decode is the cluster stop point
- **WHEN** a valid intra frame header parses through the loop-restoration cluster
  and an `lr_params()` plane signals a frame-level Wiener filter
- **THEN** the parse status names the unparsed `read_wienerns_filter()` decode and
  `consumed_bits` covers exactly the parsed prefix

### Requirement: New frame parsers never panic
All new frame-header parsing paths SHALL return typed errors on truncated or
malformed input — tile info, quantization, QM setup, segmentation, delta-q,
and lossless derivation — and SHALL be covered by property tests over
arbitrary byte slices, with positive, negative, and EOF unit tests for each
structure.

#### Scenario: Property test over arbitrary input
- **WHEN** the frame-header core parser runs over arbitrary bytes and arbitrary
  sequence-state inputs
- **THEN** it never panics and never reads past the payload bounds

### Requirement: Container-aware bitstream entry point

`splot-core` SHALL expose a container-aware bitstream entry point that returns raw
Annex B streams and IVF-wrapped Annex B streams through a single typed result. The
entry point SHALL preserve the existing Annex B envelope parser behavior and SHALL
only add format detection and container metadata.

#### Scenario: Existing Annex B parser is unchanged

- **WHEN** callers invoke the raw Annex B parser directly
- **THEN** it SHALL continue to parse only length-delimited OBUs
- **AND** SHALL NOT require an IVF header or frame record.

#### Scenario: Container parser preserves offsets

- **WHEN** callers invoke the container-aware parser on an IVF file
- **THEN** parsed OBU byte offsets SHALL be relative to the original file
- **AND** SHALL NOT be rebased to frame-payload-local offsets.

### Requirement: MFH-backed frame-header parsing

The frame-header core parser SHALL consume the resolved in-band
multi-frame header's parsed § 5.7 state on `cur_mfh_id > 0` paths: the
§ 5.18.4 default frame dimensions come from
`mfh_frame_width/height_minus_1[ cur_mfh_id ]` (with the § 5.7 omitted-size
inference to the sequence maxima), and § 5.18.7.1 `segmentation_params()`
parses its `mfh_seg_info_present_flag` / `mfh_ext_seg_flag` /
`mfh_allow_seg_info_change` gated arms. A frame whose referenced
multi-frame header is not resolvable in-band SHALL keep the existing
unsupported/Unknown routing rather than guessing field positions.

#### Scenario: MFH default dimensions

- **WHEN** an intra frame with `cur_mfh_id > 0` and
  `frame_size_override_flag == 0` references an in-band MFH carrying
  explicit dimensions
- **THEN** the parse continues through `tile_info()` with the MFH
  dimensions instead of stopping

#### Scenario: MFH-gated segmentation arms

- **WHEN** the referenced in-band MFH has `mfh_seg_info_present_flag == 1`
- **THEN** `segmentation_params()` parses the MFH-gated arm per § 5.18.7.1
  instead of stopping before it

#### Scenario: unresolvable MFH stays unsupported

- **WHEN** a frame references a `cur_mfh_id` with no resolvable in-band
  multi-frame header
- **THEN** the parse stops as before and dependent judgments stay Unknown

### Requirement: non-uniform sequence tile reuse

The sequence-header parse SHALL persist the § 5.4.2 derived
`SeqSbColStarts` / `SeqSbRowStarts` arrays, and the § 5.18.7.4
`reuse_tile_params()` path SHALL consume them so a frame reusing a
non-uniform sequence tile layout parses through `tile_info()` instead of
stopping as unimplemented.

#### Scenario: non-uniform reuse parses

- **WHEN** a frame sets `reuse_tile_info == 1` against an in-band sequence
  header with non-uniform tile spacing
- **THEN** `tile_info()` parses using the recorded start arrays

#### Scenario: uniform path unchanged

- **WHEN** a frame reuses a uniform sequence tile layout
- **THEN** parsing behaves exactly as before

### Requirement: intra-path filter parameter parsing

The frame-header core parser SHALL parse `deblocking_filter_params()`
(§ 5.18.5.2, including the `cur_mfh_id > 0` arms consulting the resolved
multi-frame header's `mfh_deblocking_filter_update` /
`mfh_apply_deblocking_filter`), `gdf_params()` (§ 5.18.7.9), and
`cdef_params()` (§ 5.18.7.10) on the intra path, gated on the parsed
§ 5.4.10 sequence filter configuration, and SHALL advance its stop status
past them to the next unparsed structure. A frame whose referenced
multi-frame header is not resolvable in-band SHALL keep the existing
unsupported routing.

#### Scenario: intra frame parses filter params

- **WHEN** an intra frame header reaches the § 5.18.2 tail with a parsed
  sequence filter configuration
- **THEN** the deblocking, GDF, and CDEF parameters are parsed and the
  stop status names the next unparsed structure

#### Scenario: MFH deblocking arm

- **WHEN** a `cur_mfh_id > 0` frame's resolved MFH sets
  `mfh_deblocking_filter_update == 1`
- **THEN** the § 5.18.5.2 MFH arm is parsed per the mirror

#### Scenario: EOF inside filter params

- **WHEN** the payload ends inside any of the three loop-filter structures
- **THEN** the parser reports the truncation without panicking, preserves the
  already-parsed control-region facts (frame size, output flags, tile / quant /
  segmentation), leaves the unreached filter fields unset, and records the
  truncation through a dedicated stop status rather than failing the whole parse

### Requirement: intra-path loop-restoration and CCSO parsing

The frame-header core parser SHALL parse `lr_params()` (§ 5.18.7.11) and
`ccso_params()` (§ 5.18.7.12) on the intra path, gated on the parsed
sequence restoration/CCSO configuration, and SHALL continue into the § 5.18.2
tail (`read_tx_mode()`, § 5.18.8.1, and beyond — see the *complete intra
frame-header parsing* requirement). When an `lr_params()` plane signals a
frame-level Wiener filter, the parser SHALL stop honestly before the unmodeled
`read_wienerns_filter()` bank decode, naming the missing coverage and preserving
the pre-Wiener facts. An EOF inside the new cluster SHALL preserve the
already-parsed frame facts.

#### Scenario: intra frame parses lr and ccso params

- **WHEN** an intra frame header reaches the post-CDEF tail with the
  gating sequence configuration parsed
- **THEN** the loop-restoration and CCSO parameters are parsed and parsing
  continues into the § 5.18.2 tail

#### Scenario: frame-level Wiener filter stops honestly

- **WHEN** an `lr_params()` plane signals `frame_filters_on`
- **THEN** the parser stops before `read_wienerns_filter()` with a named
  missing-coverage status, and the partial `lr_params()` facts parsed before
  the stop (per-plane restoration types, `frame_filters_on`, the luma
  `NumFilterClasses`, `UsesLr`, and `LoopRestorationSize`) are surfaced on a
  dedicated partial field — distinct from the complete-parse field so a
  partial parse is never mistaken for a complete one

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside `lr_params()` or `ccso_params()`
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation

### Requirement: complete intra frame-header parsing

The frame-header core parser SHALL parse the remaining § 5.18.2 intra
tail — `read_tx_mode()` (§ 5.18.8.1), `frame_reference_mode()`
(§ 5.18.8.3, no bits on intra), `skip_mode_params()` (§ 5.18.8.2), the
intra-inferred `allow_bawp`/`allow_warpmv_mode`, `reduced_tx_set`, the
§ 5.18.9.1 intra arm of `global_motion_params()`, and
`film_grain_config()` (§ 5.18.10.1) — so an intra frame header parses to
completion, the show-existing-frame path included. Within
`film_grain_config()` the `load_grain_model( fgm_id )` call reads **no
bits** (§ 6.17.10.1): it is a memory-load reference to a film-grain model
slot previously decoded by a `film_grain_obu()` (§ 5.14), so the § 5.14
film-grain-model parser is **not** invoked from the frame-header path —
only the in-band `apply_grain`, `fgm_id`, and `grain_seed` fields are read.
An EOF inside the tail SHALL preserve the already-parsed facts.

#### Scenario: intra header completes

- **WHEN** an intra frame header parses through its § 5.18.2 tail
- **THEN** the status reports a complete header and every tail structure
  is surfaced

#### Scenario: SEF completes

- **WHEN** a show-existing-frame header parses through film_grain_config
- **THEN** its status reports completion instead of stopping early

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside the new tail structures
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation

### Requirement: frame header copy accounting

The frame-header parser SHALL record `NumFrameHeaderBits` when a first
frame header's `frame_header_info()` parses to completion, and a
non-first tile group of the same coded frame SHALL have its
`frame_header_copy()` region parsed as exactly that many bits and
compared bit-for-bit against the first header. A first header that did
not parse to completion SHALL leave the copy region unparsed (Unknown
routing).

#### Scenario: copy region parses and matches

- **WHEN** a non-first tile group follows a completed intra first header
- **THEN** its header-copy bits are consumed and verified bit-identical

#### Scenario: copy mismatch is flagged

- **WHEN** the copy region differs from the first header's bits
- **THEN** a diagnostic with the governing citation is emitted

#### Scenario: incomplete first header keeps Unknown routing

- **WHEN** the first header's parse stopped before completion
- **THEN** the non-first tile group's copy region is left unparsed as
  today

### Requirement: tile-group structure parsing

The tile-group parser SHALL parse the § 5.19 remainder after the frame
header on intra-complete paths — `tile_start_and_end_present_flag`
(gated on `NumTiles > 1` from the parsed tile layout),
`tg_start`/`tg_end` at `TileColsLog2 + TileRowsLog2` bits,
`byte_alignment()`, and the `headerBytes` payload-boundary handoff — and
SHALL validate the locally-decidable tile-group range semantics with
their governing citations. Frames whose `use_bru`/`bru_inactive` cannot
be derived SHALL stop honestly before the BRU arms; the § 5.20 payload
itself stays unparsed.

#### Scenario: intra tile group parses its structure

- **WHEN** an intra-complete first tile group's frame header is followed
  by the § 5.19 remainder
- **THEN** the tg range and payload boundary are parsed and surfaced

#### Scenario: tg range violation is flagged

- **WHEN** the parsed tg range violates a governing § 6 clause
- **THEN** a diagnostic with that citation is emitted

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside the new region
- **THEN** the already-parsed facts survive and the truncation surfaces
  per the established pattern

### Requirement: inter frame-header control-region parsing

The frame-header core parser SHALL parse the § 5.18.2 inter/TIP/bridge/
switch control region — primary-reference signaling, the inter refresh
branches, the explicit reference map, the BRU triple, ref-mvs/TMVP, the
TIP block, DRL and MV-precision fields, motion modes,
`read_interpolation_filter()` (§ 5.18.5.1), the with-refs and with-bridge
frame sizes (§ 5.18.4.2/.3), and the § 5.18.3 reference-distance
derivations — gated on the parsed sequence configuration and the modeled
reference state, converging into the shared tail. A branch whose
reference-state inputs are poisoned SHALL stop honestly with facts
preserved; locally decidable § 6 clauses on the new fields SHALL carry
their citations.

#### Scenario: inter header parses its control region

- **WHEN** an inter frame follows reference-state-grounded intra frames
- **THEN** its control region parses through the shared tail

#### Scenario: poisoned reference state stops honestly

- **WHEN** an inter branch needs slot facts the model has poisoned
- **THEN** the parse stops at that branch with earlier facts preserved

#### Scenario: invalid reference index is flagged

- **WHEN** a parsed `ref_frame_idx` references a slot the modeled state
  proves invalid
- **THEN** the diagnostic with its governing citation is emitted

