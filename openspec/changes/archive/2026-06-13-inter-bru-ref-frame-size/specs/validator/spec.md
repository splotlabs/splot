# validator delta: inter-bru-ref-frame-size

Advances `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` by enforcing the § 6.17.2 `use_bru == 1`
reference-frame-dimension equality from already-modeled frame-header and §7.23 state.

## ADDED Requirements

### Requirement: a use_bru frame matches its bru_ref reference dimensions

The validator SHALL, for an inter frame with `use_bru == 1` whose resolved
FrameWidth/FrameHeight is known (`core.frame_size`), verify the § 6.17.2 conditions
(docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2, mirror :4594-4595) that
`RefFrameWidth[ref_frame_idx[bru_ref]] == FrameWidth` and
`RefFrameHeight[ref_frame_idx[bru_ref]] == FrameHeight`, when the `bru_ref`-selected slot
(bounds-checked against the recorded `ref_frame_idx`) is PROVEN valid in the §7.23 buffer
(`SlotState::Valid`, so `RefFrameWidth`/`RefFrameHeight` are known). A mismatch produces the
error diagnostic `frame-header/bru-ref-frame-size-mismatch`. A slot the buffer cannot prove
valid (`Unknown` or `ProvenInvalid`) has no proven dimensions and is silent. This is distinct
from `frame-header/ref-frame-scale-ratio` (the ≤2x/≤16x scaling bound that applies to every
`ref_frame_idx[i]`): a BRU frame must match the reference it updates exactly.

#### Scenario: a BRU frame whose size differs from its reference fires

- **WHEN** an inter frame has `use_bru == 1`, a resolved frame size, and a proven-valid
  `bru_ref` reference slot whose stored dimensions differ from the frame size
- **THEN** an error diagnostic `frame-header/bru-ref-frame-size-mismatch` (§ 6.17.2) is
  produced

#### Scenario: a matching BRU frame stays silent

- **WHEN** the proven-valid `bru_ref` reference slot's dimensions equal the frame size
- **THEN** no `frame-header/bru-ref-frame-size-mismatch` diagnostic is produced

#### Scenario: an unproven bru_ref slot is not judged

- **WHEN** the `bru_ref` slot is `Unknown` or `ProvenInvalid` (no proven dimensions)
- **THEN** no `frame-header/bru-ref-frame-size-mismatch` diagnostic is produced

#### Scenario: a non-BRU frame is not checked

- **WHEN** an inter frame does not set `use_bru == 1`
- **THEN** no `frame-header/bru-ref-frame-size-mismatch` diagnostic is produced

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
