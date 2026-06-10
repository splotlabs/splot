# validator Specification

## Purpose

Parser-driven conformance diagnostics in `splot-validate`. Diagnostics are the
product: every finding is structured data (stable `rule_id`, `severity`, optional
`spec_section`, optional byte/bit offset, human-readable `message`). A malformed
bitstream is a report, never a process failure.

Tracked by Feature IDs: `AV2-5.2.2-OBU-HEADER` (header constraints),
`AV2-5.3-RESERVED-OBU`, `AV2-5.18.2-FRAME-HEADER-INFO`,
`AV2-5.18.7.3-TILE-PARAMS`, `AV2-5.19-TILE-GROUP`,
`AV2-5.15-CONTENT-INTERPRETATION`, `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`,
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

`splot-validate` SHALL model in-band HLS availability before an OBU references
sequence/HLS state, with optional caller-provided external HLS supplied through
`ValidationOptions` (AV2 v1.0.0 § 7.3.8). The default `ValidationOptions` SHALL NOT
assume any external HLS is available.

#### Scenario: unavailable sequence header

- **GIVEN** an OBU or HLS object references a sequence-header id
- **AND** no matching in-band or caller-provided external sequence header is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `hls/unavailable-sequence-header`.

#### Scenario: multi-frame header references an available sequence header

- **GIVEN** a sequence-header OBU with `seq_header_id` equal to id earlier in the
  bitstream
- **AND** a later multi-frame header OBU with `mfh_seq_header_id` equal to id
- **WHEN** validation reaches the reference
- **THEN** validation SHALL NOT emit `mfh/sequence-header-unavailable`.

#### Scenario: multi-frame header references an unavailable sequence header

- **GIVEN** a multi-frame header OBU with `mfh_seq_header_id` equal to id
- **AND** no in-band or caller-provided sequence header with that id is available
- **WHEN** validation reaches the reference
- **THEN** validation SHALL emit `mfh/sequence-header-unavailable`.

#### Scenario: external HLS provides the referenced sequence header

- **GIVEN** a multi-frame header OBU with `mfh_seq_header_id` equal to id
- **AND** no in-band sequence header with that id, but caller-provided external HLS
  declares id available
- **WHEN** validation runs with `ExternalHlsMode::Provided`
- **THEN** validation SHALL NOT emit `mfh/sequence-header-unavailable`.

#### Scenario: external HLS disabled advisory

- **GIVEN** a multi-frame header reference that cannot be satisfied in-band
- **WHEN** validation runs with the default `ExternalHlsMode::Disabled`
- **THEN** validation SHALL emit `mfh/sequence-header-unavailable`
- **AND** SHALL additionally emit the advisory `hls/external-hls-disabled`.

### Requirement: Cross-embedded-layer timing consistency

`splot-validate` SHALL compare timing information (`timing_info()`, reached through
the content-interpretation OBU) across embedded layers of the same coded video
sequence, and flag inconsistencies (AV2 v1.0.0 § 6.4.12). The comparison SHALL be
made only between two timing values that are both present and both decidably within
the same modeled coded-video-sequence scope.

#### Scenario: matching timing across embedded layers is accepted

- **GIVEN** two content-interpretation OBUs for the same extended layer but
  different embedded layers, both carrying timing information
- **WHEN** their `num_units_in_display_tick`, `time_scale`,
  `equal_picture_interval`, and `num_ticks_per_picture_minus_1` values are equal
- **THEN** validation SHALL NOT emit any `sequence-header/timing-*-mismatch`.

#### Scenario: mismatched display tick across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `num_units_in_display_tick` values differ
- **THEN** validation SHALL emit `sequence-header/timing-display-tick-mismatch`.

#### Scenario: mismatched time scale across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `time_scale` values differ
- **THEN** validation SHALL emit `sequence-header/timing-time-scale-mismatch`.

