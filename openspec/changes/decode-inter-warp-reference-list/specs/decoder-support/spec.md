## MODIFIED Requirements

### Requirement: First-inter-frame frontier warp reference list subset
The decoder SHALL build the § 7.12.2 `WarpParamStack` under `DeriveWrl`
(corner-derived model, spatial warp-neighbour inserts from every scan
point, then warp-bank/gm/identity tail, four slots, no deduplication),
SHALL maintain the § 5.20.2.2 warp parameter bank (contents cleared per
superblock row, `WarpBankHits` and list-0 re-seeding per superblock,
unconditional § 5.20.7 per-block updates), SHALL derive WARPMV predicted
MVs by projecting `WarpParamStack[RefWarpIdx]` per § 7.12.2.2 and take
WARPMV/DELTAWARP parameter bases from the stack entry for any
`RefWarpIdx`, SHALL record § 7.13.3.20 `SubMvs` projections for warp
blocks and read them from the § 7.12.2.12 `get_mv` scan consumers, and
SHALL add the § 7.12.2.20 mixture candidates for blocks wider and
taller than 32 under the shared `PruneCount` budget.

#### Scenario: Pinned warp discriminators match AVM
- **GIVEN** the local decoder mission stream's coded frame 2
- **WHEN** the pinned WARPMV/DELTAWARP discriminator blocks parse
- **THEN** their warp parameters and motion vectors are value-identical
  to the AVM oracle, including a `RefWarpIdx > 0` selection

#### Scenario: Output frame 0 is unchanged
- **GIVEN** the local decoder mission stream decoded with a one-frame limit
- **WHEN** the display-order scheduler releases frames
- **THEN** output frame 0 is byte-identical to the avmdec raw output
