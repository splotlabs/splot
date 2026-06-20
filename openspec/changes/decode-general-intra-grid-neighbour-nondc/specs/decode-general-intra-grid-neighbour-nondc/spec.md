## ADDED Requirements

### Requirement: General intra 2-D grid non-DC SMOOTH luma over a real reconstructed above row
The decoder SHALL reconstruct a full-superblock (`n4w == 16`) § 7.13.2.13
`SMOOTH_V_PRED` / `SMOOTH_H_PRED` luma block on the general intra path at ANY
superblock position in a 2-D grid, including a superblock at row > 0
(`frontier.r > 0`). For a row > 0 block the decoder SHALL build the § 7.13.2.1
edges with `haveAbove == 1` using the real reconstructed above row
`CurrFrame[0][y - 1][...]` (the bottom row of the already-decoded above
superblock), `haveLeft` per position, and the top-right sentinel `AboveRow[w]`
from the real reconstructed above-right sample when it is decoded
(`num4AboveRight > 0`, derived faithfully to AV2 § 5.20.7.25
`count_top_right_avail` over the § 5.20.2.3 `BlockDecoded` state) — the same
edge-building and above-right resolution the SMOOTH chroma 2-D grid path uses.
It SHALL then run the § 7.13.2.13 SMOOTH predictor (linear interpolation, no
`enable_intra_edge_filter` / IDIF / upsample edge synthesis) and add the
§ 5.20.7.27 residual. The decoder SHALL keep the § 8.3.2 `y_mode_index` context
derivation unchanged: SMOOTH_V/H are non-directional
(`modeDelta < NON_DIRECTIONAL_MODES_COUNT`) so the context stays 0 and they are
admitted, while a directional neighbour (`ctx != 0`) SHALL still be rejected with
a structured `decode/unsupported-feature` diagnostic before any symbol is read.
It SHALL reject — with a structured `decode/unsupported-feature` diagnostic — a
neighbour-having directional (D135) luma block, SMOOTH luma on a sub-partitioned
(non-full-superblock) block, multiple tiles, inter prediction, and in-loop
filters, and SHALL NOT invoke AVM or dav2d.

#### Scenario: A row > 0 SMOOTH_V luma superblock decodes to the oracle
- **WHEN** `splot decode` is given the committed 2-D grid intra key frame
  `syn-vgrid-intra-192x128-q120.ivf`, whose right two superblock columns code as
  `SMOOTH_V_PRED` luma
- **THEN** the general intra path iterates the six 64x64 superblocks, and the
  row > 0 SMOOTH_V luma superblock at the middle (non-rightmost) column reads the
  real reconstructed above row (and runs the above-right resolver over its decoded
  above-right neighbour), and succeeds
- **AND** the decoded output matches the avmdec and dav2d raw outputs
  byte-for-byte (md5 `136a87190eeecb1ccd32e7cf27861c9c`)
- **AND** the decoded-frame hash is the pinned
  `c62dd0eb74ab1129e9cd4d6a326cfef9026f62ab4144a378b38cb325b45462d2`

#### Scenario: The row > 0 block reads the real above row, not the fallback
- **WHEN** the bottom (row > 0) middle SMOOTH_V luma superblock is reconstructed
- **THEN** its top row continues the gradient from the bottom row of the above
  superblock (the § 7.13.2.1 real reconstructed above row), so the reconstructed
  samples straddle the superblock boundary monotonically rather than jumping toward
  the § 7.13.2.1 no-above flat fallback (127)

#### Scenario: A directional neighbour mode is still rejected
- **WHEN** a general intra block's § 8.3.2 `y_mode_index` context resolves to a
  non-zero value because an already-decoded neighbour stored a directional
  `IntraJointMode` (`>= NON_DIRECTIONAL_MODES_COUNT`)
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  before reading any `y_mode_set` / `y_mode_index` symbol, rather than decoding with
  an unverified CDF row

#### Scenario: Existing general intra fixtures still decode bit-exact
- **WHEN** `splot decode` is given the committed 64x64, 128x64, 64x128, and
  128x128 general intra fixtures
- **THEN** each reconstructs to its previously pinned decoded-frame hash, unchanged
  by lifting the first-superblock-row SMOOTH luma gate
