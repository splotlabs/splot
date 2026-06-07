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
`AV2-5.19-TILE-GROUP`, plus the not-yet-parsed header/syntax rows in
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
