# Validator spec delta — frame-header core foundation

## ADDED Requirements

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
