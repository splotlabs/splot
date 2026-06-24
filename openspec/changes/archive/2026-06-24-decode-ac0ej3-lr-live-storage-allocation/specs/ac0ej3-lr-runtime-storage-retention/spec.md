## MODIFIED Requirements

### Requirement: ac0ej3 LR Runtime Storage Retention Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-RUNTIME-STORAGE-RETENTION` as the
fail-closed ac0ej3 storage-footprint planning frontier after
classified-Wiener storage-helper wiring. The runtime SHALL derive the two
active-bit-depth loop-restoration frame-buffer storage shapes (`CurrFrame` and
`CdefFrame`; 10-bit for the ac0ej3 mission stream) and the frame-wide
`LrTxSkip` grid dimensions from parsed AV2 sequence/frame facts, SHALL enforce
the per-frame decoded-frame limit and the aggregate retained-storage limit for
those retained storage shapes, and SHALL stop before any unpopulated storage is
used as decoded source values. The live ac0ej3 diagnostic SHALL be superseded
by `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION` once explicit unpopulated storage
shells are allocated.

#### Scenario: Storage-retention frontier derives live storage shapes
- **WHEN** the local ac0ej3 mission stream reaches active classified-luma
  Wiener NS loop-restoration units
- **THEN** the runtime derives the required active-bit-depth current/CDEF frame
  storage footprint (10-bit for the ac0ej3 mission stream) and `LrTxSkip` grid
  dimensions
- **AND** the storage-retention row remains the proof that those shapes and
  byte budgets are derived before live storage allocation

#### Scenario: Storage footprint limits fail before unsupported diagnostic
- **WHEN** caller-provided decode limits are lower than the derived
  active-bit-depth frame storage footprint (10-bit for the ac0ej3 mission
  stream) or aggregate retained-storage footprint including `LrTxSkip` storage
- **THEN** the runtime returns `decode/resource-limit`
- **AND** the runtime does not emit a storage-allocation unsupported-feature
  diagnostic

#### Scenario: No fabricated classification values
- **WHEN** the runtime has only derived storage dimensions and byte budgets
- **THEN** it SHALL NOT call the storage-backed `FilterClass` derivation helper
  with zero-filled or otherwise fabricated `CurrFrame`, `CdefFrame`, or
  `LrTxSkip` values
