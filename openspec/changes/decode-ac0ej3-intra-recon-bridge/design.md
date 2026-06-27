# Design: ac0ej3 general-intra reconstruction bridge

## Context

Two general-intra decode routes exist in `crates/splot-decode/src/runtime_minimal/`:

1. `general_intra.rs` fully reconstructs synthetic intra fixtures: it walks the
   §5.20.3.1 partition tree, and for each leaf decodes §5.20.5.3 modes plus
   §5.20.7.27 coefficients into a `LumaCoeffBlock { all_zero, eob, quant }`, then
   calls `runtime_minimal_recon` primitives to predict → add residual → write into
   a `CurrentFrameWorkspace<T>`, then deblock/CDEF/freeze.
2. `wienerns_lr/tx_records.rs` is the ac0ej3 path. The selectable-transform
   handoff (`derive_wienerns_lr_selectable_transform_record_handoff`) walks the
   SAME partition tree via `decode_general_intra_multiblock_tree`, decodes each
   luma transform's coefficients with the SAME `decode_general_intra_plane_coeffs`
   (yielding a populated `LumaCoeffBlock`), but DISCARDS `quant` after recording
   `eob`/`skip_flag` into `WienerNsLrTxSkipTransformRecord`. Chroma coefficients
   are decoded and dropped (`let _v = ...`). The walk fails closed at the first
   active IntrABC block (`read_intrabc_info` returns the currframe-samples error).

The coefficients are therefore ALREADY decoded in the reusable `LumaCoeffBlock`
form the recon primitives consume; they are just thrown away. The bridge captures
them and reconstructs in place.

## ac0ej3 frame-0 facts (AVM inspect)

- 1920x1080, 10-bit (`T = u16`), 4:2:0, single tile, key frame.
- `superblockSize = BLOCK_128X128` (32x32 MI).
- `base_q_idx = 149`, `delta_q_present = 1`, `delta_q_res = 4`. The per-block
  dequant index is `DeltaQState.current_q_index` (per-superblock).
- First 128x128 superblock (MI cols 0..32, rows 0..32) is 100% DC_PRED and 100%
  NON-IntrABC. First active IntrABC is at MI (0, 56) in superblock column 1.
- First-SB block/transform layout: BLOCK_16X64 (TX_16X16 ×4 and TX_16X64 ×1),
  BLOCK_32X64 (TX_32X32 ×2), BLOCK_64X64 (TX_64X32 ×2), BLOCK_128X64 (TX_16X4).
  All DC_PRED. Chroma is one §6.4.1 4:2:0 transform per group.
- Oracle: `ac0_prefiltered.yuv` (intra + residual, BEFORE deblock/CDEF/LR), md5
  `f7959cb85a41dcf0e6ebf9179835da03`; first-SB top-left 8x8 luma is all `68`.

## Decision: reconstruct in the walk via an optional sink

`general_intra.rs` reconstructs immediately after decoding each block; the
wienerns_lr walk already decodes coefficients in the same place. So the bridge
threads an `Option<&mut WienerNsLrReconSink>` through the selectable residual-chunk
decoders and reconstructs each block where its `LumaCoeffBlock` is produced. This
keeps prediction reading already-reconstructed neighbours (decode order == raster
order for the first SB), with zero re-walk and no re-decode.

### `WienerNsLrReconSink` (`wienerns_lr/recon.rs`)

Owns:
- `workspace: CurrentFrameWorkspace<u16>` sized to the ac0ej3 frame (via the
  existing `new_general_intra_workspace::<u16>`).
- bookkeeping for which MI regions were reconstructed vs deferred.

Methods:
- `reconstruct_luma_dc(record, &LumaCoeffBlock, qindex, use_tcq)` — maps
  `record.tx_size` to `(TX_WIDTH_LOG2, TX_HEIGHT_LOG2)` and the MI `col/row` to
  sample coords, then calls `reconstruct_general_intra_block_rect_into(PlaneId::Y,
  …)`. Only invoked for DC_PRED luma blocks; other modes are skipped (the region
  is left unreconstructed, recorded as deferred).
- `reconstruct_chroma_dc(plane, chroma_tx, x, y, &LumaCoeffBlock, qindex)` — calls
  `reconstruct_general_intra_block_rect_into(PlaneId::U/V, …)` with the chroma
  transform log2 dims (DC chroma uses no §7.14.4 TCQ term, so `use_tcq = false`).

The sink is gated to DC_PRED (the modes the first SB uses). The luma leaf mode
flows from the prelude / `decode_general_intra_block_modes`; a non-DC leaf marks
the block deferred and the sink does not write it.

### Public decode stays fail-closed

