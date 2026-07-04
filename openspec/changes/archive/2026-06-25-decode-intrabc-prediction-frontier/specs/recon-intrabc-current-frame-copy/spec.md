## ADDED Requirements

### Requirement: Current-frame IntrABC copy primitive

`splot-recon` SHALL expose a checked current-frame workspace copy primitive for
`RECON-INTRABC-CURRENT-FRAME-COPY`. The primitive SHALL copy one same-plane
source rectangle into one same-plane target rectangle inside a
`CurrentFrameWorkspace`, SHALL validate source and target geometry before
mutation, SHALL preserve sample values exactly, SHALL be safe when source and
target rectangles overlap by reading through bounded scratch storage, and SHALL
return typed `ReconError` failures instead of panicking.

#### Scenario: In-frame luma copy succeeds

- **WHEN** a caller asks a current-frame workspace to copy an in-bounds luma
  source rectangle into an equal-sized in-bounds target rectangle
- **THEN** the target rectangle contains the exact source samples after the copy
- **AND** samples outside the target rectangle are unchanged

#### Scenario: Overlapping copy reads the original source

- **WHEN** a caller copies between overlapping source and target rectangles in
  the same workspace plane
- **THEN** the target rectangle is populated from the source samples as they
  existed before target mutation
- **AND** row order cannot corrupt the copied prediction

#### Scenario: Invalid copy is typed and fail-closed

- **WHEN** the source rectangle, target rectangle, plane, or shape is invalid
- **THEN** the primitive returns a typed `ReconError`
- **AND** it does not partially mutate the target rectangle
