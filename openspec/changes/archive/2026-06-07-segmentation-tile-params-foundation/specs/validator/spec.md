# validator spec delta

## ADDED Requirements

### Requirement: Sequence headers with segment/tile info run payload-tail validation

`splot-validate` SHALL run the §5.2.1 payload-tail check (`obu_extension_flag` +
`trailing_bits`) on a sequence header whose `seg_info()` and `tile_params()` now parse
fully, so a truncated or malformed payload after the segment or tile info is reported
rather than silently accepted by an early bound.

#### Scenario: malformed tail after segment info is diagnosed

- **GIVEN** a sequence header with `seq_seg_info_present_flag` equal to 1 followed by a
  malformed §5.2.1 payload tail
- **WHEN** the validator runs
- **THEN** it SHALL emit a payload-tail conformance diagnostic
  (`trailing-bits/*`, `byte-alignment/*`, `obu-header/extension-flag-not-zero`, or
  `bitstream/parse-error`).

### Requirement: Multi-frame-header availability is gated on full tail validation

`splot-validate` SHALL run the §5.2.1 payload-tail check on a multi-frame header whose
`seg_info()` now parses fully, and SHALL record its availability only when the tail is
valid, so a later `cur_mfh_id` reference resolves only against well-formed multi-frame
headers.

#### Scenario: multi-frame header with segment info is recorded when well-formed

- **GIVEN** a multi-frame header with `mfh_seg_info_present_flag` equal to 1 and a valid
  payload tail
- **WHEN** the validator runs
- **THEN** it SHALL NOT report it bounded
- **AND** it SHALL record it as an available high-level-syntax object.

### Requirement: Sequence tile-params local constraints are checked

`splot-validate` SHALL check the local §6.17.7 tile constraints on a fully parsed
sequence tile config and emit the corresponding diagnostics.

#### Scenario: too many tile columns

- **GIVEN** a non-uniform sequence tile config whose derived tile columns exceed
  `MAX_TILE_COLS`
- **WHEN** the validator runs
- **THEN** it SHALL emit `tile-params/tile-cols-out-of-range`.

#### Scenario: too many tile rows

- **GIVEN** a non-uniform sequence tile config whose derived tile rows exceed
  `MAX_TILE_ROWS`
- **WHEN** the validator runs
- **THEN** it SHALL emit `tile-params/tile-rows-out-of-range`.

#### Scenario: non-uniform tiles do not cover the frame

- **GIVEN** a non-uniform sequence tile config whose tile column or row starts do not
  sum to the frame size in superblocks
- **WHEN** the validator runs
- **THEN** it SHALL emit `tile-params/nonuniform-cols-do-not-cover-frame` or
  `tile-params/nonuniform-rows-do-not-cover-frame` respectively.

#### Scenario: valid tile config is accepted

- **GIVEN** a valid uniform or non-uniform sequence tile config that covers the frame
  within the tile-count limits
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `tile-params/*` diagnostic.

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
