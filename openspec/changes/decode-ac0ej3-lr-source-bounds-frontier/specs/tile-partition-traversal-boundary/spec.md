## ADDED Requirements

### Requirement: Active Wiener NS LR Source-Bounds Frontier

The tile partition traversal boundary SHALL retain active frame-level Wiener NS
loop-restoration source-bound facts for the supported root LR frontier.
For each retained block, the facts SHALL identify the plane, luma 4x4 row and
column, selected LR unit row and column, current-plane block coordinates and
size, and the caller-resolved AV2 §7.20.1 luma source/stripe bounds. Failed
source-bound derivation MUST NOT commit LR-unit CDF mutations.

When an active Wiener NS LR unit reaches AV2 §5.20.10.6 with
`readFrameFilters == 0`, the boundary SHALL consume the entropy-coded per-unit
Wiener NS filter syntax needed to complete `read_lr()` before retaining the
source-bound facts. The decoded coefficients SHALL NOT be exposed as
reconstruction support by this boundary.

#### Scenario: Active source bounds are retained for a supported root unit

- **WHEN** a supported root LR frontier consumes an active
  frame-level Wiener NS unit
- **THEN** the frontier includes active source-bound facts for the covered
  loop-restore blocks
- **AND** each retained block cites the active unit row and column selected by
  the already-consumed LR unit syntax
- **AND** each retained block includes the §7.20.1 luma source and stripe bounds

#### Scenario: Per-unit Wiener NS filter syntax completes before bounds

- **WHEN** an active Wiener NS unit uses `readFrameFilters == 0`
- **THEN** the boundary consumes the required §5.20.10.6 per-unit filter syntax
- **AND** source-bound facts are retained only after that syntax succeeds
- **AND** the decoded filter coefficients are not reported as reconstruction
  output

#### Scenario: Inactive units do not retain active source blocks

- **WHEN** a supported root LR frontier consumes only inactive frame-level
  Wiener NS units
- **THEN** the frontier preserves the inactive unit selections
- **AND** the active source-bound list is empty

#### Scenario: Tile-clamped source bounds follow the sequence filter flag

- **WHEN** an active LR unit is consumed for a tile range smaller than the frame
- **AND** loop filters are disabled across tiles
- **THEN** retained source-bound facts use the tile MI range for `LumaStart*`
  and `LumaEnd*`
