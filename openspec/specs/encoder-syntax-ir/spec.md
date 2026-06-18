# encoder-syntax-ir Specification

## Purpose
Track the private, deterministic `splot-encode` syntax-planning IR used to
stage future sequence/frame/tile/block/token decisions without exposing public
packet-producing behavior.

## Requirements
### Requirement: Private Syntax Planning IR

The encoder SHALL provide a private syntax-planning IR for future
sequence/frame/tile/block/token decisions, including `SequencePlan`,
`FramePlan`, `TilePlan`, `SuperBlockPlan`, `BlockDecision`,
`PredictionDecision`, `TransformDecision`, `QuantizedCoefficients`, and ordered
syntax/token events.

#### Scenario: IR is not exported as public API

- **WHEN** the crate root is inspected
- **THEN** the syntax-planning IR is not publicly re-exported
- **AND** downstream users cannot construct encoded packets from this IR

#### Scenario: Context lifecycle remains non-emitting

- **WHEN** frames are submitted through the existing encoder context lifecycle
- **THEN** `receive_packet` continues to return no coded packet until a later
  coded-frame feature lands

### Requirement: Deterministic Plan Ordering

The encoder SHALL store syntax plans in deterministic order using explicit
typed indices/newtypes and ordered event storage rather than unordered map
iteration. Tile, superblock, block, and syntax-event child collections SHALL be
zero-based and contiguous before a plan is accepted.

#### Scenario: Repeated construction renders the same plan

- **WHEN** equivalent sequence, frame, tile, superblock, block, and token
  decisions are constructed repeatedly
- **THEN** their debug rendering and iteration order are identical

#### Scenario: Out-of-order or non-contiguous children are rejected before mutation

- **WHEN** a plan constructor receives child decisions whose explicit indices
  are not strictly ordered or not zero-based and contiguous where required
- **THEN** construction fails with a typed planning error
- **AND** no caller-visible partially mutated plan is returned

### Requirement: Bounded Planning Constructors

The encoder SHALL validate planning dimensions, frame/sequence format
compatibility, child references, coefficient entries, coefficient transform
bounds, and event counts through bounded constructors before a future writer can
consume the plan.

#### Scenario: Invalid coefficient plans fail

- **WHEN** a quantized coefficient plan contains duplicate coefficient indices
  or zero-valued coefficient entries, or a block's coefficient EOB exceeds its
  transform area
- **THEN** construction fails with a typed planning error

#### Scenario: Invalid plan references fail

- **WHEN** a sequence, frame, tile, block, or token plan references mismatched
  frame format, an out-of-bounds frame size, a different tile/frame, or a missing
  superblock, block, or coefficient child
- **THEN** construction fails with a typed planning error

#### Scenario: Count arithmetic is checked

- **WHEN** a plan would exceed an implementation-defined planning count limit
  or an arithmetic operation would overflow
- **THEN** construction fails with a typed planning error instead of wrapping
