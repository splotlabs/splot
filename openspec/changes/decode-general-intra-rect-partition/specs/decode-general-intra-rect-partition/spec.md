## ADDED Requirements

### Requirement: General intra rectangular (non-square) partition decode
The decoder SHALL decode a rectangular (non-square, `n4w != n4h`) general intra
partition leaf — the PARTITION_HORZ / PARTITION_VERT family — gated to the
DC_PRED luma + DC chroma subset, on a 64x64 8-bit 4:2:0 intra key frame whose
64x64 superblock SPLITs via PARTITION_HORZ into two rectangular 64x32 DC_PRED
leaves. The leaf's § 7.13.2.4 DC prediction SHALL read its in-frame left column /
above row from the persistent frame workspace, so a non-first rectangular leaf
DC-predicts from its already-reconstructed neighbour in § 5.20.3.1 decode (DFS)
order. The § 5.20.7.27 `coeffs()` context spans SHALL read the transform width
(`Tx_Width[txSz] >> 2`) and height (`Tx_Height[txSz] >> 2`) independently, and
the § 7.14.4 / § 7.15.4 / § 7.14.3 reconstruction SHALL use the rectangular
transform dimensions (TX_64X32 luma, TX_32X16 chroma) including the § 7.15.4.1 √2
rescale for the odd log2 ratio. It SHALL validate § 8.2.4 `exit_symbol()` after
the whole tile. It SHALL admit only the verified 64x32 geometry, rejecting a
non-DC rectangular leaf (SMOOTH / directional luma or non-DC chroma) and any
rectangular geometry other than 64x32, each with a structured
`decode/unsupported-feature` diagnostic returned before any coefficient read or
sample write.
It SHALL NOT require the § 5.20.2.3 `BlockDecoded` flag state (the DC predictor
never reads the § 7.13.2.1 above-right / below-left sentinels), and SHALL NOT
handle non-64x64 frames, inter prediction, in-loop filters, or invoke AVM or
dav2d.

#### Scenario: Horizontal rectangular split decodes to the oracle
- **WHEN** `splot decode` is given the committed rectangular partition intra key
  frame `syn-hrect-intra-64x64-q120.ivf`
- **THEN** the general intra path walks the partition tree into two rectangular
  64x32 DC_PRED leaves, decoding and reconstructing each in decode order, and
  succeeds
- **AND** the reconstructed top band centre is 60 and the bottom band centre is
  200, with flat chroma U == V == 128, matching the avmdec and dav2d raw outputs
- **AND** the decoded-frame hash is the pinned
  `6d2e94d795d46cae62d1e2cf06cf4fe5b727b0917742745af998b002a7686142`

#### Scenario: Non-first rectangular leaf predicts from a reconstructed neighbour
- **WHEN** the bottom 64x32 leaf is decoded after the top 64x32 leaf is
  reconstructed in the workspace
- **THEN** its § 7.13.2.4 DC prediction reads the reconstructed above row of the
  top leaf rather than the no-neighbour `128` fallback

#### Scenario: Unverified rectangular leaves are rejected by construction
- **WHEN** `splot decode` reaches a rectangular leaf that codes a non-DC luma
  mode, a non-DC chroma mode (e.g. SMOOTH / cardinal), or any rectangular
  geometry other than the verified 64x32
- **THEN** the decoder rejects it with a structured `decode/unsupported-feature`
  diagnostic (`general_intra_rect_non_dc_luma`, `general_intra_rect_non_dc_chroma`,
  or `general_intra_rect_unverified_geometry`) rather than producing wrong output
- **AND** the reject is guarded by construction — returned before any coefficient
  read or sample write (no desync, no output), with dedicated negative
  conformance vectors deferred to a follow-on (task 3.4), not yet fixture-backed

#### Scenario: Existing general intra fixtures are unchanged
- **WHEN** `splot decode` is given the existing general intra fixtures
  (`syn-flat-intra-64x64-q80.ivf`, `syn-quad-intra-64x64-q80.ivf`, and the
  remaining committed vectors)
- **THEN** they decode bit-exactly to their pinned decoded-frame hashes
