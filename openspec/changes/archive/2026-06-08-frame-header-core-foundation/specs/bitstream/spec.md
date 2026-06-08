# Bitstream spec delta — frame-header core foundation

## ADDED Requirements

### Requirement: Frame header parse modes

The bitstream parser SHALL expose a mode that preserves the existing activation-prefix parse and a mode that parses additional frame-header core fields.

#### Scenario: Activation-prefix mode is compatible

- **GIVEN** a frame-bearing OBU payload
- **WHEN** the parser is called in activation-prefix mode
- **THEN** it SHALL read only the activation/reference fields currently consumed by the existing parser
- **AND** it SHALL return a status indicating activation-only coverage.

### Requirement: Frame-header core status

Every frame-header parse result SHALL carry an explicit parse status.

#### Scenario: Parser stops before deep frame syntax

- **GIVEN** a frame header that reaches unimplemented filtering, quantization, segmentation, tiling, transform, global-motion, or frame-film-grain syntax
- **WHEN** the parser stops before that syntax
- **THEN** the result SHALL indicate a partial status
- **AND** callers SHALL NOT infer that full trailing bits were validated.

### Requirement: Explicit state dependencies

The frame-header core parser SHALL receive active sequence, MFH, temporal-unit, and reference-state inputs explicitly.

#### Scenario: Required state is missing

- **GIVEN** a syntax branch requiring unavailable reference-frame state
- **WHEN** the parser reaches that branch
- **THEN** it SHALL return a typed unsupported/partial status or typed error
- **AND** it SHALL NOT guess reference counts, order hints, frame sizes, or validity.

### Requirement: Frame-size helper foundation

The parser SHALL model frame size as a typed value when dimensions can be derived exactly from parsed state.

#### Scenario: Frame size exceeds active sequence maximum

- **GIVEN** a parsed frame size
- **AND** an active sequence maximum frame size
- **WHEN** the frame size exceeds the sequence maximum
- **THEN** the validator SHALL be able to report a structured diagnostic.
