## ADDED Requirements

### Requirement: local decoder mission LR Live Tx-Skip Grid

The decoder SHALL track `DECODE-LR-LIVE-TX-SKIP-GRID` as a partial
local decoder mission Wiener NS LR prerequisite after live storage allocation. The live
storage shell SHALL populate its `LrTxSkip` grid only from a complete
`WienerNsLrTxSkipGrid`, SHALL preserve each retained value exactly, and SHALL
reject mismatched dimensions or attempted re-population before mutating live
storage.

#### Scenario: Complete retained grid populates live shell

- **WHEN** a live LR storage allocation receives a complete retained
  `WienerNsLrTxSkipGrid` with matching row and column dimensions
- **THEN** every live `LrTxSkip` slot is populated with the corresponding
  retained grid value
- **AND** the live allocation reports zero unpopulated `LrTxSkip` values
- **AND** decoded `CurrFrame` and `CdefFrame` samples remain unpopulated

#### Scenario: Dimension mismatch is rejected before mutation

- **WHEN** a live LR storage allocation receives a retained `LrTxSkip` grid with
  different dimensions than the allocated shell
- **THEN** the decoder returns a structured reconstruction error
- **AND** the live allocation retains all prior `LrTxSkip` population state

#### Scenario: Re-population is rejected before mutation

- **WHEN** a live LR storage allocation has already populated its `LrTxSkip`
  grid
- **AND** another retained grid is supplied
- **THEN** the decoder returns a structured reconstruction error
- **AND** the live allocation keeps the first populated values unchanged

#### Scenario: Live local decoder mission remains fail-closed before decoded samples

- **WHEN** the local decoder mission stream reaches active classified-luma
  Wiener NS loop-restoration units
- **THEN** the runtime still returns `decode/unsupported-feature`
- **AND** it does not claim live decoded samples, `FilterClass` retention,
  `SubclassLookup`, loop-restoration filtering/output, reference refresh,
  AVM/dav2d byte equality, or successful local decoder mission decode