The sink is owned by the test harness, not the public decode. The public
selectable walk runs WITHOUT a sink (or with a sink whose workspace is discarded),
so it still fails closed at the first IntrABC block and emits no frame. The CLI
probe test continues to assert exit code 1 with the IntrABC diagnostic.

### Verification (region-based test, no 6 MB oracle committed)

A new test:
1. Decodes the local ac0ej3 fixture with a sink attached, driving the selectable
   walk until it fails at IntrABC (the workspace retains everything reconstructed
   so far — the whole first SB).
2. Reads the first-superblock luma + chroma region from the workspace.
3. Asserts that region equals a committed assertion: a SHA-256 of the verified
   first-SB sample region (derived offline from `ac0_prefiltered.yuv`), plus a
   spot check that the top-left 8x8 luma is all `68`.

The test is gated to the local mission fixture (`SPLOT_AC0EJ3_IVF` / `#[ignore]`),
matching the existing `local_ac0ej3_*` probe convention. The committed assertion
is a small hash + spot value, never the 6 MB YUV.

## Reuse and laziness

- No new prediction/residual/transform code: the bridge calls
  `reconstruct_general_intra_block_rect_into`, which already does §7.13.2 DC
  prediction over workspace neighbours + §7.14.4/§7.15.4/§7.14.3 residual for any
  plane and rectangular geometry.
- No re-walk, no second coefficient decode: the existing walk's `LumaCoeffBlock`
  is captured at its decode site.
- `dupehound`: the chunk/group reconstruction call shape mirrors the
  `general_intra.rs` dispatch; the bridge factors the shared rect-DC call into one
  sink method to avoid structural duplication.

## How this generalizes to the remaining bricks

- B2 (more modes): the sink's DC gate widens to the other
  `runtime_minimal_recon` primitives (V/H/PAETH/directional), which already exist
  and are mode-dispatched exactly as in `general_intra.rs`.
- B3/B4 (deblock + CDEF): after the full intra region reconstructs, the same
  `super::deblock` / `super::cdef` passes `general_intra.rs` already runs over a
  `CurrentFrameWorkspace` apply to the sink's workspace.
- B5 (IntrABC): replaces the fail-closed IntrABC branch with
  `RECON-INTRABC-CURRENT-FRAME-COPY` reading the sink's now-populated workspace.
- B6 (LR): the §7.20 loop-restoration source reads (already built) consume the
  sink's reconstructed CurrFrame/CdefFrame.

The sink is the shared current-frame buffer all later bricks read and write.

## Milestone reached (parse fixed by PR #497)

The original bridge brick discovered a first-superblock parse desync (the top-left
block derived a 2x TX_16X32 partition where AVM uses 4x TX_16X16, mis-aligning the
§5.20.7.27 `txb_skip` reads so the top-left DC transform decoded `all_zero` where
AVM's oracle has a residual). PR #497 added the missing per-block §5.20.10.2 CCSO
read (`read_ccso`, between `read_cdef` and `read_delta_qindex`), which resolved that
desync: ac0ej3's first superblock now parses bit-exact vs the AVM analyzer.

Fed the now-correct parse, the bridge reconstructs the verified DC subset to the
spec-correct samples. Verified against the `ac0_prefiltered.yuv` oracle:

- The frame-origin §5.20.5.3 `DC_PRED` 16x16 luma leaf reconstructs BIT-EXACT (all
  `68`) — the first bit-exact ac0ej3 reconstruction milestone.
- The bridge reconstructs every `DC_PRED` luma transform it reaches before the
  §7.13.3.18 IntrABC fail-closed rejection: 4096 frame-wide luma samples, all but
  one 16x16 block bit-exact (that block carries a small AC residual the DC-flat
  primitive drops).

Remaining first-superblock samples are OUTSIDE the verified DC subset and stay
UNRECONSTRUCTED (not blocked-on-parse): the first-SB chroma leaf is `SMOOTH` (not
`DC`), most first-SB luma is SMOOTH/directional, and one `DC_PRED` leaf carries a
small AC residual. Full-superblock bit-exact needs SMOOTH/directional/CfL
prediction and full residual inverse-transform reconstruction (follow-on rows),
then chroma and IntrABC reconstruction. The sink never writes a sample it has not
proven bit-exact, and the public decode stays fail-closed at the IntrABC wall.

## Risks / open questions

- Per-block dequant index under delta-Q: the bridge uses
  `DeltaQState.current_q_index`. Verified correct for the bit-exact DC region.
- Multi-transform DC neighbour reads within a block rely on the workspace holding
  the previous transform's reconstructed samples, which the in-walk decode order
  guarantees.
