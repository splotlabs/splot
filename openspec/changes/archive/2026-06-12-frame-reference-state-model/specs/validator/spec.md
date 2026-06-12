# validator delta: frame-reference-state-model

Models the § 7.23 reference-frame buffer state from parsed intra headers.

## ADDED Requirements

### Requirement: reference-frame buffer state model

The validator SHALL maintain a per-extended-layer per-slot
reference-frame state (`RefValid`, `RefOrderHint`, frame dimensions)
updated per the § 7.23 reference frame update process from each frame's
parsed refresh mask, OrderHint, and dimensions, with grounded reset and
show-existing-frame semantics. A frame whose refresh mask is not parsed
SHALL poison the affected slots until the next grounded reset — slot
state is never guessed. The derived state SHALL be threaded into the
frame-header parse input, and any reference-state check that becomes
locally decidable SHALL carry its governing citation.

#### Scenario: intra stream tracks slot state

- **WHEN** a sequence of completed intra frames refreshes slots
- **THEN** the per-slot state follows the § 7.23 update process

#### Scenario: unparsed refresh mask poisons

- **WHEN** a frame's refresh mask is unparsed
- **THEN** dependent slot state becomes unknown and dependent judgments
  drop until a grounded reset

#### Scenario: newly decidable reference check fires

- **WHEN** a parsed frame references a slot the modeled state proves
  invalid
- **THEN** the diagnostic with the governing citation is emitted
