# bitstream spec delta

## ADDED Requirements

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

### Requirement: Segment information `seg_info(numSegments)`

`splot-core` SHALL expose a reusable `seg_info(numSegments)` parser (AV2 v1.0.0
§5.4.9) that, for each segment and each of the `SEG_LVL_MAX` features, reads
`feature_enabled` and, when enabled, the feature value using the exact
`Segmentation_Feature_Bits`, `Segmentation_Feature_Signed`, and
`Segmentation_Feature_Max` tables — signed features via `su(1 + bitsToRead)` clipped to
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

### Requirement: Multi-frame header parses `seg_info()`

`splot-core` SHALL parse `seg_info(mfh_ext_seg_flag ? 16 : 8)` inside
`multi_frame_header_obu()` (AV2 v1.0.0 §5.7) when `mfh_seg_info_present_flag` is 1, and
SHALL no longer mark the multi-frame header bounded at `seg_info()`.

#### Scenario: multi-frame header segment info present

- **GIVEN** a multi-frame header with `mfh_seg_info_present_flag` equal to 1
- **WHEN** the multi-frame header is parsed
- **THEN** the parsed result SHALL contain the parsed `seg_info()`
- **AND** SHALL NOT report a `seg_info()` bound.

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

### Requirement: Parsers never panic on arbitrary input

`splot-core` SHALL parse `su(n)`, `seg_info()`, and `tile_params()` without panicking
on arbitrary or truncated input, returning structured errors instead.

#### Scenario: arbitrary input to the new parsers

- **GIVEN** arbitrary byte input
- **WHEN** `read_su`, `parse_seg_info`, or `parse_tile_params` is run over it
- **THEN** no library panic, unwrap, or unreachable state SHALL occur.
