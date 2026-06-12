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
`AV2-7.3-OBU-ORDERING`, `AV2-7.3.8-HLS-AVAILABILITY`, and
`AV2-IVF-CONTAINER`.
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

### Requirement: Frame CCSO-params conformance diagnostics
The validator SHALL emit structured error diagnostics, with stable `rule_id`,
`severity`, `spec_section`, and byte offsets, for the locally-decidable
§ 6.17.7.8 CCSO-params constraints on a parsed frame `ccso_params()`:
`ccso_ext_filter` equal to `7` (the reserved value), and
`1 << ccso_max_band_log2` greater than `CCSO_BAND_NUM`. The reference-state
CCSO requirements (`ccso_ref_idx < NumTotalRefs` and the reuse equalities)
are dead on the intra path and SHALL NOT be guessed. Each new rule id SHALL be
registered in `docs/VALIDATOR-DIAGNOSTICS.md`.

#### Scenario: Reserved CCSO ext filter
- **WHEN** a frame header parses a `ccso_params()` plane with
  `ccso_ext_filter == 7`
- **THEN** the validator reports an error diagnostic citing § 6.17.7.8 at the
  frame-header OBU offset

#### Scenario: Conforming CCSO params are silent
- **WHEN** a frame header parses `ccso_params()` with `ccso_ext_filter != 7`
  and `1 << ccso_max_band_log2 <= CCSO_BAND_NUM`
- **THEN** no CCSO-params diagnostics are emitted

### Requirement: Frame QM reference diagnostics
The validator SHALL check the locally-decidable § 6.17.6.2 constraints for
parsed `setup_qm_params()` levels that reference custom quantizer matrices
(`qm_y`/`qm_u`/`qm_v` less than `NUM_CUSTOM_QMS`) against its existing
quantizer matrix availability state: the referenced custom QM slot's
`QmNumPlanes` SHALL equal the active sequence's `NumPlanes`, and
layer-dependency constraints SHALL only be checked when the required
dependency maps are available, never guessed. Violations SHALL be error
diagnostics citing § 6.17.6.2; unavailable state SHALL NOT produce false
positives.

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
The validator and inspector SHALL report each frame-header parse status
distinctly — the complete intra-path terminal (after the § 5.18.2 tail), the
complete show-existing-frame terminal (after its `film_grain_config()`), the honest
stop before the unmodeled `read_wienerns_filter()` frame-level Wiener bank decode,
the truncation status for a payload that ends inside the loop-filter /
loop-restoration / CCSO cluster, and the truncation status for a payload that ends
inside the § 5.18.2 tail — and SHALL NOT claim full § 5.18 frame-header conformance
(including trailing-bits conformance of the carrying OBU) from any of these statuses,
since the frame header is followed by the rest of `tile_group_obu()` (§ 5.19). A
frame header truncated inside the cluster or tail SHALL still expose its
already-parsed control-region facts to the state-supported diagnostics (the
truncation SHALL NOT silence earlier frame-size / output-class checks). Existing
frame-header activation and HLS reference diagnostics SHALL be preserved unchanged.

#### Scenario: Inspector surfaces new fields and status
- **WHEN** `splot inspect` runs on a stream whose frame header parses through
  quantization/segmentation/tiling
- **THEN** the JSON frame-header summary includes the parsed quantizer, QM,
  segmentation, and tile-layout fields plus the new stop-point status label

#### Scenario: Existing diagnostics regression-safe
- **WHEN** the existing validator test suite runs after this change
- **THEN** all previously emitted diagnostics (rule ids, severities, spec
  sections) are unchanged

### Requirement: OPS dependency-map agreement

`splot-validate` SHALL check explicitly signalled `ops_mlayer_map` /
`ops_tlayer_map` entries for dependency closure under the activated sequence
header's `MLayerDependencyMap` / `TLayerDependencyMap` (AV2 v1.0.0 § 6.10.7):
for any embedded layer `cMId` included by `ops_mlayer_map` whose
`MLayerDependencyMap[cMId][rMId]` is 1, bit `rMId` SHALL also be included for
all non-negative `rMId < cMId`, and for any temporal layer `cTId` included by
`ops_tlayer_map[..][cMId]` whose `TLayerDependencyMap[cMId][cTId][rTId]` is 1,
bit `rTId` SHALL also be included for all non-negative `rTId < cTId`. Each
per-extended-layer entry is checked against the sequence header activated for
that entry's extended layer, both when the OPS OBU is observed and when a
later activation makes the pairing decidable, without duplicate diagnostics
for the same `(OPS instance, entry, sequence header)` pairing. An activation
is decidable only when confirmed by a parsed frame-header reference or while
the OBU-order fallback is the sole in-band sequence header — with several
in-band candidates and no frame, the check SHALL defer to the frame-driven
activation rather than guess. Inherited and absent mlayer info SHALL NOT be
checked (§ 6.10.7 binds the maps "if present").

#### Scenario: OPS mlayer map missing a required dependency

