# bitstream spec delta

## ADDED Requirements

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