#### Scenario: mismatched equal-picture-interval across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information
- **WHEN** their `equal_picture_interval` values differ
- **THEN** validation SHALL emit
  `sequence-header/timing-equal-picture-interval-mismatch`.

#### Scenario: mismatched ticks-per-picture across embedded layers

- **GIVEN** two embedded layers in one coded video sequence that both carry timing
  information with `equal_picture_interval` equal to 1
- **WHEN** their `num_ticks_per_picture_minus_1` values differ
- **THEN** validation SHALL emit `sequence-header/timing-num-ticks-mismatch`.

#### Scenario: timing not yet comparable

- **GIVEN** at most one embedded layer carries present timing information in the
  modeled coded-video-sequence scope
- **WHEN** validation runs
- **THEN** the validator SHALL NOT fabricate a timing-mismatch diagnostic.

### Requirement: Content-interpretation range conformance

`splot-validate` SHALL enforce the locally-decidable § 6.14 range constraints of the
content-interpretation OBU.

#### Scenario: chroma sample position out of range

- **GIVEN** a content-interpretation OBU with `ci_chroma_sample_position_top` or
  `ci_chroma_sample_position_bottom` greater than 5
- **WHEN** validation runs
- **THEN** validation SHALL emit
  `content-interpretation/chroma-sample-position-out-of-range`.

#### Scenario: aspect ratio idc out of range

- **GIVEN** a content-interpretation OBU with `ci_aspect_ratio_idc` not equal to 255
  and greater than 16
- **WHEN** validation runs
- **THEN** validation SHALL emit
  `content-interpretation/aspect-ratio-idc-out-of-range`.

### Requirement: Repeated content-interpretation identity

