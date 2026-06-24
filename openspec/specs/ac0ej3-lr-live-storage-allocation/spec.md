## Purpose

Track the ac0ej3 fail-closed runtime frontier after storage-footprint planning,
before live decoded samples or `LrTxSkip` values are populated.

## Requirements

### Requirement: ac0ej3 LR Live Storage Allocation Frontier

The decoder SHALL track `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION` as the live
fail-closed ac0ej3 frontier after runtime storage-footprint planning. After
active Wiener NS LR unit syntax, §7.20.1 source-bound facts, §7.20.2/§7.20.3
source-read state, §7.20.4 classified-luma dependency coordinates, and
storage-footprint limit checks have succeeded, the runtime SHALL allocate
private unpopulated storage shells for the active-bit-depth `CurrFrame` and
`CdefFrame` buffers plus the frame-wide `LrTxSkip` grid, and SHALL stop before
any unpopulated value can be consumed as decoded source data.

#### Scenario: Live ac0ej3 reaches live-storage allocation frontier

- **WHEN** the local ac0ej3 mission stream reaches active classified-luma
  Wiener NS loop-restoration units
- **THEN** the runtime allocates unpopulated active-bit-depth storage shells for
  `CurrFrame`, `CdefFrame`, and `LrTxSkip`
- **AND** the runtime returns `decode/unsupported-feature` referencing
  `DECODE-AC0EJ3-LR-LIVE-STORAGE-ALLOCATION`
- **AND** the diagnostic states that decoded sample population, `LrTxSkip`
  values, filtering, output, and reference refresh are still unsupported

#### Scenario: Storage limits fail before shell allocation diagnostic

- **WHEN** caller-provided decode limits are lower than the derived frame
  storage footprint or aggregate retained-storage footprint
- **THEN** the runtime returns `decode/resource-limit`
- **AND** the runtime does not emit the live-storage allocation
  unsupported-feature diagnostic

#### Scenario: No fabricated storage-backed classification values

- **WHEN** the runtime has allocated only unpopulated storage shells
- **THEN** it SHALL NOT call storage-backed `FilterClass` derivation with
  zero-filled or otherwise fabricated `CurrFrame`, `CdefFrame`, or `LrTxSkip`
  values
