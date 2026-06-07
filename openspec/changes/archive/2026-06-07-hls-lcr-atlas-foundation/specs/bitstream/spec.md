# bitstream spec delta

## ADDED Requirements

### Requirement: `leb128()` descriptor reader

`splot-core` SHALL expose a panic-free `BitReader::read_leb128()` that decodes the AV2
v1.0.0 § 4.11.4 `leb128()` descriptor (up to eight little-endian 7-bit groups with a
continuation bit), returning a structured error rather than panicking on a truncated,
overlong, or out-of-`u32`-range code.

#### Scenario: multi-byte value

- **GIVEN** the two bytes `0x80 0x01`
- **WHEN** `read_leb128()` reads them
- **THEN** it SHALL return `128`
- **AND** advance the reader by two bytes.

#### Scenario: truncated code

- **GIVEN** a single `0x80` byte (continuation bit set, no following byte)
- **WHEN** `read_leb128()` reads it
- **THEN** it SHALL return a structured end-of-input error
- **AND** SHALL NOT panic.

#### Scenario: overlong code

- **GIVEN** nine bytes each with the continuation bit set
- **WHEN** `read_leb128()` reads them
- **THEN** it SHALL return a structured invalid-LEB128 error.

### Requirement: Layer configuration record OBU parsing

`splot-core` SHALL parse `layer_config_record_obu()` (AV2 v1.0.0 § 5.8) into typed
syntax, dispatching on `obu_xlayer_id` to `lcr_global_info()` or
`lcr_local_info(obu_xlayer_id)`, reading the full nested syntax (including the
length-bounded `lcr_global_payload()`), never skipping payload bits, and never reading
past the OBU boundary. It SHALL retain reserved-zero fields rather than rejecting them,
and SHALL surface a `layer_config_record_obu()` as `ParsedObu::LayerConfigurationRecord`.

#### Scenario: minimal global record

- **GIVEN** a global LCR (`obu_xlayer_id == GLOBAL_XLAYER_ID`) with no optional sections
- **WHEN** the parser reads it
- **THEN** it SHALL return a global record exposing `lcr_global_config_record_id` and
  `lcr_xlayer_map`.

#### Scenario: global payload remaining bits

- **GIVEN** a global LCR with `lcr_global_payload_present_flag` set and an
  `lcr_data_size` larger than the parsed `lcr_xlayer_info`
- **WHEN** the parser reads the payload
- **THEN** it SHALL consume exactly `lcr_data_size * 8` bits, including the trailing
  `lcr_remaining_payload_bit` bits.

#### Scenario: payload size overflow

- **GIVEN** a global payload whose parsed content exceeds `lcr_data_size * 8` bits
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured layer-config-record error
- **AND** SHALL NOT panic.

#### Scenario: truncated record

- **GIVEN** a layer configuration record OBU that ends mid-field
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured end-of-input error
- **AND** SHALL NOT panic.

### Requirement: Atlas segment info OBU parsing

`splot-core` SHALL parse `atlas_segment_info_obu()` (AV2 v1.0.0 § 5.9) into typed
syntax for all five `ats_atlas_segment_mode_idc` modes plus `ats_label_segment_info()`,
never skipping payload bits and never reading past the OBU boundary. It SHALL
range-check `ats_atlas_segment_mode_idc` and the segment/region counts before looping,
returning a structured error rather than parsing an undefined mode or an unbounded
loop, and SHALL surface an `atlas_segment_info_obu()` as `ParsedObu::AtlasSegment`.

#### Scenario: single-mode atlas

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc == SINGLE_ATLAS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a record with `num_segments == 1` and the nominal
  dimensions.

#### Scenario: out-of-range mode

- **GIVEN** an atlas OBU with `ats_atlas_segment_mode_idc` greater than 4
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured atlas-segment error
- **AND** SHALL NOT panic.

#### Scenario: out-of-range segment count

- **GIVEN** an atlas OBU whose segment count reaches `MAX_NUM_ATLAS_SEGMENTS`
- **WHEN** the parser reads it
- **THEN** it SHALL return a structured atlas-segment error before iterating the
  segment loop.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
