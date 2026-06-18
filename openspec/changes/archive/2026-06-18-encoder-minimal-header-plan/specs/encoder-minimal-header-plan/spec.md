## ADDED Requirements

### Requirement: Private Minimal Header Plan

The encoder SHALL provide a private minimal header plan for the future writer
handoff. The plan SHALL name the sequence-header, first-frame-header, and
single-tile tile-group header intent needed before coded tile payload work, and
it SHALL remain private to `splot-encode`.

#### Scenario: Header plan is not public API

- **WHEN** the crate root is inspected
- **THEN** the minimal header plan is not publicly re-exported
- **AND** downstream users cannot construct coded packets from the plan

#### Scenario: Header plan records the first-frame header intent

- **WHEN** a valid current-subset encoder config and matching first input frame
  metadata are planned
- **THEN** the plan records one sequence header intent
- **AND** the plan records one first-frame header intent for that frame
- **AND** the plan records one tile-group header intent covering tile 0

### Requirement: Bounded Header Plan Construction

The encoder SHALL validate minimal header plans before construction succeeds.
Unsupported formats, zero dimensions, and frame/config metadata mismatches SHALL
fail with typed private planning errors.

#### Scenario: Valid current-subset frame metadata is accepted

- **WHEN** a plan is built for 8-bit YUV420 frame metadata whose visible luma
  size matches the encoder config
- **THEN** construction succeeds
- **AND** repeated construction yields equal plan values and stable debug output

#### Scenario: Unsupported or mismatched metadata is rejected

- **WHEN** a plan is built with zero config dimensions, unsupported bit depth or
  chroma layout, or frame metadata that disagrees with the encoder config
- **THEN** construction fails with a typed private header-planning error
- **AND** no partially constructed plan is returned

### Requirement: Header Planning Does Not Emit Packets

Minimal header planning SHALL NOT expose a successful public encode path and
SHALL NOT enqueue or return `Packet` values from the encoder context.

#### Scenario: Context lifecycle remains non-emitting

- **WHEN** a valid frame is submitted through `Context::send_frame`, the context
  is flushed, and packets are drained
- **THEN** `Context::receive_packet` does not return a coded `Packet`
- **AND** the context reaches `Finished` through the existing no-output
  lifecycle
