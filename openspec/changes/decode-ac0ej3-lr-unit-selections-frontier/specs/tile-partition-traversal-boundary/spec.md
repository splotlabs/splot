## ADDED Requirements

### Requirement: Frame-Level Wiener NS LR Unit Selection State

The tile partition traversal boundary SHALL preserve the supported frame-level
Wiener NS LR-unit selections in syntax order. Each selection SHALL identify the
plane, the absolute LR unit row and column after tile-origin offset adjustment,
and whether AV2 §5.20.10.5 `use_wiener_ns` selected active
`RESTORE_WIENER_NONSEP`. The boundary SHALL preserve existing aggregate consumed
and active unit counts, and failed traversal attempts MUST NOT commit LR-unit CDF
mutations.

#### Scenario: Inactive unit selection is retained

- **WHEN** a supported superblock-root LR frontier consumes an inactive
  frame-level Wiener NS unit
- **THEN** the frontier includes one selection with the corresponding plane,
  unit row, unit column, and `active = false`
- **AND** the aggregate active count remains zero

#### Scenario: Active unit selection is retained

- **WHEN** a supported superblock-root LR frontier consumes an active
  frame-level Wiener NS unit
- **THEN** the frontier includes one selection with the corresponding plane,
  unit row, unit column, and `active = true`
- **AND** callers can continue to fail closed before claiming loop-restoration
  reconstruction or output support

#### Scenario: Multi-unit syntax order is retained

- **WHEN** a supported superblock-root LR frontier covers multiple frame-level
  Wiener NS LR units
- **THEN** the frontier's selections are ordered by the §5.20.10.4 unit-row loop
  and then the unit-column loop
- **AND** each stored coordinate uses the tile-origin-adjusted LR unit index