`splot-validate` SHALL flag a content-interpretation OBU that is repeated for the
same embedded layer within the modeled coded-video-sequence scope carrying different
*information* (AV2 v1.0.0 § 6.14: a repeated CI OBU must "contain the same
information"). The comparison SHALL be sound and complete for the fields it covers:
it compares `ci_scan_type_idc`, the chroma sample position, `timing_info()`, and the
**derived** color description and aspect ratio (§ 6.14 Table 6.13 / § 5.15 aspect
tables, resolving presets, reserved ids, and absence to canonical values including
the § 5.15 unspecified defaults), and SHALL NOT hard-flag a difference confined to
the decoder-ignored `ci_reserved_2bit` or an alias-equivalent re-encoding (a preset
vs. its equivalent explicit triple or SAR, or a reserved id vs. an explicit
unspecified one).

#### Scenario: repeated content interpretation with differing timing

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` within one coded video sequence
- **WHEN** their `timing_info()`, `ci_scan_type_idc`, or chroma sample position
  differs
- **THEN** validation SHALL emit `content-interpretation/repeated-ci-not-identical`.

#### Scenario: repeated content interpretation with differing color or aspect

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` both carrying a color description (or aspect ratio)
- **WHEN** their derived color information (or derived sample aspect ratio) differs
  (e.g. BT.709 vs BT.2100 PQ, or SAR 1:1 vs 12:11)
- **THEN** validation SHALL emit `content-interpretation/repeated-ci-not-identical`.

#### Scenario: repeat differing only in reserved bits

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` whose parsed § 5.15 fields are identical except `ci_reserved_2bit`
- **WHEN** validation runs
- **THEN** validation SHALL NOT emit `content-interpretation/repeated-ci-not-identical`.

#### Scenario: repeat differing only in alias-equivalent color/aspect encoding

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)` whose color (or aspect) is encoded differently but derives to the
  same value (a preset idc vs. the equivalent explicit triple or SAR, or a reserved
  id vs. an explicit unspecified one)
- **WHEN** validation runs
- **THEN** validation SHALL NOT emit `content-interpretation/repeated-ci-not-identical`.

#### Scenario: repeat with present color/aspect after an unspecified default

- **GIVEN** two content-interpretation OBUs for the same `(obu_xlayer_id,
  obu_mlayer_id)`, one omitting the color description (or aspect ratio) so it derives
  to the § 5.15 unspecified default, and the other carrying a specific value (e.g.
  BT.709, or SAR 1:1)
- **WHEN** validation runs
- **THEN** validation SHALL emit `content-interpretation/repeated-ci-not-identical`.

### Requirement: Content-interpretation reserved bits

`splot-validate` SHALL surface a non-zero `ci_reserved_2bit` (AV2 v1.0.0 § 6.14).

#### Scenario: non-zero reserved bits

- **GIVEN** a content-interpretation OBU whose `ci_reserved_2bit` is not 0
- **WHEN** validation runs
- **THEN** validation SHALL emit `content-interpretation/reserved-bits-nonzero` as a
  warning (the value is ignored by a decoder, so it is not a hard error).

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

### Requirement: Quantizer Matrix duplicate-reset validation

`splot-validate` SHALL report an error when a quantizer matrix OBU with
`qm_bit_map == 0` is not the first quantizer matrix OBU between coded frames
(AV2 v1.0.0 § 6.12).

#### Scenario: duplicate reset

- **GIVEN** two quantizer matrix OBUs between coded-frame boundaries, both with
  `qm_bit_map == 0`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `qm/duplicate-reset-between-frames`.

#### Scenario: single reset is conformant

- **GIVEN** a single quantizer matrix OBU with `qm_bit_map == 0` between coded frames
- **WHEN** the stream is validated
- **THEN** the validator SHALL NOT emit `qm/duplicate-reset-between-frames`.

### Requirement: Quantizer Matrix duplicate-level validation

`splot-validate` SHALL report an error when the same quantizer matrix level is specified
twice between coded frames (AV2 v1.0.0 § 6.12).

#### Scenario: duplicate level

- **GIVEN** two quantizer matrix OBUs between coded-frame boundaries that both specify
  level `L`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `qm/duplicate-level-between-frames`.

#### Scenario: same level across a coded frame is allowed

- **GIVEN** two quantizer matrix OBUs that specify level `L` but are separated by a
  frame-bearing OBU
- **WHEN** the stream is validated
- **THEN** the validator SHALL NOT emit `qm/duplicate-level-between-frames`.

### Requirement: Quantizer Matrix HLS availability state

`splot-validate` SHALL record per-level quantizer-matrix availability for future
frame-reference validation.

#### Scenario: user-defined level parsed

- **WHEN** a quantizer matrix OBU specifies level `L`
- **THEN** the validator SHALL record level `L` with its defining layer identity, plane
  count, and data-present status.

### Requirement: Film grain update-flags validation

`splot-validate` SHALL report an error when `fgm_update_flags == 0` (AV2 v1.0.0 § 6.13).

#### Scenario: empty film-grain update

- **WHEN** a film grain OBU has `fgm_update_flags == 0`
- **THEN** the validator SHALL emit `film-grain/update-flags-zero`.

### Requirement: Film grain chroma-idc validation

`splot-validate` SHALL report an error when `fgm_chroma_idc > 3` (AV2 v1.0.0 § 6.13).

#### Scenario: out-of-range chroma idc

- **WHEN** a film grain OBU has `fgm_chroma_idc` greater than `3`
- **THEN** the validator SHALL emit `film-grain/chroma-idc-out-of-range`.

### Requirement: Film grain duplicate-slot validation

`splot-validate` SHALL report an error when the same film-grain slot is updated more
than once in the same coded frame unit, subject to the validator's coded-frame-unit
boundary model (AV2 v1.0.0 § 6.13).

#### Scenario: duplicate slot in one coded frame unit

- **GIVEN** two film grain OBUs in the same coded frame unit that both update slot `i`
- **WHEN** the stream is validated
- **THEN** the validator SHALL emit `film-grain/duplicate-slot-in-coded-frame-unit`.

### Requirement: Film grain HLS availability state

`splot-validate` SHALL record per-slot film-grain availability for future
frame-reference validation.

#### Scenario: slot updated

- **WHEN** a film grain OBU updates slot `i`
- **THEN** the validator SHALL record slot `i` with its defining layer identity and
  chroma format.

### Requirement: Deferred frame-reference validation

`splot-validate` SHALL NOT claim frame-reference validation for quantizer matrices or
film grain (`using_qmatrix` / `qm_*`, `apply_grain` / `fgm_id`) until the relevant
frame-header fields are parsed and proven.

#### Scenario: no frame-reference diagnostics this phase

- **WHEN** a stream contains quantizer-matrix or film-grain OBUs without a parsed frame
  header
- **THEN** the validator SHALL NOT emit any `qm/unavailable-*` or `film-grain/unavailable-*`
  diagnostics.

### Requirement: Padding OBU diagnostics

`splot-validate` SHALL emit `padding/*` diagnostics for the locally-decidable AV2 v1.0.0
§ 5.16 / § 6.15 violations of `padding_obu()`.

#### Scenario: all-zero padding payload

- **GIVEN** a non-empty `OBU_PADDING` payload whose bytes are all zero
- **WHEN** the validator runs
- **THEN** it SHALL emit a `padding/all-zero-payload` error.

#### Scenario: malformed padding trailing bits

- **GIVEN** an `OBU_PADDING` whose last non-zero byte is not a valid `trailing_bits()`
  pattern
- **WHEN** the validator runs
- **THEN** it SHALL emit a `padding/invalid-trailing-bits` error.

#### Scenario: empty padding accepted

- **GIVEN** an `OBU_PADDING` with `obuPayloadSize == 0`
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `padding/*` error.

### Requirement: Metadata OBU diagnostics

`splot-validate` SHALL emit `metadata/*` diagnostics for the locally-decidable AV2 v1.0.0
§ 6.16 violations of the metadata OBUs.

#### Scenario: short layer idc out of range

- **GIVEN** a `metadata_short_obu()` with `muh_layer_idc >= 3`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/short-layer-idc-out-of-range` error (§ 6.16.2).

#### Scenario: group reserved bits non-zero

- **GIVEN** a non-cancelled group unit with `muh_reserved_zero_2bits != 0`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-reserved-bits-nonzero` warning (§ 6.16.3 says
  the field is ignored by decoders, so a non-zero value is a producer anomaly).

#### Scenario: group xlayer map global bit set

- **GIVEN** a global group unit with `muh_layer_idc == LAYER_VALUES` whose
  `muh_xlayer_map` has bit 31 set
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-xlayer-map-global-bit-set` error (§ 6.16.3).

#### Scenario: group mlayer map below obu mlayer

- **GIVEN** a group unit with `muh_layer_idc == LAYER_VALUES` whose `muh_mlayer_map` sets
  a bit `m` less than `obu_mlayer_id`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/group-mlayer-map-below-obu-mlayer` error (§ 6.16.3).

#### Scenario: temporal point info in a group

- **GIVEN** a `metadata_group_obu()` unit whose `metadata_type ==
  METADATA_TYPE_TEMPORAL_POINT_INFO`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/temporal-point-info-not-short` error (§ 6.16.11).

#### Scenario: timecode fields out of range

- **GIVEN** a `metadata_timecode()` with `seconds_value > 59`, `minutes_value > 59`, or
  `hours_value > 23` (when present)
- **WHEN** the validator runs
- **THEN** it SHALL emit the corresponding `metadata/timecode-seconds-out-of-range`,
  `metadata/timecode-minutes-out-of-range`, or `metadata/timecode-hours-out-of-range`
  error (§ 6.16.7).

#### Scenario: scan-type reserved pic struct

- **GIVEN** a `metadata_scan_type()` with `mps_pic_struct_type > 12`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/scan-type-pic-struct-reserved` error (§ 6.16.10).

### Requirement: Metadata temporal-unit ordering classification

`splot-validate` SHALL classify a metadata OBU for temporal-unit ordering (AV2 v1.0.0
§ 7.3.7) from its parsed `metadata_is_suffix` bit: global prefix metadata is a global
temporal-unit prefix OBU, global suffix metadata is not, and non-global metadata is a
coded extended layer OBU.

#### Scenario: global prefix metadata after a coded layer

- **GIVEN** a global metadata OBU with `metadata_is_suffix == 0` that follows a coded
  extended layer unit within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL emit an `obu-order/global-hls-after-coded-layer` error.

#### Scenario: global suffix metadata after a coded layer

- **GIVEN** a global metadata OBU with `metadata_is_suffix == 1` that follows a coded
  extended layer unit within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit an `obu-order/global-hls-after-coded-layer` error for it.

#### Scenario: non-global metadata uses coded xlayer order

- **GIVEN** a non-global metadata OBU within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL treat it as a coded extended layer OBU for ascending
  `obu_xlayer_id` ordering.

### Requirement: Metadata persistence and cancellation lifetime

`splot-validate` SHALL track active metadata per `(obu_xlayer_id, metadata_type)` within
a coded video sequence and apply the AV2 v1.0.0 § 6.16.3 `muh_persistence_idc` and
`muh_cancel_flag` semantics, including cross-layer propagation via the sequence header's
layer-dependency maps. The propagation queries model decoder applicability and SHALL NOT
emit diagnostics by themselves.

#### Scenario: cancel clears active metadata for the layer

- **GIVEN** a metadata unit of a type with BASIC persistence active for an extended layer
- **AND** a later metadata unit of the same type with `muh_cancel_flag == 1` for that layer
- **WHEN** the validator runs
- **THEN** the metadata SHALL no longer be considered active for that layer.

#### Scenario: global persistence ignores cancel

- **GIVEN** a metadata unit with `muh_persistence_idc == GLOBAL_PERSISTENCE`
- **WHEN** a later `muh_cancel_flag == 1` of the same type is observed
- **THEN** the global metadata SHALL remain active (the cancel is a no-op, § 6.16.3).

#### Scenario: repeated HDR metadata with different content in a CVS

- **GIVEN** two HDR CLL (or HDR MDCV) metadata units in the same coded video sequence
  whose § 6.16.3 layer targeting associates both with at least one common embedded
  layer — regardless of how each unit encodes that targeting (`LAYER_GLOBAL`,
  `LAYER_CURRENT`, or explicit `LAYER_VALUES` maps) — and whose contents differ
- **WHEN** the validator runs
- **THEN** it SHALL emit `metadata/hdr-cll-repeat-content-differs` (or
  `metadata/hdr-mdcv-repeat-content-differs`) (§ 6.16.5 / § 6.16.6). Units whose
  association is not derivable from the bitstream (`LAYER_UNSPECIFIED`,
  `LAYER_CURRENT` on a `GLOBAL_XLAYER_ID` OBU, reserved `muh_layer_idc`) SHALL NOT
  be compared.

### Requirement: Scan-type CVS-wide consistency

`splot-validate` SHALL enforce the AV2 v1.0.0 § 6.16.10 cross-OBU scan-type constraints:
the Table 6.18 restrictions tying each defined `mps_pic_struct_type` value to a required
content-interpretation `ci_scan_type_idc` (and to `equal_picture_interval == 1` for the
frame-doubling/tripling values 7/8), and the requirement that `mps_pic_struct_type`
stays within a single permitted group for all pictures of the CVS. (The spec mirror
defines no `mps_source_scan_type_idc` ↔ `ci_scan_type_idc` consistency rule —
`mps_source_scan_type_idc` only shares its value semantics with `ci_scan_type_idc`.)

#### Scenario: pic-struct group changes within a CVS

- **GIVEN** two scan-type metadata units in the same CVS whose `mps_pic_struct_type`
  values fall into different Table 6.18 groups
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/scan-type-pic-struct-group-inconsistent` error.

#### Scenario: pic-struct contradicts the established scan type

- **GIVEN** a defined `mps_pic_struct_type` whose Table 6.18 required `ci_scan_type_idc`
  differs from a non-zero `ci_scan_type_idc` established by a content-interpretation OBU
  in the same CVS scope, where both sides belong to the same § 7.3.8.11
  content-interpretation-parameter epoch (the CI parameters re-initialize to defaults at
  each temporal unit containing a CLK or OLK for the extended layer, so a pre-epoch CI no
  longer establishes the parameters a post-epoch picture sees, and vice versa)
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/scan-type-ci-scan-type-mismatch` error.

### Requirement: Decoded-frame-hash verification

`splot-validate` SHALL verify `metadata_decoded_frame_hash` (§ 6.16.13) against the
decoded output samples once a decoder is available.

#### Scenario: hash mismatch

- **GIVEN** a decoded frame and a `metadata_decoded_frame_hash` whose recomputed MD5 over
  the output samples differs from the signaled value
- **WHEN** the validator runs (with a decoder)
- **THEN** it SHALL emit a hash-mismatch error.

### Requirement: Metadata placement inside coded frame units

`splot-validate` SHALL validate that prefix metadata (`metadata_is_suffix == 0`) appears
before the frame data and suffix metadata (`metadata_is_suffix == 1`) after it within a
coded frame unit (AV2 v1.0.0 § 7.3.3 / § 7.3.4), once frame-header and tile-group parsing
locate the frame-data boundary.

#### Scenario: suffix metadata before frame data

- **GIVEN** a coded frame unit whose suffix metadata appears before the frame data
- **WHEN** the validator runs (with frame/tile parsing)
- **THEN** it SHALL emit a placement error.

### Requirement: Preserve existing HLS frame-reference diagnostics

The validator SHALL preserve existing sequence-header and multi-frame-header availability checks for frame headers.

#### Scenario: Frame references missing sequence header

- **GIVEN** a frame header directly references a sequence header id that is not available in-band
- **AND** external HLS is disabled
- **WHEN** validation runs
- **THEN** the validator SHALL emit `hls/unavailable-sequence-header`.

#### Scenario: Frame references missing MFH

- **GIVEN** a frame header references a `cur_mfh_id` without an available MFH
- **AND** external HLS is disabled
- **WHEN** validation runs
- **THEN** the validator SHALL emit `hls/unavailable-multi-frame-header`.

### Requirement: Frame-header core diagnostics

The validator SHALL emit structured diagnostics for state-supported local frame-header violations.

#### Scenario: Bridge reference index out of range

- **GIVEN** `bridge_frame_ref_idx` is parsed
- **AND** the validator knows `NumRefFrames`
- **WHEN** `bridge_frame_ref_idx >= NumRefFrames`
- **THEN** the validator SHALL emit `frame-header/bridge-ref-index-out-of-range`.

#### Scenario: Frame size exceeds sequence maximum

- **GIVEN** a parsed frame size
- **AND** an active sequence maximum
- **WHEN** the frame width or height exceeds the active sequence maximum
- **THEN** the validator SHALL emit `frame-header/frame-size-exceeds-sequence-max`.

#### Scenario: RAS requires long-term frame id bits

- **GIVEN** the active sequence has `long_term_frame_id_bits == 0`
- **WHEN** a frame OBU has `obu_type == OBU_RAS_FRAME`
- **THEN** the validator SHALL emit `frame-header/ras-requires-long-term-frame-id-bits`.

### Requirement: No false positives for unavailable state

The validator SHALL NOT emit normative errors for checks that require reference-frame state it does not model.

#### Scenario: Reference state is absent

- **GIVEN** the core parser reaches a reference-state-dependent branch
- **AND** the validator does not yet model the required reference state
- **WHEN** validation runs
- **THEN** the validator SHALL record partial/unsupported status or an informational diagnostic
- **AND** it SHALL NOT emit a conformance error based on guessed state.

### Requirement: Inspector status

Inspector output SHALL expose frame-header parse status for frame-bearing OBUs.

#### Scenario: Core frame header is partially parsed

- **GIVEN** a frame-bearing OBU
- **WHEN** `inspect --json` is run
- **THEN** the JSON SHALL include a frame-header status such as `activation_fields_only`, `core_fields_only`, or `stopped_before_filtering_quant_segmentation`.

### Requirement: Active operating point set state

`splot-validate` SHALL maintain active in-band operating point set records keyed by
`(obu_xlayer_id, ops_id)` with the non-monotonic reset/update semantics of AV2 v1.0.0
§ 6.10.1, distinct from the monotonic HLS availability store.

#### Scenario: reset clears active OPS

- **GIVEN** an OPS that defines `(xlayer, ops_id)`, followed by an OPS OBU with
  `ops_reset_flag == 1` and `ops_cnt == 0` for that layer
- **WHEN** the validator runs
- **THEN** the previously defined OPS SHALL no longer be available
- **AND** a later buffer-removal-timing reference to it SHALL be unavailable.

#### Scenario: update changes the active count

- **GIVEN** an OPS defined with `ops_cnt == 2` that is then redefined with
  `ops_cnt == 3`
- **WHEN** a buffer-removal-timing OBU references it with `br_ops_cnt == 2`
- **THEN** the validator SHALL compare against the updated `ops_cnt == 3`.

### Requirement: Locally-decidable OPS semantics

`splot-validate` SHALL emit `ops/*` diagnostics for the locally-decidable § 6.10
conformance violations.

#### Scenario: local reserved bits

- **GIVEN** a local OPS with a non-zero `ops_reserved_2bits`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/local-reserved-bits-nonzero` error.

#### Scenario: reserved mlayer-info idc

- **GIVEN** a global OPS with `ops_mlayer_info_idc == 3`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/mlayer-info-idc-reserved` error.

#### Scenario: payload size mismatch

- **GIVEN** an operating point payload whose computed `opsBytes` differs from its
  declared `ops_data_size`
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/payload-size-mismatch` error.

#### Scenario: inherited op-index out of range

- **GIVEN** an inherited operating-point reference whose `ops_embedded_op_index` is out
  of range for the referenced operating point set
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/inherited-op-index-out-of-range` error.

### Requirement: Buffer removal timing references

`splot-validate` SHALL validate OPS-dependent buffer-removal-timing references against
active OPS state, gating hard errors on external HLS being disabled.

#### Scenario: unavailable operating point set

- **GIVEN** an OPS-dependent BRT whose `br_ops_id` resolves to no active OPS
- **WHEN** the validator runs with external HLS disabled
- **THEN** it SHALL emit a `brt/unavailable-operating-point-set` error.

#### Scenario: count mismatch

- **GIVEN** an OPS-dependent BRT whose `br_ops_cnt` differs from the active OPS
  `ops_cnt`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `brt/ops-count-mismatch` error.

#### Scenario: declared external OPS suppresses the hard missing-OPS error

- **GIVEN** an OPS-dependent BRT whose `br_ops_id` resolves to no in-band OPS
- **AND** the caller declares that `(obu_xlayer_id, br_ops_id)` is available as external
  HLS
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit a hard `brt/unavailable-operating-point-set` error.

#### Scenario: external HLS that does not declare the OPS still flags it

- **GIVEN** an OPS-dependent BRT whose `br_ops_id` resolves to no in-band OPS
- **AND** external HLS is provided but does not declare that `(obu_xlayer_id, br_ops_id)`
- **WHEN** the validator runs
- **THEN** it SHALL emit a `brt/unavailable-operating-point-set` error.

### Requirement: Buffer removal timing ordering classification

`splot-validate` SHALL classify `OBU_BUFFER_REMOVAL_TIMING` for temporal-unit ordering
per AV2 § 7.3.3 / § 7.3.4 / § 7.3.7: a local BRT is a coded-extended-layer OBU, and a
global BRT is not a global temporal-unit prefix OBU.

#### Scenario: local BRT starts the coded-layer phase

- **GIVEN** a local BRT followed by a global OPS within a temporal unit
- **WHEN** the validator runs
- **THEN** it SHALL flag the global OPS with `obu-order/global-hls-after-coded-layer`,
  because the local BRT started the coded-layer phase.

#### Scenario: global BRT is not flagged for ordering

- **GIVEN** a global BRT before or after a coded extended layer unit
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit an `obu-order/global-hls-after-coded-layer` error for the
  BRT.

### Requirement: Frame tile-info conformance diagnostics
The validator SHALL emit structured error diagnostics, with stable `rule_id`,
`severity`, `spec_section`, and byte offsets, for the locally-decidable
§ 6.17.7.2 tile-info constraints on parsed frame headers: `TileCols` greater
than `MAX_TILE_COLS`, `TileRows` greater than `MAX_TILE_ROWS`, and
`context_update_tile_id` not less than `TileCols * TileRows`. Each new rule id
SHALL be registered in `docs/VALIDATOR-DIAGNOSTICS.md`.

#### Scenario: Out-of-range context update tile id
- **WHEN** a frame header parses a `context_update_tile_id` greater than or
  equal to `TileCols * TileRows`
- **THEN** the validator reports an error diagnostic citing § 6.17.7.2 at the
  frame-header OBU offset

#### Scenario: Conforming tile layout is silent
- **WHEN** a frame header parses a tile layout within the MAX_TILE_COLS and
  MAX_TILE_ROWS bounds and a valid `context_update_tile_id`
- **THEN** no tile-info diagnostics are emitted

### Requirement: Frame QM reference diagnostics
For parsed `setup_qm_params()` levels that reference custom quantizer matrices
(`qm_y`/`qm_u`/`qm_v` less than `NUM_CUSTOM_QMS`), the validator SHALL check
the locally-decidable § 6.17.6.2 constraints against its existing quantizer
matrix availability state: the referenced custom QM slot's `QmNumPlanes` SHALL
equal the active sequence's `NumPlanes`, and layer-dependency constraints SHALL
only be checked when the required dependency maps are available, never guessed.
Violations SHALL be error diagnostics citing § 6.17.6.2; unavailable state
SHALL NOT produce false positives.

#### Scenario: Custom QM plane-count mismatch
- **WHEN** a frame header references a custom QM whose recorded plane count
  differs from the active sequence's `NumPlanes`
- **THEN** the validator reports an error diagnostic citing § 6.17.6.2

#### Scenario: Missing QM state stays silent
- **WHEN** a frame header references a custom QM slot for which no quantizer
  matrix OBU state is available
- **THEN** the validator does not emit a § 6.17.6.2 plane-count diagnostic for
  that reference (the existing QM availability diagnostics own that case)

### Requirement: Frame-header parse coverage reporting stays honest
The validator and inspector SHALL report the new stopped-before-deblocking
parse status distinctly, and SHALL NOT claim full § 5.18 frame-header
conformance for frame headers parsed only through the new stop point. Existing
frame-header activation and HLS reference diagnostics SHALL be preserved
unchanged.

#### Scenario: Inspector surfaces new fields and status
- **WHEN** `splot inspect` runs on a stream whose frame header parses through
  quantization/segmentation/tiling
- **THEN** the JSON frame-header summary includes the parsed quantizer, QM,
  segmentation, and tile-layout fields plus the new stop-point status label

#### Scenario: Existing diagnostics regression-safe
- **WHEN** the existing validator test suite runs after this change
- **THEN** all previously emitted diagnostics (rule ids, severities, spec
  sections) are unchanged
