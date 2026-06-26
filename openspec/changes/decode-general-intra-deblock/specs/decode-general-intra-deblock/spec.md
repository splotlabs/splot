## ADDED Requirements

### Requirement: General intra § 7.17 deblocking-filter orchestration
The decoder SHALL apply the AV2 § 7.17 deblocking filter in place over the
reconstructed general intra frame, after the block walk and before the frame is
frozen, for the verified subset: an 8-bit 4:2:0 intra key frame with
`df_delta_q` all zero, a single tile, segmentation disabled, and the other
in-loop filters (GDF, CDEF, CCSO, loop restoration) disabled. The orchestration
SHALL derive the per-(plane, pass) § 7.17.6 filter level (`lvl =
q_clamped(qindex2, delta)` with `delta` resolving to zero and `df_delta_q` all
zero, so `lvl = q_clamped(base_q_idx, 0)`), the § 7.17.5 `(qThr, side)`
strengths over the § 9.2 `Side_Thresholds` table, then iterate the § 7.17.1 /
§ 7.17.2 plane × pass × MI edge loop gated on
`apply_deblocking_filter[plane == 0 ? pass : plane + 1]` (with 4:2:0 chroma
`rowStep` / `colStep`), and for each edge run the § 7.17.2 derivation
(`sbEdge` / `onScreen` / `isBlockEdge` / `isTxEdge` / `applyFilter`, with
`isSubPuEdge == 0` for the intra key frame and `skip == 0` for the residual-coded
blocks), the § 7.17.4 screen-edge-clipped `filterSize`, the § 7.17.3
`maxWidthNeg` / `maxWidthPos`, the § 7.17.7.2 filter choice over the gathered
perpendicular `s` / `t` sample lines, and the § 7.17.7.1 sample filter for each
edge position. A deblock-off frame (`apply_deblocking_filter == [false; 4]`)
SHALL run the pass as a no-op so the reconstruction is byte-identical.

This requirement SHALL NOT claim a nonzero `df_delta_q` deblock-active frame, a
10-bit deblock-active frame, `allow_df_sub_pu` sub-PU edges, segmentation /
lossless segments, multiple tiles, the other in-loop filters, inter frames, or
any AVM / dav2d invocation. A deblock-active frame outside this admitted subset
SHALL be rejected with a structured `decode/unsupported-feature` diagnostic
before any caller-visible output. The 8-bit deblock-off and 10-bit reconstruction
paths SHALL remain byte-identical.

#### Scenario: deblock-active intra frame decodes to the oracle
- **WHEN** `splot decode` is given the committed deblock-active intra key frame
  `syn-2sb-deblock-intra-128x64-q100.ivf`
  (`apply_deblocking_filter == [false, true, true, true]`,
  `df_delta_q == [0, 0, 0, 0]`, base_q_idx 100, two 64x64 superblocks split into
  four 32x32 DC_PRED luma + DC chroma blocks each)
- **THEN** the general intra path reconstructs the frame, applies the § 7.17
  deblocking pass in place, and succeeds
- **AND** the `--output-format raw` bytes equal the avmdec and dav2d raw outputs
  exactly (raw md5 `ca302adc8641007251c5947b3d5c73ba`)
- **AND** the deblock effect is the luma horizontal pass at the y=32
  transform/block boundary (2 luma samples change), while the chroma passes leave
  the flat chroma planes unchanged

#### Scenario: larger-effect deblock-active fixtures decode to the oracle
- **WHEN** `splot decode` is given the committed
  `syn-2sb-deblock-intra-128x64-q98.ivf` (24 luma samples change) or
  `syn-2sb-deblockwide-intra-128x64-q100.ivf` (38 luma samples change)
- **THEN** each reconstructs bit-exactly to the avmdec and dav2d raw outputs
  (raw md5 `5cb616189cf501809d6986a9ef1058a7` and
  `82d8be50486d46fdd771e73a214bf5c2` respectively)

#### Scenario: multi-superblock-row deblock-active fixture exercises the y=64 sbEdge
- **WHEN** `splot decode` is given the committed
  `syn-grid-deblock-intra-128x128-q100.ivf` (128x128, a 2x2 grid of 64x64
  superblocks, `apply_deblocking_filter == [false, true, true, true]`)
- **THEN** it reconstructs bit-exactly to the avmdec and dav2d raw outputs (raw
  md5 `1e4675e63da02a22431390e293e4c0ba`), exercising the luma-horizontal y=64
  64-sample-grid `sbEdge` iteration (64 `sbEdge` edges) that the 64-tall 128x64
  fixtures cannot reach

#### Scenario: luma-vertical, y=64 sbEdge, and chroma paths are unit-pinned
- **WHEN** the orchestration runs with a forced `apply` pattern over a synthetic
  workspace carrying a clean step at the x=64 (luma vertical), y=64 (luma
  horizontal `sbEdge`), or chroma x=32 (= luma x=64) boundary
- **THEN** the luma-vertical pass smooths the x=64 edge, the luma-horizontal pass
  smooths the y=64 `sbEdge` (with its § 7.17.3 negative-side max-width cap
  bounding the upward extent), and the chroma pass smooths the chroma boundary
  while leaving the unenabled plane flat — flat interiors and non-edge positions
  stay unchanged, deterministically pinning the luma-vertical / y=64-`sbEdge` /
  chroma paths that are admitted and reachable but never sample-changing in an
  avmenc-producible DC-multi-block oracle fixture

#### Scenario: deblock-off frame stays byte-identical
- **WHEN** the existing general intra fixtures whose
  `apply_deblocking_filter == [false; 4]` are decoded after the deblock pass is
  added
- **THEN** each reconstructs to the same bytes as before, because the § 7.17 pass
  is a no-op when no pass is enabled

#### Scenario: nonzero df_delta_q and 10-bit deblock-active reject
- **WHEN** a deblock-active frame has a nonzero `df_delta_q[i]`, or is 10-bit
- **THEN** the decoder rejects it before any caller-visible output with a
  structured `decode/unsupported-feature` diagnostic, because no oracle fixture
  pins the shifted filter level or the 10-bit deblock pass