- **GIVEN** an activated sequence header whose `MLayerDependencyMap[1][0]` is 1
- **AND** an OPS entry for that extended layer whose `ops_mlayer_map` includes
  embedded layer 1 but not embedded layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/mlayer-dependency-missing` error (§ 6.10.7).

#### Scenario: OPS tlayer map missing a required dependency

- **GIVEN** an activated sequence header whose
  `TLayerDependencyMap[0][1][0]` is 1
- **AND** an OPS entry for that extended layer whose `ops_tlayer_map` for
  embedded layer 0 includes temporal layer 1 but not temporal layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `ops/tlayer-dependency-missing` error (§ 6.10.7).

#### Scenario: dependency-closed OPS maps are silent

- **GIVEN** an activated sequence header and an OPS whose explicit
  `ops_mlayer_map` / `ops_tlayer_map` entries are dependency-closed under its
  maps
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `ops/*-dependency-missing` diagnostic.

#### Scenario: OPS before the first activation is still checked

- **GIVEN** a temporal unit carrying a sequence header, then a global OPS,
  then a frame header that activates that sequence header for the entry's
  extended layer
- **AND** the OPS maps disagree with the activated header's maps
- **WHEN** the validator runs
- **THEN** it SHALL emit the corresponding `ops/*-dependency-missing` error
  exactly once for that pairing.

#### Scenario: no activated sequence header means no OPS agreement check

- **GIVEN** an OPS entry for an extended layer with no in-band activated
  sequence header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `ops/*-dependency-missing` diagnostic for
  that entry (the maps are never fabricated from defaults).

#### Scenario: external sequence headers suppress the OPS agreement check

- **GIVEN** validation runs with `ExternalHlsMode::Provided` declaring at
  least one sequence header
- **WHEN** an OPS entry's maps disagree with the in-band activated header
- **THEN** the validator SHALL NOT emit an `ops/*-dependency-missing` error
  (an externally activated header with unmodeled maps may govern).

#### Scenario: a same-id sequence-header redefinition re-binds the checks

- **GIVEN** an OPS finding emitted against sequence header id `N`
- **AND** a later sequence header reusing id `N` whose agreement inputs
  (dependency maps or `seq_lcr_id`) changed while the stored OPS maps still
  disagree
- **WHEN** the validator runs
- **THEN** it SHALL re-emit the finding against the redefined content (the
  id's dedup keys are invalidated by the redefinition).

#### Scenario: an ambiguous fallback activation defers to the frame

- **GIVEN** two in-band sequence headers available before any frame, where the
  OPS maps disagree with the first-seen (fallback) header but agree with the
  second
- **AND** a frame header that loads the second header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `ops/*-dependency-missing` diagnostic (the
  pairing binds the frame-confirmed header, not the fallback guess).

### Requirement: LCR dependency-map agreement

`splot-validate` SHALL check the activated LCR's
`lcr_mlayer_map[isGlobal][xId]` / `lcr_tlayer_map[isGlobal][xId][cMId]` for
the same dependency closure under the activated sequence header's
`MLayerDependencyMap` / `TLayerDependencyMap` (AV2 v1.0.0 § 6.8.9, all four
`isGlobal` × map bullets). The pairing binds the header's § 6.4.1
*association*, snapshotted at each observation of that sequence header: the
`seq_lcr_id` resolution (local-first-then-global, § 6.4.1) against the LCRs
present prior to that observation, with the resolved record's embedded-layer
maps as of that observation. The check runs at decidable activation events
(frame-confirmed, or the sole-in-band-header fallback) for the `xId == x`
entry, once per `(xlayer, sequence header, defining LCR OBU)` pairing. An LCR
arriving after every observation of the activating sequence header SHALL NOT
be paired (§ 6.4.1 associates only an LCR "present prior to this sequence
header"), a re-observed header SHALL re-take its association snapshot (an LCR
that arrived between two sightings pairs with the later one), a record
redefined after the header's latest observation SHALL NOT replace the
snapshot, and the check SHALL be suppressed whenever external HLS is enabled
(an unmodeled external local LCR would win the § 6.4.1 resolution). The
diagnostics SHALL carry the associated LCR OBU's byte offset.

#### Scenario: activated local LCR mlayer map missing a required dependency

- **GIVEN** an activated sequence header for xlayer `x` with
  `seq_lcr_id != 0` resolving to an in-band local LCR in xlayer `x`
- **AND** `MLayerDependencyMap[1][0]` is 1 while the LCR's
  `lcr_mlayer_map[0][x]` includes embedded layer 1 but not embedded layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `lcr/mlayer-dependency-missing` error (§ 6.8.9)
  at the LCR OBU's offset.

#### Scenario: activated global LCR tlayer map missing a required dependency

- **GIVEN** an activated sequence header for xlayer `x` whose `seq_lcr_id`
  resolves to an in-band global LCR whose `lcr_xlayer_map` includes `x`
- **AND** `TLayerDependencyMap[0][1][0]` is 1 while the LCR's
  `lcr_tlayer_map[1][x][0]` includes temporal layer 1 but not temporal layer 0
- **WHEN** the validator runs
- **THEN** it SHALL emit an `lcr/tlayer-dependency-missing` error (§ 6.8.9).

#### Scenario: dependency-closed activated LCR is silent

- **GIVEN** an activated sequence header whose `seq_lcr_id` resolves to an
  in-band LCR whose maps for that xlayer are dependency-closed under the
  header's maps
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic.

#### Scenario: unactivated or unresolved LCR pairings are not checked

- **GIVEN** an LCR that no activated sequence header resolves via
  `seq_lcr_id`, or a sequence header with `seq_lcr_id == 0`
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic, and
  SHALL NOT emit duplicates when the same pairing re-activates across frames.

#### Scenario: a later LCR is not retroactively paired

- **GIVEN** a sequence header with `seq_lcr_id != 0` followed (not preceded)
  by an LCR with that id whose maps disagree
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic (the
  § 7.3.8.3 availability diagnostic owns the stream).

#### Scenario: a repeated sequence header pairs with a now-present LCR

- **GIVEN** a sequence header with `seq_lcr_id != 0`, then a disagreeing LCR
  with that id, then a bit-identical repeat of the sequence header
- **WHEN** the validator runs
- **THEN** it SHALL emit the corresponding `lcr/*-dependency-missing` error
  exactly once (the LCR is present prior to the repeat, § 6.4.1).

#### Scenario: a post-header redefinition is not the association

- **GIVEN** a dependency-closed LCR, then a sequence header whose
  `seq_lcr_id` resolves to it, then a disagreeing redefinition of that LCR,
  then a frame loading the header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic (the
  header's association snapshot is the dependency-closed record).

#### Scenario: provided external HLS suppresses the LCR agreement check

- **GIVEN** validation runs with `ExternalHlsMode::Provided` (even an empty
  set)
- **WHEN** an in-band resolved LCR's maps disagree with the activated header
- **THEN** the validator SHALL NOT emit any `lcr/*-dependency-missing`
  diagnostic.

#### Scenario: a redefinition replaces the checked maps

- **GIVEN** a violating LCR followed by a redefinition of the same id without
  embedded-layer info, then the activating sequence header
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit any `lcr/*-dependency-missing` diagnostic (the
  latest definition has nothing to check).

### Requirement: Frame-header MFH layer-dependency checks
`splot-validate` SHALL enforce the § 7.3.8.7 layer-dependency constraints for
a parsed frame-header prefix with `cur_mfh_id > 0` whose multi-frame header
and the MFH's `mfh_seq_header_id` both resolve in-band, using the § 6.17.2
predicate evaluated after the sequence header is loaded:
`MLayerDependencyMap[obu_mlayer_id][MfhMLayerId[cur_mfh_id]]` SHALL be 1 and
`TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][MfhTLayerId[cur_mfh_id]]`
SHALL be 1, where `obu_mlayer_id` / `obu_tlayer_id` are the frame header's and
`MfhMLayerId` / `MfhTLayerId` are the recorded multi-frame header's. This
resolves the deferred `TODO(spec: AV2-5.7-MULTI-FRAME-HEADER)` check.

#### Scenario: frame does not depend on the MFH's embedded layer

- **GIVEN** a frame header with `cur_mfh_id > 0` resolving to an MFH recorded
  with `MfhMLayerId` equal to `m`
- **AND** the loaded sequence header's
  `MLayerDependencyMap[obu_mlayer_id][m]` is 0
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit a `frame-header/mfh-mlayer-dependency-missing` error
  (§ 7.3.8.7) at the frame-header OBU's offset.

#### Scenario: frame does not depend on the MFH's temporal layer

- **GIVEN** a frame header with `cur_mfh_id > 0` resolving to an MFH recorded
  with `MfhTLayerId` equal to `t`
- **AND** the loaded sequence header's
  `TLayerDependencyMap[obu_mlayer_id][obu_tlayer_id][t]` is 0
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL emit a `frame-header/mfh-tlayer-dependency-missing` error
  (§ 7.3.8.7).

#### Scenario: satisfied MFH layer dependencies are silent

- **GIVEN** a frame header whose `cur_mfh_id` resolves to an MFH whose
  recorded layer ids satisfy both § 6.17.2 dependency-map predicates under the
  loaded sequence header
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit any `frame-header/mfh-*-dependency-missing`
  diagnostic.

#### Scenario: unresolved MFH or sequence header is not layer-checked

- **GIVEN** a frame header with `cur_mfh_id > 0` whose MFH is unavailable, or
  whose MFH's sequence header resolves only externally or not at all
- **WHEN** the validator observes the frame-header prefix
- **THEN** it SHALL NOT emit any `frame-header/mfh-*-dependency-missing`
  diagnostic (the existing availability diagnostics own those cases).

### Requirement: IVF validation input

`splot-validate` SHALL validate both raw Annex B inputs and IVF-wrapped Annex B
inputs through the default byte-validation API.

#### Scenario: Valid IVF input validates like its payload

- **WHEN** `Validator::validate_bytes` receives an IVF file whose frames contain
  conformant Annex B OBUs
- **THEN** validation SHALL report no errors caused by the container
- **AND** SHALL run the existing OBU checks over the frame payload OBUs.

#### Scenario: Malformed IVF input is a report

- **WHEN** `Validator::validate_bytes` receives a malformed IVF file
- **THEN** validation SHALL emit a stable `ivf/*` diagnostic
- **AND** SHALL return a `ValidationReport` rather than panicking or returning a
  CLI-only error.

### Requirement: IVF diagnostic namespace

IVF diagnostics SHALL use the `ivf/` namespace, include severity, byte offset when
known, and a human-readable message.

#### Scenario: Truncated frame payload diagnostic

- **WHEN** an IVF frame declares more payload bytes than remain in the input
- **THEN** validation SHALL emit `ivf/truncated-frame-payload`
- **AND** the diagnostic SHALL point at the first missing byte offset.

### Requirement: Distinct embedded-layer count per coded video sequence

`splot-validate` SHALL count the distinct `obu_mlayer_id` values observed in each
extended layer's coded video sequence (AV2 v1.0.0 § 6.4.1: counting applies to all
OBUs, even non-layer-specific ones) and SHALL emit
`sequence-state/distinct-mlayer-count-exceeds-seq-max` (severity `error`) when the
count exceeds the active sequence header's `SeqMaxMlayerCnt`. Counting SHALL reset at
each § 7.3.6 CVS start (CLK) for that extended layer; because the new coded video
sequence starts AT the temporal unit containing the CLK, the same-temporal-unit ids
observed before the CLK (canonically the § 7.3.8.1 resent-at-RAP sequence header,
forced to `obu_mlayer_id == 0`) SHALL be re-attributed to the new coded video sequence
and counted toward its `SeqMaxMlayerCnt`. Ids from temporal units before the boundary
temporal unit SHALL NOT count into the new coded video sequence, and OBUs whose
attribution to the CVS is ambiguous under the documented reading (global
`obu_xlayer_id`) SHALL NOT be counted (sound under-approximation).

#### Scenario: distinct mlayer ids exceed SeqMaxMlayerCnt

- **GIVEN** an active sequence header with `seq_max_mlayer_cnt_minus_1 == 0`
  (`SeqMaxMlayerCnt == 1`)
- **WHEN** OBUs of the same coded video sequence carry two distinct `obu_mlayer_id`
  values
- **THEN** validation SHALL emit `sequence-state/distinct-mlayer-count-exceeds-seq-max`
  with spec section § 6.4.1.

#### Scenario: count resets at a CVS boundary

- **GIVEN** a coded video sequence using `SeqMaxMlayerCnt` distinct `obu_mlayer_id`
  values
- **WHEN** a CLK starts a new CVS for the extended layer and the new CVS uses a
  disjoint but equally sized set of `obu_mlayer_id` values
- **THEN** validation SHALL NOT emit
  `sequence-state/distinct-mlayer-count-exceeds-seq-max`.

#### Scenario: pre-CLK header is re-attributed to the new CVS

- **GIVEN** an active sequence header with `SeqMaxMlayerCnt == 1`
- **WHEN** a single temporal unit carries a resent sequence header at
  `obu_mlayer_id == 0` followed by a CLK at `obu_mlayer_id == 1` (the CLK begins a new
  coded video sequence at that temporal unit)
- **THEN** validation SHALL count `{0, 1}` toward the new CVS and emit
  `sequence-state/distinct-mlayer-count-exceeds-seq-max` exactly once with spec section
  § 6.4.1.

### Requirement: SWITCH and RAS frame dependency-map self-containment

`splot-validate` SHALL emit
`frame-header/switch-or-ras-mlayer-dependency-not-self-contained` (severity `error`)
when a frame-bearing OBU with `obu_type` equal to `OBU_SWITCH` or `OBU_RAS_FRAME` has,
for any embedded layer ID `m` not equal to its `obu_mlayer_id`,
`MLayerDependencyMap[obu_mlayer_id][m] != 0` in the active sequence header
(AV2 v1.0.0 § 6.4.1).

#### Scenario: switch frame depends on another embedded layer

- **GIVEN** an active sequence header whose `MLayerDependencyMap` marks embedded layer
  1 as depending on embedded layer 0
- **WHEN** an `OBU_SWITCH` with `obu_mlayer_id == 1` is validated
- **THEN** validation SHALL emit
  `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` with spec section
  § 6.4.1.

#### Scenario: self-contained RAS frame passes

- **GIVEN** an active sequence header whose `MLayerDependencyMap` row for embedded
  layer 1 references only embedded layer 1
- **WHEN** an `OBU_RAS_FRAME` with `obu_mlayer_id == 1` is validated
- **THEN** validation SHALL NOT emit
  `frame-header/switch-or-ras-mlayer-dependency-not-self-contained`.

### Requirement: Single active sequence header per extended layer per CVS

`splot-validate` SHALL emit `hls/multiple-active-sequence-headers` (severity `error`)
when, within one extended layer's coded video sequence, a frame-confirmed sequence
activation is followed by a non-CLK activation of a different `seq_header_id` with no
intervening CVS start (AV2 v1.0.0 § 7.3.6: within each extended layer, only one
sequence header remains active for the duration of a coded video sequence). The check
SHALL NOT fire when the prior activation was only an OBU-order fallback guess, and
SHALL be suppressed only when caller-provided external HLS declares at least one
sequence header (an external channel that declares no sequence header cannot supply an
out-of-band active header, so it SHALL NOT suppress).

#### Scenario: second activation without a CLK

- **GIVEN** a frame-confirmed activation of `seq_header_id == 0` for an extended layer
- **WHEN** a later non-CLK frame header in the same CVS activates `seq_header_id == 1`
  for that extended layer
- **THEN** validation SHALL emit `hls/multiple-active-sequence-headers` with spec
  section § 7.3.6.

#### Scenario: re-activation across a CLK is conforming

- **GIVEN** a frame-confirmed activation of `seq_header_id == 0` for an extended layer
- **WHEN** a CLK starts a new CVS for that extended layer and its frame header
  activates `seq_header_id == 1`
- **THEN** validation SHALL NOT emit `hls/multiple-active-sequence-headers`.

#### Scenario: unreferenced extra sequence header is conforming

- **GIVEN** an active sequence header for an extended layer
- **WHEN** a sequence-header OBU with a different `seq_header_id` appears in the
  bitstream without being referenced by any frame header
- **THEN** validation SHALL NOT emit `hls/multiple-active-sequence-headers`
  (§ 7.3.6 permits unactivated additional sequence headers).

### Requirement: Monotonic output order agreement across a CMVS

`splot-validate` SHALL track § 7.3.2 coded-multistream-video-sequence boundaries with a
three-state tracker (`Outside` / `Inside` / `Unknown`) and SHALL emit
`sequence-state/monotonic-output-order-mismatch` (severity `error`) when, definitively
inside a CMVS, extended layers are associated with active sequence headers that
disagree on `monotonic_output_order_flag` (AV2 v1.0.0 § 6.4.1). The check SHALL NOT
fire in the `Outside` or `Unknown` tracker states. When a disagreement is observed at a
sequence-header OBU before any CLK in the temporal unit, the `Inside` membership is only
provisional (a later MSDO-less CLK could end the CMVS, § 7.3.2 end condition 2), so the
emission SHALL be deferred to temporal-unit completion and dropped if the temporal unit
turns out to end the CMVS or become `Unknown`. The check SHALL be suppressed only when
caller-provided external HLS declares at least one sequence header (consistent with the
distinct-mlayer and active-sequence-limit gates); an external channel declaring no
sequence header SHALL NOT suppress it.

#### Scenario: flag disagreement inside a CMVS

- **GIVEN** a CMVS begun by a temporal unit containing a CLK with an accompanying MSDO
- **AND** two extended layers whose activated sequence headers disagree on
  `monotonic_output_order_flag`
- **WHEN** the second of the two headers is activated
- **THEN** validation SHALL emit `sequence-state/monotonic-output-order-mismatch` with
  spec section § 6.4.1.

#### Scenario: disagreement outside any CMVS is not flagged

- **GIVEN** two independent extended layers with no MSDO and no global layer
  configuration record in the bitstream
- **WHEN** their activated sequence headers disagree on `monotonic_output_order_flag`
- **THEN** validation SHALL NOT emit `sequence-state/monotonic-output-order-mismatch`.

#### Scenario: provisional-Inside redefinition before a CMVS-ending CLK is not flagged

- **GIVEN** a CMVS that is committed `Inside`
- **WHEN** a temporal unit begins with a same-CVS sequence-header redefinition that
  disagrees on `monotonic_output_order_flag`, followed by an MSDO-less CLK that ends the
  CMVS for that temporal unit (§ 7.3.2 end condition 2)
- **THEN** validation SHALL NOT emit `sequence-state/monotonic-output-order-mismatch`
  (the provisional verdict is deferred and dropped once the CLK is observed).

### Requirement: Intra-CVS operating-point buffer-delay sum constancy

`splot-validate` SHALL track the last explicitly signaled
`ops_decoder_buffer_delay + ops_encoder_buffer_delay` sum per
`(obu_xlayer_id, ops_id, operating-point index)` and SHALL emit
`decoder-model/buffer-delay-sum-changed` (severity `error`, AV2 v1.0.0 § 6.10.5
with § 6.4.13) when the same triple is redefined within one coded video sequence,
with no intervening OPS reset, both signalings explicitly carrying decoder-model
info, and a differing sum. Absent decoder-model info (including Annex E
resource-availability defaults) SHALL NOT participate in any comparison, and a
defining OPS that omits decoder-model info for a previously tracked operating
point SHALL clear that triple's stored baseline (Annex E.1 non-persistence).

#### Scenario: intra-CVS OPS redefinition changes the sum

- **GIVEN** an operating point set defining an operating point with explicit
  `ops_decoder_buffer_delay + ops_encoder_buffer_delay == S`
- **WHEN** a later OPS in the same coded video sequence redefines the same
  `(obu_xlayer_id, ops_id, operating-point index)` without an OPS reset, with
  explicit decoder-model info whose sum differs from `S`
- **THEN** validation SHALL emit `decoder-model/buffer-delay-sum-changed` with
  severity `error`.

#### Scenario: redefinition across a CVS boundary is not an error

- **GIVEN** an operating point with an explicit buffer-delay sum
- **WHEN** the same triple is redefined with a different sum after a CLK starts a
  new coded video sequence for that extended layer
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed`.

#### Scenario: redefinition without explicit decoder-model info is ignored

- **GIVEN** an operating point with an explicit buffer-delay sum
- **WHEN** the same triple is redefined in the same CVS without
  `ops_decoder_model_info_for_this_op_present_flag` set
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed` and
  SHALL NOT compare against any default values.

### Requirement: Cross-boundary buffer-delay sum advisory

`splot-validate` SHALL emit `decoder-model/buffer-delay-sum-changed-across-cvs`
(severity `warning`, AV2 v1.0.0 § 6.4.13 / § 6.10.5) when explicitly signaled
buffer-delay sums change across a coded-video-sequence or OPS-reset boundary:
either the activated sequence header's `seq_decoder_model_info()` sum changing
across a CLK boundary within the same extended layer (frame-confirmed activations
only), or an operating point's sum changing across a CVS or OPS-reset boundary for
the same triple. The diagnostic message SHALL state that the constraint scope is
ambiguous in the specification and the finding is advisory under the broad
reading. A frame-confirmed activated header that omits `seq_decoder_model_info()`
SHALL clear that extended layer's stored baseline (Annex E.1 non-persistence).

#### Scenario: activated sequence headers disagree across a CLK

- **GIVEN** a frame-confirmed activated sequence header with explicit
  `seq_decoder_model_info()` sum `S` for an extended layer
- **WHEN** a CLK starts a new CVS for that extended layer and its frame-confirmed
  activated header carries explicit decoder-model info with a sum differing from
  `S`
- **THEN** validation SHALL emit
  `decoder-model/buffer-delay-sum-changed-across-cvs` with severity `warning`.

#### Scenario: headers without decoder-model info never fire the advisory

- **GIVEN** consecutive coded video sequences whose activated sequence headers
  omit `seq_decoder_model_info()`
- **WHEN** the validator runs
- **THEN** validation SHALL NOT emit
  `decoder-model/buffer-delay-sum-changed-across-cvs`.

#### Scenario: external HLS suppresses both decoder-model diagnostics

- **GIVEN** validation options with caller-provided external HLS
- **WHEN** any buffer-delay sum change is observed
- **THEN** validation SHALL NOT emit `decoder-model/buffer-delay-sum-changed` or
  `decoder-model/buffer-delay-sum-changed-across-cvs`.

### Requirement: Annex A profile constraints

The validator SHALL check the activated sequence header against the AV2
profile definitions (Annex A.2 Table A.1): a reserved `seq_profile_idc`
(5–30), a `chroma_format_idc` outside the profile's allowed set, or a
`bit_depth_idc` outside 0–1 for profiles 0–4 SHALL each produce an error
diagnostic citing Annex A.2. The Configurable profile (31) SHALL NOT be
checked against chroma/bit-depth sets (Table A.1 leaves them unconstrained).

#### Scenario: 4:2:2 under a 4:2:0 profile

- **WHEN** an activated sequence header signals `seq_profile_idc = 0` with
  `chroma_format_idc = CHROMA_FORMAT_422`
- **THEN** `annex-a/profile-chroma-format-mismatch` (error) is emitted

#### Scenario: configurable profile is unconstrained

- **WHEN** an activated sequence header signals `seq_profile_idc = 31` with
  any chroma format
- **THEN** no profile-mismatch diagnostic is emitted

### Requirement: Annex A level and tier value spaces

The validator SHALL flag reserved level indices (Table A.7: 22–30) on
activated `seq_level_idx` and observed `ops_level_idx` values as errors, and
SHALL flag `seq_tier = 1` below level 4.0 as a warning (Table A.9 NOTE — a
non-normative source, hence advisory severity).

#### Scenario: reserved level index

- **WHEN** an activated sequence header signals `seq_level_idx = 25`
- **THEN** `annex-a/level-reserved` (error) is emitted

### Requirement: Annex A static level limits

The validator SHALL enforce the static conformance block of Annex A.4 for a
parsed intra frame header under an activated sequence header whose
`seq_level_idx` maps into Tables A.8/A.9 (not 31, not reserved):
`FrameWidth * FrameHeight <= MaxPicSize`, `FrameWidth <= MaxHSize`,
`FrameHeight <= MaxVSize`, `NumTiles <= MaxTiles`, `TileCols <= MaxTileCols`,
and `FrameWidth, FrameHeight >= 16`, each violation an error diagnostic
citing Annex A.4. Level 31 SHALL disable all of these.

#### Scenario: frame exceeds the level picture size

- **WHEN** a level-2.0 stream carries an intra frame with
  `FrameWidth * FrameHeight > 147456`
- **THEN** `annex-a/frame-size-exceeds-level` (error) is emitted

#### Scenario: maximum-parameters level

- **WHEN** `seq_level_idx = 31`
- **THEN** no level-limit diagnostics are emitted for any frame size

### Requirement: MSDO sub-stream PTL floor agreement

The validator SHALL enforce the § 6.6 sub-stream constraint sentences:
`multistream_profile_idc` SHALL be ≥ every `sub_stream_max_profile[i]`, and a
sequence header activated by the i-th sub-stream (frame-confirmed, mapped via
`sub_xlayer_id[i]`) SHALL NOT exceed the declared `sub_stream_max_profile[i]`
/ `sub_stream_max_level[i]` / `sub_stream_max_tier[i]`, in either arrival
order (MSDO before or after the activation).

#### Scenario: substream level exceeds the declared maximum

- **WHEN** an MSDO declares `sub_stream_max_level[0] = 4` for
  `sub_xlayer_id[0] = 1` and a frame-confirmed sequence header with
  `seq_level_idx = 8` activates on extended layer 1
- **THEN** `msdo/substream-level-exceeds-max` (error, § 6.6) is emitted

#### Scenario: equality passes

- **WHEN** the activated header's `seq_level_idx` equals
  `sub_stream_max_level[i]`
- **THEN** no substream-max diagnostic is emitted

### Requirement: MSDO DOH-constraint flag requirement

The validator SHALL emit an error when, definitively inside a coded
multistream video sequence, any frame-confirmed activated sequence header has
`monotonic_output_order_flag = 0` while the recorded MSDO has
`multistream_doh_constraint_flag = 0` (§ 6.6).

#### Scenario: non-monotonic layer without the DOH flag

- **WHEN** a CMVS-inside activated header signals
  `monotonic_output_order_flag = 0` and the MSDO's
  `multistream_doh_constraint_flag` is 0
- **THEN** `msdo/doh-constraint-required` (error, § 6.6) is emitted

### Requirement: non-RAP MSDO identity

The validator SHALL compare each temporal unit's MSDO payload against the
previous MSDO at temporal-unit end and emit an error when the temporal unit
is not a random access point (§ 7.4.1: contains no CLK/OLK/RAS OBU) and the
payloads differ (§ 7.3.8.2). A random-access-point temporal unit SHALL update
the reference payload without a comparison.

#### Scenario: changed MSDO outside a random access point

- **WHEN** a temporal unit without CLK/OLK/RAS carries an OBU_MSDO whose
  payload differs from the previous OBU_MSDO
- **THEN** `msdo/non-rap-not-identical` (error, § 7.3.8.2) is emitted at
  temporal-unit end

#### Scenario: changed MSDO at a random access point

- **WHEN** a temporal unit containing a CLK carries a changed OBU_MSDO
- **THEN** no identity diagnostic is emitted and the reference updates

### Requirement: MSDO and activated global LCR agreement

The validator SHALL enforce the § 6.8.2 agreement constraints when an
OBU_MSDO and an activated global layer configuration record are present in
the same coded multistream video sequence, evaluated when CMVS membership is
final: stream-count equality, sub-xlayer containment, aggregate-info
consistency (Annex A.3/A.1 mappings, level, tier), per-substream PTL
equality, and DOH-flag equality. An observed but never-activated global LCR
SHALL trigger none of these.

#### Scenario: stream count disagrees

- **WHEN** a CMVS contains an MSDO with `num_streams_minus_2 + 2 = 2` and an
  activated global LCR with `LcrMaxNumXLayerCount = 3`
- **THEN** `lcr/msdo-stream-count-mismatch` (error, § 6.8.2) is emitted

#### Scenario: unactivated global LCR is inert

- **WHEN** a global LCR is observed but no frame-confirmed activation
  resolves to it
- **THEN** no § 6.8.2 agreement diagnostic is emitted

### Requirement: LCR DOH-constraint flag requirement

The validator SHALL emit `lcr/doh-constraint-required` (error, § 6.8.2) when,
with CMVS membership final, any frame-confirmed activated sequence header has
`monotonic_output_order_flag = 0` while the activated global LCR has
`lcr_doh_constraint_flag = 0`.

#### Scenario: non-monotonic layer without the LCR DOH flag

- **WHEN** a CMVS-inside activated header signals
  `monotonic_output_order_flag = 0` and the activated global LCR's
  `lcr_doh_constraint_flag` is 0
- **THEN** `lcr/doh-constraint-required` is emitted

### Requirement: CMVS boundary-set identity

The validator SHALL emit `cmvs/boundary-set-mismatch` (error, § 7.3.2) when
the MSDO-derived coded-multistream-video-sequence boundary set decidably
disagrees with the MSDO-plus-LCR-derived set, and SHALL stay silent in every
Unknown tracker state.

#### Scenario: undecidable stays silent

- **WHEN** the CMVS tracker cannot decide both boundary sets
- **THEN** no boundary diagnostic is emitted

### Requirement: Annex A interoperability-point OBU presence

The validator SHALL enforce the Table A.4 MSDO/LCR presence requirements at
coded-video-sequence scope with: the interoperability point taken from the
MSDO's `multistream_profile_idc` when an MSDO is present, else from
frame-confirmed activated headers; only activated global LCRs satisfying the
global-LCR arms; per-temporal-unit observation attribution that assigns a
CLK-bearing temporal unit's HLS OBUs to the new coded video sequence
(§ 7.3.6); windows spanning the whole coded video sequence; and suppression
when external HLS is provided.

#### Scenario: multi-xlayer stream without MSDO

- **WHEN** a profile-0 CVS contains two distinct non-global `obu_xlayer_id`
  values across its temporal units and no OBU_MSDO
- **THEN** `annex-a/msdo-required-for-iop` (error) is emitted at CVS end

#### Scenario: unactivated global LCR does not satisfy the arm

- **WHEN** an IOP2 CVS requires a global LCR and contains one that is never
  activated
- **THEN** the presence requirement still fails

#### Scenario: pre-CLK MSDO belongs to the new sequence

- **WHEN** a temporal unit carries an OBU_MSDO before the CLK that starts a
  new coded video sequence
- **THEN** the MSDO counts toward the new sequence's window, not the prior
  one

### Requirement: LCR PTL ceilings constrain activated headers

The validator SHALL enforce the § 6.8.5 ceiling sentences: when
`lcr_seq_profile_tier_level_info(i)` is present in the LCR activated by
extended layer `i`'s frame-confirmed sequence header, the header's
`seq_profile_idc`, `seq_level_idx`, `seq_tier`, and
`seq_max_mlayer_cnt_minus_1 + 1` SHALL each be ≤ the corresponding
LCR-declared maximum, with equality passing and absent PTL info comparing
nothing.

#### Scenario: level exceeds the LCR ceiling

- **WHEN** a frame-confirmed header with `seq_level_idx = 8` activates an
  LCR declaring `lcr_max_level_idx[i] = 4` for its layer
- **THEN** `lcr/ptl-level-exceeds-max` (error, § 6.8.5) is emitted

#### Scenario: equality passes

- **WHEN** the header's value equals the LCR-declared maximum
- **THEN** no ceiling diagnostic is emitted

### Requirement: LCR rep-info equality with activated headers

The validator SHALL enforce the § 6.8.8 equality sentences between an
activated LCR's representation info and each sequence header activated by
the same extended layer (frame dimensions, bit depth, chroma format,
cropping window), emitting `lcr/rep-info-mismatch` (error) naming the
disagreeing field; absent rep-info SHALL compare nothing.

#### Scenario: dimension mismatch

- **WHEN** an activated LCR declares `lcr_max_pic_width = 1920` and the
  activated header has `max_frame_width_minus_1 + 1 = 1280`
- **THEN** `lcr/rep-info-mismatch` (error, § 6.8.8) is emitted naming the
  width field

### Requirement: timecode inference requires a previous value

The validator SHALL emit an error when a § 6.16.7 timecode omits
`seconds_value`, `minutes_value`, or `hours_value` and no previous set of
clock timestamp syntax elements in decoding order carried that value, per the
mirror's "it is required that such a previous … shall have been present"
sentences. The decoding-order chain is keyed per the carrying OBU's concrete
`(obu_xlayer_id, obu_mlayer_id)`: METADATA_TYPE_TIMECODE is layer-specific
(§ 6.16.3 Table 6.17), so a timecode on one embedded layer is not the
"previous set" of one on a different embedded layer and must not seed its
inference; a `LAYER_UNSPECIFIED` timecode chains per its own carrying scope.

#### Scenario: inferred seconds without any previous timecode

- **WHEN** the first timecode in scope omits `seconds_value`
  (`full_timestamp_flag = 0`, `seconds_flag = 0`)
- **THEN** `metadata/timecode-inferred-without-previous` (error, § 6.16.7)
  is emitted naming `seconds_value`

#### Scenario: inference after a present value passes

- **WHEN** a timecode omits `seconds_value` after a previous set carried it
- **THEN** no inference diagnostic is emitted

#### Scenario: inference is keyed per targeted embedded layer

- **WHEN** a full-timestamp `LAYER_CURRENT` timecode on `(obu_xlayer_id 0,
  obu_mlayer_id 0)` is followed by a `LAYER_CURRENT` timecode on `(obu_xlayer_id
  0, obu_mlayer_id 1)` that omits `seconds_value`
- **THEN** `metadata/timecode-inferred-without-previous` (error, § 6.16.7) is
  emitted: the `(0, 0)` timecode is not the previous set for `(0, 1)`

### Requirement: timecode n_frames bound

The validator SHALL emit an error when a § 6.16.7 timecode's `n_frames` is not
less than `maxPicPerSecond` (`ceil(time_scale / TicksPerPicture)`) and an
in-scope content interpretation establishes `ci_timing_info_present_flag == 1`
at or after the layer's § 7.3.8.11 random-access-point epoch, per the mirror's
"When ci_timing_info_present_flag is equal to 1, n_frames shall be less than
maxPicPerSecond". The bound is paired against the in-scope CI timing in both
arrival orders (a content interpretation arriving after the timecode
re-evaluates), and the diagnostic anchors at the offending timecode metadata
OBU. The § 6.16.3 layer targeting scopes the pairing: a derivable `LAYER_VALUES`
timecode naming only some embedded layers does not pair with an untargeted
layer's CI, while a timecode whose targeting is not bitstream-derivable
(`LAYER_UNSPECIFIED`) compares nothing for the bound — the spec leaves its layer
association unspecified, so no CI's rate binds it. § 7.3.6 coded-video-sequence
boundaries are per extended layer: a CLK for one extended layer does not prune a
global timecode observation aimed at another extended layer. The CI-re-send
dedup is epoch-aware: an identical CI repeated in a later temporal unit with no
random access point in between does not re-report the already-paired
observation, while a CI re-sent in a random-access temporal unit re-pairs the
new coded video sequence's observations at the CLK.

#### Scenario: n_frames at the rate ceiling is flagged

- **WHEN** a timecode carries `n_frames == maxPicPerSecond` and an in-scope CI
  establishes the timing
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is
  emitted, anchored at the timecode OBU

#### Scenario: n_frames just below the ceiling passes

- **WHEN** a timecode carries `n_frames == maxPicPerSecond - 1`
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted

#### Scenario: targeting excludes an untargeted layer's CI

- **WHEN** a `LAYER_VALUES` timecode targets embedded layer 1 only, embedded
  layer 0 carries a low-rate CI the `n_frames` would exceed, and embedded layer
  1 carries a CI under which the `n_frames` is legal
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted

#### Scenario: unspecified targeting compares nothing

- **WHEN** a `LAYER_UNSPECIFIED` timecode whose `n_frames` would exceed an
  extended-layer-0 CI's low-rate `maxPicPerSecond` is observed
- **THEN** no `metadata/timecode-n-frames-exceeds-rate` diagnostic is emitted
  (the spec does not say which layers the timecode applies to, so no CI's rate
  binds it — a zero-false-positive rule)

#### Scenario: a global observation survives an unrelated layer's CLK

- **WHEN** a global `LAYER_VALUES` timecode targeting extended layer 1 is
  observed, a CLK for extended layer 0 only follows, and a low-rate CI for
  extended layer 1 that the `n_frames` exceeds then arrives
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is
  emitted: the extended-layer-0 CLK does not prune the layer-1 observation

#### Scenario: an identical CI repeat with no random access point reports once

- **WHEN** a CI establishes a low-rate timing and a violating timecode is
  reported, then the identical CI is re-sent in a later temporal unit with no
  CLK or OLK in between
- **THEN** `metadata/timecode-n-frames-exceeds-rate` is emitted exactly once
  (the epoch-aware dedup does not replay the recheck for the already-paired
  observation)

#### Scenario: a CI re-sent across a random access point still pairs

- **WHEN** a pre-RAP CI establishes a low-rate timing, a later random-access
  temporal unit holds a timecode that violates the bound followed by the same CI
  re-sent with identical timing and then a CLK, and the deferred pre-RAP pairing
  is dropped by the § 7.3.8.11 reinitialization
- **THEN** `metadata/timecode-n-frames-exceeds-rate` (error, § 6.16.7) is still
  emitted, anchored at the timecode OBU (the new coded video sequence's timecode
  is re-paired against the re-sent CI at the CLK)

### Requirement: timecode counting_type reserved value

The validator SHALL warn when a § 6.16.7 `counting_type` is in the reserved
range 7..31. The counting_type table marks those values "reserved" with no
"shall" forbidding them (§ 6.16.7 only recommends counting_type "should be the
same for all pictures"), so a reserved value is a decoder-ignored producer
anomaly (warning), matching the established reserved-value pattern.

#### Scenario: reserved counting_type is warned

- **WHEN** a timecode carries `counting_type == 7`
- **THEN** `metadata/timecode-counting-type-reserved` (warning, § 6.16.7) is
  emitted and the report stays conformant (no error)

#### Scenario: a defined counting_type is silent

- **WHEN** a timecode carries `counting_type == 6` (the highest defined value)
- **THEN** no `metadata/timecode-counting-type-reserved` diagnostic is emitted

### Requirement: decoded frame hash reserved field

The validator SHALL warn when a § 6.16.13 `metadata_decoded_frame_hash()` carries
a non-zero `reserved` bit, per the mirror's "reserved shall be set to 0 and
ignored by decoders". The bit is decoder-ignored, so the finding is a producer
anomaly (warning), matching the established decoder-ignored reserved-field
pattern; the `plane_hash` / `frame_hash` verification against decoded output
stays decoder-blocked.

#### Scenario: non-zero reserved bit is warned

- **GIVEN** a `metadata_decoded_frame_hash()` OBU whose `reserved` bit is 1
- **WHEN** the validator runs
- **THEN** it SHALL emit a `metadata/decoded-frame-hash-reserved-nonzero`
  warning (§ 6.16.13)

#### Scenario: zero reserved bit is silent

- **GIVEN** a `metadata_decoded_frame_hash()` OBU whose `reserved` bit is 0
- **WHEN** the validator runs
- **THEN** it SHALL NOT emit a `metadata/decoded-frame-hash-reserved-nonzero`
  warning

### Requirement: HLS availability replays from random access points

The validator SHALL verify, for every § 7.4.1 random access point, that each
HLS OBU referenced at or after it was (re)sent in or after the random access
point's temporal unit (§ 7.3.8.1) — a resend inside a temporal unit carrying
LEADING_* frame OBUs SHALL NOT qualify (those temporal units drop under
random access), while temporal units whose leading-ness is undecidable SHALL
qualify (under-approximation, never a false positive). Externally-declarable
kinds follow the documented partial-declaration suppression policy.

#### Scenario: sequence header only before the RAP

- **WHEN** a frame after a CLK references a sequence header last sent in an
  earlier temporal unit and not resent in the CLK's temporal unit
- **THEN** a replay diagnostic citing § 7.3.8.1 is emitted

#### Scenario: resend in the RAP temporal unit passes

- **WHEN** the referenced HLS OBU is resent inside the random access point's
  temporal unit
- **THEN** no replay diagnostic is emitted

#### Scenario: leading-TU resend does not qualify

- **WHEN** the only post-RAP resend sits in a temporal unit carrying an
  OBU_LEADING_* frame
- **THEN** the replay diagnostic is emitted

### Requirement: coded-frame-unit segmentation and presence order

The validator SHALL segment each (obu_xlayer_id, obu_mlayer_id,
obu_tlayer_id) triple's consecutive OBUs into coded frame units and enforce
the § 7.3.3/§ 7.3.4 presence order — content interpretation (zero or one),
multi-frame headers, the pre-frame region (buffer-removal timing with the
zero-or-one bound in non-output units, quantization matrices, film grain,
prefix metadata), a single coded frame (same-type tile OBUs with
`is_first_tile_group` 1-then-0, or exactly one SEF), and the suffix-metadata
tail — with OBU_PADDING position-free and any unit containing an OBU whose
classification is undecidable treated as Unknown (no diagnostics).

#### Scenario: prefix metadata after the coded frame

- **WHEN** a non-suffix metadata OBU follows the coded frame in its unit
- **THEN** a `frame-unit/` presence-order error citing § 7.3.3 is emitted

#### Scenario: second BRT in a non-output unit

- **WHEN** a coded non-output frame unit carries two buffer-removal-timing
  OBUs
- **THEN** an error citing § 7.3.4 is emitted (an output unit with two is
  conforming)

#### Scenario: first-tile-group flag

- **WHEN** the first tile OBU of a coded frame has `is_first_tile_group = 0`
  or a later one has `is_first_tile_group = 1`
- **THEN** an error citing § 7.3.3/§ 7.3.4 is emitted

#### Scenario: undecidable unit stays silent

- **WHEN** a unit contains a frame OBU whose output classification is
  unavailable (unsupported parse path)
- **THEN** no segmentation diagnostic is emitted for that unit

### Requirement: content interpretation in the first coded frame unit

The validator SHALL enforce § 7.3.8.10: a content-interpretation OBU may
appear only in its layer's first coded frame unit of the temporal unit, and
the § 6.16.5/§ 6.16.6 first-coded-picture indication halves follow the same
segmentation.

#### Scenario: CI in a later frame unit

- **WHEN** a CI OBU appears in the second coded frame unit of its layer's
  temporal unit
- **THEN** an error citing § 7.3.8.10 is emitted

### Requirement: coded-extended-layer-unit structure

The validator SHALL enforce the § 7.3.6 in-unit OBU order (layer
configuration records, operating point sets, atlas segments, sequence
headers, then per-embedded-layer frame units in ascending `obu_mlayer_id`,
with PADDING position-free) and the § 7.3.6 constraint family: at least one
coded output frame unit, non-output-implies-output per embedded layer, one
OrderHint across all output units, the CLK/OLK first-frame-unit and
lowest-layer rules, no CLK+OLK mix, all-leading-or-none, and
content-interpretation only in each layer's first frame unit. Units whose
classification is Unknown SHALL NOT fire.

#### Scenario: sequence header after a frame unit

- **WHEN** a CELU carries a sequence header after its first coded frame unit
  began
- **THEN** a `celu/` ordering error citing § 7.3.6 is emitted

#### Scenario: output units disagree on OrderHint

- **WHEN** two coded output frame units in one CELU carry different parsed
  `order_hint` values
- **THEN** an error citing § 7.3.6 is emitted

#### Scenario: CLK and OLK mixed

- **WHEN** one CELU contains both a CLK and an OLK OBU
- **THEN** an error citing § 7.3.6 is emitted

### Requirement: DOH-gated OrderHint agreement

The validator SHALL enforce the § 7.3.7 DOH constraints when the recorded
`multistream_doh_constraint_flag` or `lcr_doh_constraint_flag` equals 1: one
OrderHintBits for all frame units in the temporal unit and one OrderHint
across the coded output frame units of the temporal unit's CELUs; with the
flag 0 the checks SHALL stay silent.

#### Scenario: cross-CELU OrderHint mismatch under the DOH flag

- **WHEN** the DOH flag is 1 and two CELUs' output frame units in one
  temporal unit carry different OrderHint values
- **THEN** an error citing § 7.3.7 is emitted

#### Scenario: flag off stays silent

- **WHEN** no DOH constraint flag is set
- **THEN** no DOH OrderHint diagnostic is emitted

