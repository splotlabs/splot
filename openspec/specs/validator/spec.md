# validator Specification

## Purpose

Parser-driven conformance diagnostics in `splot-validate`. Diagnostics are the
product: every finding is structured data (stable `rule_id`, `severity`, optional
`spec_section`, optional byte/bit offset, human-readable `message`). A malformed
bitstream is a report, never a process failure.

Tracked by Feature IDs: `AV2-5.2.2-OBU-HEADER` (header constraints),
`AV2-5.3-RESERVED-OBU`, `AV2-5.18.2-FRAME-HEADER-INFO`,
`AV2-5.18.7.3-TILE-PARAMS`, `AV2-5.19-TILE-GROUP`,
`AV2-7.3-OBU-ORDERING`, and `AV2-7.3.8-HLS-AVAILABILITY`.
## Requirements
### Requirement: structured diagnostics

Every check SHALL emit `Diagnostic`s with a stable `rule_id`, a `severity`, the AV2
`spec_section` where applicable, and a byte offset where known.

#### Scenario: global xlayer constraint

- **WHEN** an `OBU_TEMPORAL_DELIMITER` has `obu_xlayer_id != GLOBAL_XLAYER_ID`
- **THEN** an error diagnostic `obu-header/global-xlayer-required` (§ 6.2.2) is produced

### Requirement: reserved OBU handling

A reserved OBU SHALL be reported informationally; a reserved OBU whose payload is
entirely zero SHALL be an error (AV2 v1.0.0 § 5.3 / § 6.2.3 require a non-zero
trailing bit).

#### Scenario: all-zero reserved payload

- **WHEN** a reserved OBU carries an entirely-zero payload
- **THEN** an error diagnostic `obu-reserved/all-zero-payload` is produced

### Requirement: diagnostic rule-id namespace

Diagnostic rule ids SHALL use a documented kebab/slash prefix (`obu-header/`,
`obu-reserved/`, `bitstream/`). Narrower diagnostics derived from a modeled feature
MAY use the Feature ID as a base with a `.SUFFIX`.

#### Scenario: undocumented prefix is rejected

- **WHEN** a diagnostic rule id uses a prefix that is not documented
- **THEN** `cargo xtask check-feature-status` fails

### Requirement: Sequence header semantic diagnostics

`splot-validate` SHALL emit stable diagnostics for locally decidable §6.4 sequence-header semantic violations covered by implemented parsers.

#### Scenario: zero timing values

- **GIVEN** a sequence header with timing information present
- **WHEN** `num_units_in_display_tick == 0` or `time_scale == 0`
- **THEN** validation SHALL emit a `sequence-header/` diagnostic with severity `error` and the relevant AV2 section.

### Requirement: Activated sequence layer limits

`splot-validate` SHALL use available activated sequence headers to validate OBU layer identifiers.

#### Scenario: temporal layer exceeds active sequence maximum

- **GIVEN** an active sequence header for an extended layer
- **AND** a subsequent non-global OBU associated with that layer
- **WHEN** the OBU has `obu_tlayer_id > max_tlayer_id`
- **THEN** validation SHALL emit `sequence-state/tlayer-exceeds-max`.

#### Scenario: embedded layer exceeds active sequence maximum

- **GIVEN** an active sequence header for an extended layer
- **AND** a subsequent non-global OBU associated with that layer
- **WHEN** the OBU has `obu_mlayer_id > max_mlayer_id`
- **THEN** validation SHALL emit `sequence-state/mlayer-exceeds-max`.

### Requirement: HLS availability state

`splot-validate` SHALL model in-band HLS availability before an OBU references sequence/HLS state.

#### Scenario: unavailable sequence header

- **GIVEN** an OBU or HLS object references a sequence-header id
- **AND** no matching in-band or caller-provided external sequence header is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `hls/unavailable-sequence-header`.

### Requirement: Temporal-unit ordering

`splot-validate` SHALL continue to enforce the implemented subset of AV2 temporal-unit order.

#### Scenario: duplicate temporal delimiter

- **GIVEN** a temporal unit with a global temporal delimiter already seen
- **WHEN** another global temporal delimiter appears before the next temporal unit begins
- **THEN** validation SHALL emit `obu-order/duplicate-temporal-delimiter`.

### Requirement: Layer configuration record and atlas syntax checks

`splot-validate` SHALL run stateless syntax checks over `OBU_LAYER_CONFIGURATION_RECORD`
and `OBU_ATLAS_SEGMENT` payloads, surfacing parse and range violations as `lcr/*` and
`atlas/*` diagnostics and warning on a non-zero reserved-zero field (AV2 § 6.8).

#### Scenario: non-zero reserved field

- **GIVEN** a layer configuration record whose `lcr_global_reserved_zero_5bits` is
  non-zero
- **WHEN** the validator runs
- **THEN** it SHALL emit a `lcr/reserved-bits-nonzero` warning.

#### Scenario: out-of-range atlas mode

- **GIVEN** an atlas segment OBU with `ats_atlas_segment_mode_idc` greater than 4
- **WHEN** the validator runs
- **THEN** it SHALL emit an `atlas/segment-mode-out-of-range` error.

### Requirement: Layer configuration record and atlas availability

`splot-validate` SHALL track in-band layer-configuration-record and local
atlas-segment availability and emit diagnostics when a reference cannot be resolved
(AV2 § 7.3.8.3 / § 7.3.8.4), gating the hard errors on external HLS being disabled. The
global atlas (§ 7.3.8.4 "can be available") SHALL NOT be flagged when missing.

#### Scenario: local LCR references an unavailable global LCR

