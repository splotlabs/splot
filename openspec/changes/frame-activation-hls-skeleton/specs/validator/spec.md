# validator spec delta

## ADDED Requirements

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

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