- **GIVEN** a local LCR whose `lcr_global_id` is non-zero and no preceding global LCR
  has that `lcr_global_config_record_id`
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `lcr/global-lcr-unavailable` error.

#### Scenario: local LCR references an unavailable local atlas

- **GIVEN** a local LCR whose `lcr_local_atlas_id` has no preceding local atlas segment
  OBU in the same extended layer
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit an `atlas/local-atlas-unavailable` error.

### Requirement: Sequence header `seq_lcr_id` resolution

`splot-validate` SHALL resolve a sequence header's `seq_lcr_id` (when non-zero) to an
available local LCR (same xlayer) or, failing that, an available global LCR whose
`lcr_xlayer_map` includes the sequence header's xlayer (AV2 § 6.4.1 / § 7.3.8.6).

#### Scenario: seq_lcr_id resolves to no LCR

- **GIVEN** a sequence header with `seq_lcr_id != 0` and no matching LCR
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `hls/unavailable-layer-configuration-record` error.

#### Scenario: global LCR omits the header's xlayer

- **GIVEN** a sequence header whose `seq_lcr_id` resolves to a global LCR whose
  `lcr_xlayer_map` does not include the header's `obu_xlayer_id`
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `lcr/global-xlayer-map-missing-xlayer` error.

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

### Requirement: Frame-header sequence-header references are checked

`splot-validate` SHALL check a parsed frame-header prefix that directly references
`seq_header_id_in_frame_header` and emit `hls/unavailable-sequence-header` (error,
§7.3.8.6) when the referenced sequence header is not available in-band or through
caller-supplied external HLS. An out-of-range `seq_header_id_in_frame_header` instead
emits `frame-header/seq-header-id-out-of-range` (error) and is not double-reported as
unavailable.

#### Scenario: missing direct sequence-header reference

- **GIVEN** a frame header with `cur_mfh_id` equal to 0
- **AND** `seq_header_id_in_frame_header` names a sequence header not available
  in-band or externally
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit `hls/unavailable-sequence-header`.

#### Scenario: direct sequence-header reference is available in-band

- **GIVEN** a frame header with `cur_mfh_id` equal to 0
- **AND** `seq_header_id_in_frame_header` names an available in-band sequence header
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit `hls/unavailable-sequence-header`.

#### Scenario: direct sequence-header reference is available externally

- **GIVEN** a frame header with `cur_mfh_id` equal to 0
- **AND** `seq_header_id_in_frame_header` is declared through external HLS
- **WHEN** the validator runs with `ExternalHlsMode::Provided`
- **THEN** it SHALL NOT emit `hls/unavailable-sequence-header`.

### Requirement: Frame-header multi-frame-header references are checked

`splot-validate` SHALL check a parsed frame-header prefix with `cur_mfh_id` greater
than 0 and emit `hls/unavailable-multi-frame-header` (error, §7.3.8.7) when the
referenced multi-frame header is not available in-band. An out-of-range `cur_mfh_id`
instead emits `frame-header/cur-mfh-id-out-of-range` (error) and is not
double-reported as unavailable.

#### Scenario: missing MFH reference

- **GIVEN** a frame header with `cur_mfh_id` greater than 0
- **AND** no available multi-frame header with that id
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit `hls/unavailable-multi-frame-header`.

#### Scenario: MFH reference is available

- **GIVEN** a frame header with `cur_mfh_id` greater than 0
- **AND** an available multi-frame header with that id referencing an available
  sequence header
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit `hls/unavailable-multi-frame-header`
- **AND** it SHALL use the multi-frame header's sequence-header reference for modeled
  activation checks.

### Requirement: Sequence activation uses parsed frame-header references

`splot-validate` SHALL use the sequence header referenced by a parsed CLK/OLK
frame-header prefix to update the active sequence state for the modeled extended
layer, overriding the OBU-order fallback.

#### Scenario: CLK activates a referenced sequence header

- **GIVEN** two available sequence headers with different layer limits
- **AND** a CLK frame header that references the second one
- **WHEN** the validator observes the CLK frame-header prefix
- **THEN** it SHALL activate the referenced sequence header for that extended layer
- **AND** subsequent layer-limit checks SHALL use the referenced header, not the
  OBU-order fallback.

### Requirement: In-CVS repeated sequence header remains detectable across activation

`splot-validate` SHALL keep a CVS-opening sequence header's fingerprint across the
activating CLK within a temporal unit, so a non-identical repeat of that sequence
header later in the temporal unit is still flagged as
`hls/repeated-sequence-header-not-identical`.

#### Scenario: non-identical repeat after the activating CLK

- **GIVEN** a sequence header that opens a coded video sequence
- **AND** a CLK frame header that references it
- **AND** a later sequence header in the same temporal unit reusing the id with
  different payload bytes
- **WHEN** the validator observes the later sequence header
- **THEN** it SHALL emit `hls/repeated-sequence-header-not-identical`.

### Requirement: Prefix-only parse is not full frame conformance

`splot-validate` SHALL NOT treat full frame-header or tile-group parsing as complete
because the prefix skeleton parsed the activation/reference fields, and SHALL NOT emit
a full-payload trailing-bits diagnostic for a prefix-only parse.

#### Scenario: prefix parser stops after activation fields

- **GIVEN** a frame header that contains additional unparsed §5.18 syntax after the
  activation/reference fields
- **WHEN** the prefix parser reaches its designed stopping point
- **THEN** the parsed-payload summary SHALL report a prefix-only status
- **AND** the implementation matrix SHALL remain partial for full frame-header
  coverage.
