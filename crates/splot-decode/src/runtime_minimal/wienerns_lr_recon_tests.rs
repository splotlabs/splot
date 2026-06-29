// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Region-verification tests for the ac0ej3 general-intra reconstruction bridge.
//!
//! Drives the real ac0ej3 mission stream through the `TX_MODE_SELECT` selectable
//! transform-record walk with a reconstruction sink attached, and verifies the
//! reconstructed NON-IntrABC DC region BIT-EXACT against the AVM pre-filter
//! reconstruction oracle (`ac0_prefiltered.yuv`,
//! md5 `f7959cb85a41dcf0e6ebf9179835da03`).
//!
//! With the per-block CCSO read the selectable walk parses ac0ej3's
//! first superblock bit-exact vs AVM, so the bridge reconstructs the verified DC
//! subset to the spec-correct samples. The first 16x16 luma block (the §5.20.5.3
//! `DC_PRED` leaf at the frame origin) reconstructs BIT-EXACT (all `68`); the
//! committed constants below are the small oracle assertion derived offline from
//! `ac0_prefiltered.yuv`.
//!
//! Reconstructed-and-verified region for frame-0 (gated to the proven DC subset):
//!   * Luma: the full first-3-superblock DC region — the rectangle x[0,192) x
//!     y[0,128), 24576 samples — is bit-exact. Fixing the MI(4,0) `TX_16X64`
//!     keystone (the §7.13.2.12 IBP DC modifier plus the non-square `TX_16X64`
//!     residual) unblocked the whole DC chain that bordered it through the §7.13.2
//!     edge-coverage guard, widening the region 24x from the original 1024-sample
//!     `BLOCK_16X64` column. Every sample is the down-predicted flat `DC_PRED`
//!     value `64` except the origin 16x16 leaf (`68`, 256 samples) and the MI(4,0)
//!     IBP DC step (`65`, the top-left 3 columns x 16 rows == 48 samples). Whole
//!     region strictly bit-exact, no confident-wrong workspace samples.
//!   * Chroma: the frame-origin `DC_PRED` 32x32 U and V transforms (the §5.20.3.1
//!     SDP chroma tree at chroma `(0,0)`) — both flat `512`, the 10-bit no-neighbour
//!     DC fallback — are bit-exact (2048 chroma samples). The U/V origin reads only
//!     its own off-frame edges (chroma `DC_PRED`, not CfL).
//!   * Everything the primitive cannot prove bit-exact is DEFERRED: NON-DC luma
//!     (SMOOTH / directional); NON-DC chroma (the SMOOTH chroma leaf at chroma
//!     `(32,0)`); any IST / FSC leaf; a frame with a non-zero quantizer delta or
//!     matrix; and any block whose §7.13.2 prediction edges border a deferred
//!     (un-reconstructed) neighbour. The non-square (`TX_16X64`) `DC_PRED` residual
//!     and the §7.13.2.12 IBP DC modifier are now MODELLED and proven bit-exact at
//!     the MI(4,0) keystone, so they no longer wall the DC chain. The sink never
//!     claims a sample it has not proven bit-exact.
//!
//! Parse fidelity (verified against the AVM mode/uv_mode oracle): every luma leaf splot resolves in the reachable region agrees with
//! AVM's `inspect --mode`, and the chroma origin resolves to `DC_PRED` matching
//! `inspect --uv_mode`. The reachable region is now bounded by the §7.13.3.18
//! IntrABC fail-closed stop, not by a reconstruction-primitive wall.
//!
//! The oracle YUV is 6 MB and is NOT committed; the committed assertions are the
//! region flat values (`68` / `65` / `64` luma, `512` chroma), their sample sums,
//! and FNV-1a-64 checksums. The PUBLIC decode stays fail-closed; these tests
//! exercise the bridge through a test-only sink driver, gated to the local mission
//! fixture (`SPLOT_AC0EJ3_IVF` or `$HOME/Documents/SplotLabs/ac0ej3.ivf`) and
//! `#[ignore]`d to match the existing `local_ac0ej3_*` probe convention.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use splot_parallel::ThreadCount;
use splot_recon::PlaneId;

use crate::{DecodeContext, DecodeOptions, DecodeRuntimeConfig};

use super::reconstruct_ac0ej3_intra_region_from_plan;
use super::reconstruct_ac0ej3_intra_region_from_plan_with_mode;
use super::wienerns_lr::WienerNsLrReconSink;

/// Frame-origin `DC_PRED` luma leaf side (a 16x16 §5.20.6 `TxSize` transform).
const BLOCK0_SIDE: usize = 16;

/// Committed oracle assertion for the frame-origin 16x16 `DC_PRED` luma block,
/// derived offline from the AVM pre-filter reconstruction `ac0_prefiltered.yuv`
/// (its first 16x16 luma samples; sample-major u16 little-endian). The block is a
/// flat `DC_PRED` leaf, so every sample is `68`.
const BLOCK0_FLAT_LUMA: u16 = 68;
const BLOCK0_SAMPLE_COUNT: usize = BLOCK0_SIDE * BLOCK0_SIDE;
const BLOCK0_SAMPLE_SUM: u64 = 17_408;
const BLOCK0_FNV1A64: u64 = 0x68b9_9236_1d60_fb25;

/// The full `BLOCK_16X64` left luma column the sink reconstructs (one 16-wide
/// transform column, `16` samples across and the full superblock height of `64`
/// down). The §5.20.5.3 `DC_PRED` origin leaf is the flat `68` block; the
/// down-predicted `DC_PRED` transforms below it are the flat oracle value `64`.
const LUMA_COLUMN_WIDTH: usize = 16;
const LUMA_COLUMN_HEIGHT: usize = 64;
/// Oracle value for the `DC_PRED` transforms below the origin leaf (rows 16..64),
/// derived offline from `ac0_prefiltered.yuv`.
const LUMA_COLUMN_BELOW_ORIGIN: u16 = 64;
const LUMA_COLUMN_SAMPLE_COUNT: usize = LUMA_COLUMN_WIDTH * LUMA_COLUMN_HEIGHT;
/// Sum of the full 16x64 column (`256 * 68 + 768 * 64`).
const LUMA_COLUMN_SAMPLE_SUM: u64 = 66_560;
/// FNV-1a-64 over the full 16x64 column (row-major, sample-major u16 LE), matching
/// the offline oracle checksum derivation.
const LUMA_COLUMN_FNV1A64: u64 = 0x893d_3114_b40a_7325;

/// The full first-3-superblock luma DC region the sink now reconstructs: the
/// rectangle x[0,192) (three 64-wide superblock columns) x y[0,128) (two
/// superblock rows), 24576 samples. Fixing the MI(4,0) `TX_16X64` keystone (the
/// §7.13.2.12 IBP DC modifier + the non-square `TX_16X64` residual) unblocks the
/// whole DC chain that bordered it through the §7.13.2 edge-coverage guard, so the
/// verified region widens 24x from the original 1024-sample column. Every sample is
/// the down-predicted flat `DC_PRED` value `64` except the origin 16x16 leaf
/// (`68`, 256 samples) and the MI(4,0) IBP DC step (`65`, the top-left 3 columns x
/// 16 rows == 48 samples). Derived offline from `ac0_prefiltered.yuv`.
const LUMA_REGION_WIDTH: usize = 192;
const LUMA_REGION_HEIGHT: usize = 128;
const LUMA_REGION_SAMPLE_COUNT: usize = LUMA_REGION_WIDTH * LUMA_REGION_HEIGHT;
/// Sum of the 192x128 region (`256 * 68 + 48 * 65 + 24272 * 64`).
const LUMA_REGION_SAMPLE_SUM: u64 = 1_573_936;
/// FNV-1a-64 over the 192x128 region (row-major, sample-major u16 LE).
const LUMA_REGION_FNV1A64: u64 = 0x31c1_4055_9bd3_8725;
/// The §7.13.2.12 IBP DC value at the MI(4,0) `TX_16X64` leaf's top-left 3 columns.
const MI40_IBP_STEP: u16 = 65;

/// The total reconstructed luma sample count after the §7.12.2.19 IntrABC ref-MV
/// weight-sort advancement. The prior §7.12.2.6 above-row stage reconstructed
/// `204800` samples (it stopped at the MI(192,112) §7.12.2.19 multi-candidate
/// weight-sort defer); modelling the §7.12.2.19 max-weight-to-slot-0 reorder (with
/// the §7.12.2.6 per-candidate weights) admits the `BLOCK_64X32` MI(192,112) block
/// — which has TWO distinct spatial candidates ((-1024,0) step 7 weight 2 +
/// (-512,0) step 8 weight 1) so the sort runs (a no-op swap; slot 0 keeps the
/// max-weight (-1024,0), drl=1 selects (-512,0), bit-exact vs avmdec) — and its
/// downstream IntrABC siblings faithfully. Each admitted block keeps the entropy
/// parse synced in decode order, so the walk reconstructs many more proven-subset
/// general-intra DC / cardinal leaves before the next defer.
///
/// The verified region is `245760` bit-exact luma samples (bounding box x[0,447]
/// y[0,1023], a non-rectangular union of the covered MI units). After the §5.20.4.1
/// SDP chroma-reference MI-size fix (which removed the MI(240,240) §8.3.2 `do_split`
/// left-context desync and the downstream `bitstream_desync` over-read) and the
/// §5.20.7.27 coefficient context-write edge clamp (modelling AVM
/// `av2_set_entropy_contexts`, which advanced the walk past the bottom-edge skipped
/// TX_64X64 transforms whose 16-tall left span overhangs the tile by 2 MI rows), the
/// walk reconstructed `233472` samples and stopped at the §5.20.6.1 selectable
/// transform-record `recon_luma_write` frontier MI(64,0) — a `TX_64X32` (non-square)
/// `V_PRED` `all_zero` leaf at the FRAME TOP (`mi_row == 0`, so `haveAbove == 0`).
/// Modelling the §7.13.2.1 single-neighbour edge fallback (`haveAbove == 0 &&
/// haveLeft == 1`: `AboveRow[i] = CurrFrame[plane][y][x-1]`, the block's left
/// neighbour repeated across the synthesized above row) lets the §7.13.2.8 V_PRED
/// copy reconstruct the flat `68` block, which CASCADES: its now-covered samples
/// become valid left/above neighbours for the rest of the top-row SB columns 4-6,
/// adding the solid rectangle x[256,448) y[0,64) (`12288` samples: `11264` of `68`
/// plus a `32x32` patch of `64` at x[352,384) y[0,32)).
///
/// Modelling the §7.13.2.2 PAETH (`PAETH_PRED`) predictor for the two-sided
/// `haveAbove == 1 && haveLeft == 1` config — the §7.13.2.1 above row, left column,
/// AND the real reconstructed corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` —
/// then admits the two `all_zero` `TX_64X64` PAETH leaves MI(64,16) (x[256,320)
/// y[64,128)) and MI(80,16) (x[320,384) y[64,128)) and the DC / cardinal leaves they
/// CASCADE into (their now-covered samples become valid neighbours), adding `14848`
/// bit-exact samples (`245760` → `260608`). The single-sided PAETH configs
/// (`haveAbove ^ haveLeft`, e.g. MI(56,248)) DEFER: the oracle shows PAETH does not
/// match the naive §7.13.2.1 single-neighbour fallback there, so they await their own
/// verified model. The walk then stops at the GENUINELY DISTINCT next mechanism.
///
/// With the §7.11.3 reconstructed-sample WORKSPACE WRITE now clamped to the frame
/// edge (modelling AVM's in-frame-only reconstruction: a transform overhanging a
/// partial-superblock frame edge writes only the in-frame rows/cols and drops the
/// overhang — see [`CurrentFramePlane::clamp_rect_to_storage`] in `splot-recon`), the
/// `TX_64X64 DC_PRED all_zero` leaf at MI(0,256) (sample x[0,64) y[1024,1088)) — whose
/// 64-tall write overhangs the 1080-tall luma storage by 8 rows — now writes its 56
/// IN-FRAME rows (y[1024,1080), `3584` samples of the flat `64`) instead of erroring
/// `WorkspaceRectOutOfBounds`. The §7.13.2.1 frame-edge EDGE-EXTENSION (a transform
/// overhanging the frame bottom/right edge-extends its clamped in-frame left column /
/// above row back to the block's full nominal height/width by replicating the LAST
/// in-frame sample, per AVM `av2/common/reconintra.c:1191-1195`) lets the bottom-edge
/// `TX_64X64 DC_PRED` block at MI(16,256) (sample x[64,128) y[1024,1080)) — whose
/// 56-row in-frame left column previously errored `IntraPredictionEdgeLengthMismatch`
/// (`expected:64, actual:56`) in the §7.13.2 DC primitive — reconstruct its 56 in-frame
/// rows (`3584` flat-`64` samples) bit-exact, advancing the region `264192` → `267776`.
///
/// The §5.20.6.1 IntrABC `record_block` mode-info fill is now ALSO clamped to the
/// frame edge (modelling AVM §5.20.3.2 `block_coded(r,c) { r < MiRows && c < MiCols }`,
/// 05-syntax-structures.md:9621): a non-IntrABC `BLOCK_128X64` leaf at MI(256,0) whose
/// nominal 16-tall MI footprint overhangs the 270-row MI grid by 2 MI rows (8 luma
/// rows) records only its 14 in-frame MI rows instead of erroring
/// `..._intrabc_block_bounds`. The walk advances past that former parse/recon frontier
/// and the bottom partial-SB row's in-frame samples (MI rows 256..269, y[1024,1080))
/// now reconstruct, growing the region `267776` → `273152` (+5376).
///
/// The §6.19.7.12 IntrABC PREDICTION-GEOMETRY target is now ALSO clamped to the visible
/// region (the same §5.20.3.2 `block_coded` model, one pipeline stage later): the
/// bottom-edge `BLOCK_16X64` IntrABC block at MI(256,56) — whose nominal 64-tall target
/// overhangs the 1080-row luma frame by 8 rows — derives an EFFECTIVE 16x56 in-frame
/// target (congruent 16x56 source) instead of erroring `intrabc_target_bounds`, so the
/// parse advances past that former frontier. That block's own reconstruction still
/// DEFERS (it stays at the fill value, so the region count is UNCHANGED at `273152`):
/// its real DV `(row=-1024, col=0)` is a -128px VERTICAL displacement whose source sits
/// in the PREVIOUS superblock row, which AVM validates only via the
/// `allow_global_intrabc` path (an unmodeled DV class), so `intrabc_dv_proven_valid`
/// conservatively defers the copy — never a confident-wrong sample. The §5.20
/// `reset_block_context` write and the §5.20.6.1 PC-Wiener `LrTxSkip` FilterClass grid
/// retention are now ALSO clamped to the frame edge (the same §5.20.3.2 `block_coded`
/// model): the bottom-edge skipped transforms at MI(256,0) — whose nominal 16-tall MI
/// footprint overhangs the 270-row MI grid by 2 — zero / fill only their on-frame
/// cells instead of erroring `skipped_context_reset` / `LrTxSkip transform record
/// bounds`. With those clamps the recon-sink handoff now runs to COMPLETION (the
/// verified subset reconstructs; out-of-subset blocks defer to their fill value), and
/// the parse-only public path advances to the §7.20.4
/// `live_frame_samples_unpopulated` gate. That milestone left the verified region at
/// `273152` (the skipped edge blocks were already covered/deferred).
///
/// The §7.15.4 primary inverse transform now reconstructs with the REAL retained
/// `PlaneTxType` instead of a hardcoded `DCT_DCT`: `LumaCoeffBlock` carries the
/// already-decoded `metadata.luma_tx_type`, the inverse resolves the actual
/// `Transform_1d_Type[PlaneTxType]` row/col kernels, and the former non-square /
/// cardinal `eob > 1` tx-type defers are gone. This unblocked the MI(112,16) H_PRED
/// `TX_16X16` `eob == 6` leaf (and its cascade), growing the region to `299264`.
///
/// The §7.13.3.18 GLOBAL IntrABC wavefront DV-validity branch is now WIRED into the
/// live displaced-copy admission ([`intrabc_dv_proven_valid`]): the §6.19.7.12
/// `av2_is_dv_valid` local-IBC same-SB subset is tried first, then — on an intra-only
/// frame with an explicitly-read `allow_global_intrabc` — the modelled global
/// wavefront branch admits a source in the already-coded top-left wavefront region.
/// With the §5.20.5.5 y-mode neighbour-reorder gate fixed, every
/// global-IntrABC displaced copy and the regular-intra cascade it re-ignites is
/// per-sample bit-exact, growing the region `299264` → `670976`. The blocks the
/// global branch does NOT prove (e.g. the still-deferred MI(36,224) V_PRED leaf, no
/// longer mis-parsed but not yet reconstructable through this sink) stay UNCOVERED at
/// their fill value — the sink never claims a sample it has not proven bit-exact.
///
/// The §7.13.3.18 NON-skip integer-DV IntrABC RESIDUAL leaves are now reconstructed:
/// the displaced copy lands as the §7.13.2 prediction, then each §5.20.7.27 residual
/// transform leaf adds its decoded residual onto the copied predictor (the §7.14.4 /
/// §7.15.4 / §7.14.3 dequant → inverse → Clip1-add path the DC / cardinal intra leaves
/// already use, over the IntrABC predictor instead of an intra prediction). Admitted
/// only for an integer DV, a fully-reconstructed source, no real §5.20.7.29 IST
/// (`sec_tx_type == 0`), and a reconstructable residual; a fractional DV, a real IST,
/// an uncovered source, or chroma still DEFER. This grew the region `670976` →
/// `743456` (the 13 reachable non-skip integer-DV IntrABC blocks plus the regular-intra
/// + IntrABC cascade their reconstructed targets re-ignite through the coverage guards).
///
/// Verified ZERO-mismatch, per sample, over EVERY covered luma sample against the AVM
/// pre-filter reconstruction oracle (the `inspect --dump-prefiltered` luma plane,
/// 1920x1080 u16-LE, stride 3840), aggregated by count + sum + FNV-1a-64 in
/// [`LUMA_RECON_REGION_SAMPLE_SUM`] / [`LUMA_RECON_REGION_FNV1A64`].
///
/// Modelling the §7.13.2.8 ONE-SIDED IDIF luma predictor (zone-1 `pAngle < 90` reads
/// the above row + above-right; zone-3 `pAngle > 180` reads the left column +
/// below-left) over the proven no-edge-filter subset grew the region `743456` →
/// `743520` (+64, the zone-1 `TX_8X8` leaf at MI(248,28), `pAngle 81` from
/// `Mode_To_Angle[D67] + AngleDeltaY(-3) * ANGLE_STEP`).
///
/// Wiring the §7.13.2.18 intra edge filter + §7.13.2.14 corner filter into the
/// one-sided IDIF reconstructors then admitted the corner-filter + edge-filter-active
/// one-sided sub-class (`743520` → `743776`, +256, the zone-1 16x16 `D45`-seed leaf at
/// MI(148,168), `pAngle 58`, `strength 3`, corner active). The §7.13.2.15/16 per-edge
/// `filterType` is derived from the REAL decoded neighbour `is_smooth` modes recorded
/// in the coverage map; the §7.13.2.17 strength + §7.13.2.7 `numPx` drive the
/// `av2_filter_intra_edge` sweep, and the §7.13.2.14 corner blend
/// (`needAbove && needLeft && (w + h) >= 24`) rewrites the shared corner from the
/// reconstructed opposite-edge `[0]` sample. A leaf is admitted only when `useIBP == 0`
/// (`applyIbp && EVEN AngleDeltaY` DEFERS — the §7.13.2.9 IBP secondary blend is
/// unmodelled), `MrlIndex == 0`, square, the read edge + above-right/below-left are
/// reconstructed, AND (when the corner fires) the opposite-edge `[0]` sample is
/// reconstructed — otherwise the leaf DEFERS rather than reading a fill value (so the
/// prompt's named MI(232,28) seed DEFERS until its left neighbour MI(231,28) is
/// reconstructed by the decode-order cascade). The `useIBP` / `MrlIndex > 0` one-sided
/// leaves still DEFER.
///
/// Generalising the one-sided IDIF reconstructors + edge builders to NON-SQUARE
/// transforms (`log2_width != log2_height`) then admitted the non-square one-sided
/// sub-class (`743776` → `743904`, +128). §7.13.2.8 is non-square-aware: `maxBase ==
/// w + h - 1`, the §5.20.7.29 wide-angle remap's tall-block (`h == k*w`) / wide-block
/// (`w == k*h`) wrap branches now fire (verified VERBATIM vs AVM `wide_angle_mapping`,
/// `reconintra.h`), the §7.13.2.1 `aboveLimit` keys on `w` (above-right capped at
/// `tx_size_wide_unit == mi_w`), and `leftLimit` keys on `h` (below-left capped at
/// `tx_size_high_unit == mi_h`). When the §7.13.2.18 edge filter is active it consumes
/// the full padded `mi_w`/`mi_h` above-right/below-left span, so the coverage guard
/// requires that whole span covered to match AVM's pad boundary (`has_top_right` /
/// `has_bottom_left`); a no-op filter only needs the projection's `max_read` reads.
/// The `useIBP` / `MrlIndex > 0` / zone-2 one-sided leaves still DEFER.
///
/// Relaxing the §7.13.2.1 single-neighbour cardinal fallback gate to the
/// origin-adjacent orthogonal sample (the AVM `(!need_left && n_top_px == 0)` /
/// `(!need_above && n_left_px == 0)` fast path that fills the whole block with
/// `left_ref[0]`/`above_ref[0]`, `reconintra.c:1150-1163`) admitted the PARTIAL
/// cardinal-fallback sub-class (`743904` → `772576`, +28672). The seed is the
/// `TX_16X8 V_PRED` leaf MI(272,0), x[1088,1152) y[0,32): `mi_row == 0` so
/// `haveAbove == 0`, and the left neighbour column at MI col 271 is only PARTIALLY
/// reconstructed (rows 0-1 covered by an earlier IntrABC copy, rows 2-7 deferred).
/// V_PRED reads ONLY `left_ref[0] = CurrFrame[0][1087]` (`= 68`), so the block is
/// flat `68` and the deferred deeper rows are never read — the old full-edge gate
/// deferred it spuriously. Admitting it cascades into its downstream
/// decode-order neighbours. Zero mismatch vs the AVM prefilter oracle over the
/// whole grown region. The NO-neighbour midpoint fallback (both edges off-grid,
/// e.g. the frame origin) still DEFERS.
///
/// Admitting the §7.13.2.2 PAETH RESIDUAL leaves (dropping the old `all_zero` gate
/// — the PAETH predictor reads the same real reconstructed above row / left column /
/// corner, then the §5.20.7.27 residual is ADDED via the standard §7.14.3
/// `Clip1(pred + inverse-transform(residual))`, exactly as the directional paths do)
/// re-ignited the decode-order cascade: the region grew `772576` → `775904` (+3328).
/// A residual-bearing PAETH leaf reconstructs in decode order once its
/// `haveAbove && haveLeft` neighbours (the above row, left column, AND diagonal
/// corner unit) are covered; admitting them unblocks their downstream neighbours.
/// Zero mismatch vs the AVM prefilter oracle over the whole grown region (verified
/// per sample against `/tmp/pref.yuv` frame-0, md5 `f7959cb8…`). PAETH leaves whose
/// neighbours are still deferred, and `mrl_index > 0` PAETH, still DEFER.
///
/// Wiring the §7.13.2.8 ZONE-2 (middle, `90 < pAngle < 180`) two-sided IDIF
/// predictor — the generalized middle primitive over the in-block above row + left
/// column + shared corner, with the §7.13.2.18 edge filter on BOTH edges and the
/// §7.13.2.14 corner blend — grew the region `775904` → `791776` (+15872). The same
/// commit fixed a latent §7.13.2.1 zone-3 (left-reading) corner bug the zone-2
/// cascade EXPOSED: a `haveAbove == 1` interior zone-3 leaf must read the corner
/// `LeftCol[-1]` from the DIAGONAL above-left `CurrFrame[y - 1][x - 1]`, not the
/// left-column top `CurrFrame[y][x - 1]` (the prior code, correct only for the
/// frame-top `haveAbove == 0` leaves that were previously reachable). With both, the
/// whole grown region is bit-exact (the off-by-one at MI(64,240)/(256,960) — the
/// D203+`AngleDeltaY` zone-3 leaf unblocked by a zone-2 neighbour — is resolved).
/// Zone-2 leaves whose above row / left column / corner are still deferred DEFER.
const LUMA_RECON_SAMPLE_TOTAL: usize = 791_776;
/// Sum of every reconstructed luma sample in the verified region (derived from the AVM
/// pre-filter oracle over the sink's covered MI units, zero mismatch vs splot).
const LUMA_RECON_REGION_SAMPLE_SUM: u64 = 53_847_203;
/// FNV-1a-64 over every reconstructed luma sample (row-major over the covered MI
/// units, sample-major u16 LE), the whole-region per-value oracle pin: a wrong
/// reconstruction anywhere in the covered region changes this checksum even at the
/// same sample count.
const LUMA_RECON_REGION_FNV1A64: u64 = 0x093f_7fdd_63fd_ae46;

/// The bottom-edge `TX_64X64 DC_PRED` block at MI(16,256), x[64,128) y[1024,1080):
/// its 56 in-frame rows (the 64-tall block overhangs the 1080-tall frame by 8). The
/// block reconstructs to the flat down-predicted `DC_PRED` oracle value `64`.
const BOTTOM_EDGE_FLAT: u16 = 64;
/// In-frame sample count of the bottom-edge block (`64 * 56`).
const BOTTOM_EDGE_SAMPLE_COUNT: usize = 64 * 56;
/// Sum of the bottom-edge block's 56 in-frame rows (`3584 * 64`).
const BOTTOM_EDGE_SAMPLE_SUM: u64 = 229_376;

/// The SB-column-3 `BLOCK_64X64 H_PRED` block (x[192,256) x y[0,64)) — the
/// §7.13.3.18 IntrABC source. Mode `H_PRED` (pAngle 180, `AngleDeltaY == 0`), split
/// into four `TX_32X32` `DCT_DCT` transforms. Each row is the §7.13.2.8 step-5
/// horizontal copy of the real reconstructed left column (the x=191 DC region edge,
/// flat `64`), so the block is flat `64` except the top-right `TX_32X32` which
/// carries a `+4` DC residual to flat `68` (verified against the oracle). Derived
/// offline from `ac0_prefiltered.yuv`.
const HPRED_BLOCK_X: usize = 192;
const HPRED_BLOCK_Y: usize = 0;
const HPRED_BLOCK_SIDE: usize = 64;
const HPRED_BLOCK_SAMPLE_COUNT: usize = HPRED_BLOCK_SIDE * HPRED_BLOCK_SIDE;
/// Sum of the H_PRED block (`3072 * 64 + 1024 * 68`).
const HPRED_BLOCK_SAMPLE_SUM: u64 = 266_240;
/// FNV-1a-64 over the H_PRED block (row-major, sample-major u16 LE).
const HPRED_BLOCK_FNV1A64: u64 = 0x615c_4637_5763_6325;
/// The flat `H_PRED` copy value (the x=191 left-column DC edge), and the value of
/// the top-right `TX_32X32` after its `+4` DC residual.
const HPRED_FLAT: u16 = 64;
const HPRED_TOP_RIGHT_STEP: u16 = 68;

/// The first §7.13.3.18 IntrABC block's `BLOCK_32X64` luma TARGET (MI(16,56) →
/// x[224,256) x y[64,128)). The §5.20.5.4 block vector is integer (row `-512`
/// eighth-pel == `-64` samples, col `0`), and the block is a `skip` leaf (zero
/// residual), so §7.13.3.18 reduces to a plain copy of the displaced `CurrFrame`
/// SOURCE rectangle x[224,256) x y[0,64) — the right half of the already-reconstructed
/// H_PRED block. Source (hence target) is the top-right `TX_32X32` `68` over its top
/// 32 rows (y[64,96), copied from the H_PRED `68` at y[0,32)) and flat `64` below
/// (y[96,128)). The oracle confirms `target == source` over the full 32x64 block.
/// Derived offline from `ac0_prefiltered.yuv`.
const INTRABC_TARGET_X: usize = 224;
const INTRABC_TARGET_Y: usize = 64;
const INTRABC_TARGET_WIDTH: usize = 32;
const INTRABC_TARGET_HEIGHT: usize = 64;
const INTRABC_SAMPLE_COUNT: usize = INTRABC_TARGET_WIDTH * INTRABC_TARGET_HEIGHT;
/// The IntrABC source rectangle (x[224,256) x y[0,64)) the target copies from — the
/// right half of the reconstructed SB-column-3 H_PRED block.
const INTRABC_SOURCE_X: usize = 224;
const INTRABC_SOURCE_Y: usize = 0;
/// The `68` band height of the IntrABC target: its top 32 rows (the copied top-right
/// `TX_32X32` `68`), the rest flat `64`.
const INTRABC_TOP_BAND_HEIGHT: usize = 32;
/// Sum of the IntrABC target (`1024 * 68 + 1024 * 64`).
const INTRABC_TARGET_SAMPLE_SUM: u64 = 135_168;
/// FNV-1a-64 over the IntrABC target (row-major, sample-major u16 LE).
const INTRABC_TARGET_FNV1A64: u64 = 0xb70e_5832_e8aa_2325;

/// The luma region newly ADMITTED by modelling the §7.12.2.1 step-8 SB-border
/// IntrABC SMVP candidate: x[128,224) x y[192,256) (96x64 = 6144 samples), the
/// region the MI(32,56) ref-MV-stack admission unblocks in the SB-row-1 walk. The
/// admission keeps the entropy parse synced past MI(32,56), so this downstream
/// proven-subset intra leaf reconstructs in decode order — a flat `64` rectangle
/// (the same DC-region edge value propagated through SB row 1). Derived offline
/// from the AVM pre-filter luma oracle (`ac0_prefiltered.yuv` / frame-0 prefilter
/// luma dump). These constants PIN the admitted samples per value (NOT merely the
/// aggregate count), mirroring [`INTRABC_TARGET_FNV1A64`]: a wrong reconstruction
/// of this region would change the sum / FNV even at the same sample count.
const STEP8_ADMITTED_REGION_X: usize = 128;
const STEP8_ADMITTED_REGION_Y: usize = 192;
const STEP8_ADMITTED_REGION_WIDTH: usize = 96;
const STEP8_ADMITTED_REGION_HEIGHT: usize = 64;
const STEP8_ADMITTED_SAMPLE_COUNT: usize =
    STEP8_ADMITTED_REGION_WIDTH * STEP8_ADMITTED_REGION_HEIGHT;
/// The flat oracle value across the newly-admitted region.
const STEP8_ADMITTED_FLAT: u16 = 64;
/// Sum of the newly-admitted region (`6144 * 64`).
const STEP8_ADMITTED_SAMPLE_SUM: u64 = 393_216;
/// FNV-1a-64 over the newly-admitted region (row-major, sample-major u16 LE).
const STEP8_ADMITTED_FNV1A64: u64 = 0xa61d_8c75_326d_e325;

/// The first NON-skip §7.13.3.18 IntrABC RESIDUAL block: the `BLOCK_32X64` luma TARGET
/// at MI(0,112) → x[448,480) x y[0,64). Its §5.20.5.4 block vector is integer (row `0`,
/// col `-256` eighth-pel == `-32` samples), so §7.13.3.18 reduces to a plain copy of the
/// displaced SOURCE rectangle x[416,448) y[0,64) (the flat-`68` right half of the SB
/// column DC region) AS THE PREDICTION; the block is NON-skip, so each §5.20.7.27
/// residual transform leaf then ADDS its decoded residual onto that copied predictor (the
/// first leaf has `eob == 11`). The result is the per-sample `Clip1(prediction +
/// inverse-transform(residual))` — a genuine NON-flat block (oracle values 64..73, not a
/// flat copy), proving the residual-add ran. Derived offline from `ac0_prefiltered.yuv`.
const INTRABC_RESIDUAL_TARGET_X: usize = 448;
const INTRABC_RESIDUAL_TARGET_Y: usize = 0;
const INTRABC_RESIDUAL_TARGET_WIDTH: usize = 32;
const INTRABC_RESIDUAL_TARGET_HEIGHT: usize = 64;
const INTRABC_RESIDUAL_SAMPLE_COUNT: usize =
    INTRABC_RESIDUAL_TARGET_WIDTH * INTRABC_RESIDUAL_TARGET_HEIGHT;
/// Sum of the IntrABC residual target (the §7.14.3 prediction+residual reconstruction).
const INTRABC_RESIDUAL_TARGET_SAMPLE_SUM: u64 = 142_016;
/// FNV-1a-64 over the IntrABC residual target (row-major, sample-major u16 LE): a wrong
/// composition (a flat copy with no residual, or a residual onto the wrong predictor)
/// changes this checksum even at the same sample count.
const INTRABC_RESIDUAL_TARGET_FNV1A64: u64 = 0x1b9c_a7cf_eaad_e9a5;

/// The frame-origin chroma `DC_PRED` transform side (a 32x32 §5.20.6 `TxSize` in
/// the 4:2:0 chroma plane, the chroma leaf covering the §5.20.3.1 SDP chroma tree
/// at the frame origin). Both U and V resolve to chroma `DC_PRED` with no neighbour
/// (the §7.13.2.1 no-neighbour fallback) and an `all_zero` residual, so each plane
/// is the flat 10-bit DC fallback `1 << (10 - 1)` == `512`.
const CHROMA_ORIGIN_SIDE: usize = 32;
/// Flat oracle value for the frame-origin chroma `DC_PRED` block (10-bit
/// no-neighbour DC fallback), derived offline from `ac0_prefiltered.yuv` (its first
/// 32x32 U and V samples are both uniformly `512`).
const CHROMA_ORIGIN_FLAT: u16 = 512;
const CHROMA_ORIGIN_SAMPLE_COUNT: usize = CHROMA_ORIGIN_SIDE * CHROMA_ORIGIN_SIDE;
/// Sum of one 32x32 chroma origin plane (`1024 * 512`).
const CHROMA_ORIGIN_SAMPLE_SUM: u64 = 524_288;
/// FNV-1a-64 over one 32x32 chroma origin plane (row-major, sample-major u16 LE),
/// matching the offline oracle checksum derivation (identical for U and V).
const CHROMA_ORIGIN_FNV1A64: u64 = 0xa53e_893c_24f1_e325;

fn local_ac0ej3_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SPLOT_AC0EJ3_IVF") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join("Documents/SplotLabs/ac0ej3.ivf"))
}

fn context() -> DecodeContext {
    DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).expect("context")
}

fn require_fixture() -> PathBuf {
    let Some(path) = local_ac0ej3_path() else {
        panic!("set SPLOT_AC0EJ3_IVF or HOME for the ignored local ac0ej3 reconstruction test");
    };
    assert!(
        path.is_file(),
        "local ac0ej3 fixture not found at {}",
        path.display()
    );
    path
}

/// Reads the local ac0ej3 fixture, plans it, and runs the selectable
/// transform-record walk, returning the reconstruction sink. The shared preamble
/// for every ignored ac0ej3 oracle-pin test (fixture → plan → reconstruct).
fn reconstruct_ac0ej3_sink() -> WienerNsLrReconSink<u16> {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");
    reconstruct_ac0ej3_intra_region_from_plan(&bytes, options, &plan)
        .expect("reconstruct ac0ej3 region")
}

/// As [`reconstruct_ac0ej3_sink`], but drives the DIAGNOSTIC-ONLY full-reconstruction
/// sink (every luma leaf reconstructed in decode order, gates dropped, far-edge
/// read-or-pad from the per-transform `BlockDecoded` availability). Used ONLY by the
/// `SPLOT_AC0EJ3_FULL_RECON` whole-frame differential harness.
fn reconstruct_ac0ej3_full_recon_sink() -> WienerNsLrReconSink<u16> {
    let path = require_fixture();
    let bytes = std::fs::read(&path).expect("read ac0ej3 fixture");
    let options = DecodeOptions::default();
    let plan = context().plan_bytes(&bytes, options).expect("plan ac0ej3");
    reconstruct_ac0ej3_intra_region_from_plan_with_mode(&bytes, options, &plan, true)
        .expect("full-recon ac0ej3 region")
}

/// FNV-1a-64 over a u16 sample stream (little-endian bytes), matching the offline
/// oracle checksum derivation.
struct Fnv1a64(u64);

impl Fnv1a64 {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn update_u16(&mut self, value: u16) {
        for byte in value.to_le_bytes() {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

/// Verifies a reconstructed luma rectangle `[x, x+width) x [y, y+height)` against
/// the pre-filter oracle: asserts every sample equals `expected(x, y)`, then pins
/// the aggregate sample count, sum, and FNV-1a-64. `region` is a human label for
/// assertion messages. Shared by the per-region oracle-pin tests so each one is a
/// declaration of its bbox + expected closure + committed (count, sum, fnv).
fn assert_luma_region_oracle(
    sink: &WienerNsLrReconSink<u16>,
    region: &str,
    (x0, width): (usize, usize),
    (y0, height): (usize, usize),
    expected: impl Fn(usize, usize) -> u16,
    pins: (usize, u64, u64),
) {
    let (want_count, want_sum, want_fnv) = pins;
    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in y0..y0 + height {
        for x in x0..x0 + width {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            let want = expected(x, y);
            assert_eq!(
                sample, want,
                "{region} luma ({x},{y}) must be {want}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }
    assert_eq!(count, want_count, "{region} sample count");
    assert_eq!(
        sum, want_sum,
        "{region} sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        want_fnv,
        "{region} FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Infrastructure check: the reconstruction bridge threads a sink through the
/// selectable transform-record walk and reconstructs the verified `DC_PRED` luma
/// region into a current-frame workspace in decode order, while the public decode
/// path stays fail-closed. This proves the bridge wiring (sink threading +
/// primitive reuse) over the live ac0ej3 parse.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_reconstruction_bridge_populates_a_workspace_region() {
    let sink = reconstruct_ac0ej3_sink();

    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    assert!(
        luma4x4 > 0,
        "the reconstruction bridge must populate luma samples"
    );
    for y in 0..BLOCK0_SIDE {
        for x in 0..BLOCK0_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            assert!(sample < 1024, "10-bit luma sample out of range: {sample}");
        }
    }
}

/// The §7.13.2.1 PER-TRANSFORM far-edge availability the bridge records for the live
/// ac0ej3 stream matches AVM `has_top_right` (`av2/common/reconintra.c:59`) at the
/// transform granularity, not the partition granularity that under-counted before.
///
/// * MI(224,30) is the LEFT `TX_16X8` of a `BLOCK_32X8` `V_PRED` coding block. AVM
///   `has_top_right` short-circuits on `col_off + tx_size_wide_unit < plane_bw_unit`
///   (`0 + 4 < 8`), reading the already-decoded row above the WHOLE 32x8 block, so
///   `num4AboveRight == 4`. The old block-granularity `count_top_right_avail(w4 ==
///   n4w == 8)` scanned PAST the coding block into the next, undecoded block and
///   recorded `0` (the #566 bug).
/// * MI(216,72) is a NON-top `TX_8X32` of a `BLOCK_16X64` `V_PRED` block (`row_off >
///   0`, `col_off == 0`). AVM `reconintra.c:110` returns `tx_size_wide_unit == 2`
///   for the in-block above-right; the raw §5.20.7.25 scan would have returned `0`
///   (the in-block above transform is not yet `BlockDecoded`-marked at the callback).
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_per_transform_far_edge_matches_avm_has_top_right() {
    let sink = reconstruct_ac0ej3_sink();
    assert_eq!(
        sink.block_decoded_far_edge(224, 30),
        Some((4, 0)),
        "LEFT TX_16X8 of the 32x8 V_PRED block reads its above-right within the coding block (num4AboveRight == 4, not the partition-granularity 0)"
    );
    assert_eq!(
        sink.block_decoded_far_edge(216, 72),
        Some((2, 0)),
        "in-block non-top TX_8X32 of the 16x64 V_PRED block reads its above-right within the coding block (num4AboveRight == 2)"
    );
}

/// Bit-exact verification against the AVM pre-filter reconstruction oracle for the
/// frame-origin `DC_PRED` luma block. With the now-AVM-faithful first-superblock
/// parse, including the CCSO read, the bridge reconstructs this block bit-exact: every
/// sample is the committed flat value `68`, and the block's sum and FNV-1a-64
/// checksum match the oracle. This is the first BIT-EXACT ac0ej3 reconstruction
/// milestone, verified against the AVM pre-filter oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_frame_origin_dc_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in 0..BLOCK0_SIDE {
        for x in 0..BLOCK0_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            assert_eq!(
                sample, BLOCK0_FLAT_LUMA,
                "frame-origin luma ({x},{y}) must be {BLOCK0_FLAT_LUMA}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(
        count, BLOCK0_SAMPLE_COUNT,
        "frame-origin block sample count"
    );
    assert_eq!(
        sum, BLOCK0_SAMPLE_SUM,
        "frame-origin block sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        BLOCK0_FNV1A64,
        "frame-origin block FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Bit-exact verification of the FULL `BLOCK_16X64` left luma column against the
/// AVM pre-filter oracle. The sink reconstructs not only the frame-origin `DC_PRED`
/// 16x16 leaf (flat `68`) but the down-predicted `DC_PRED` 16-wide transforms below
/// it (rows 16..64, flat `64`), so the whole 16x64 (1024-sample) column is verified
/// — a 4x widening of the asserted bit-exact region beyond the origin 16x16 block.
/// Both the per-sample values and the column's sum + FNV-1a-64 checksum match the
/// oracle. (SMOOTH leaves past this column are DEFERRED, not widened here — see the
/// module doc comment: splot's parse diverges from AVM immediately past this column,
/// so every reachable splot-SMOOTH leaf disagrees with the AVM mode oracle.)
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_block_16x64_luma_column_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in 0..LUMA_COLUMN_HEIGHT {
        for x in 0..LUMA_COLUMN_WIDTH {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            let expected = if y < BLOCK0_SIDE {
                BLOCK0_FLAT_LUMA
            } else {
                LUMA_COLUMN_BELOW_ORIGIN
            };
            assert_eq!(
                sample, expected,
                "BLOCK_16X64 luma ({x},{y}) must be {expected}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(
        count, LUMA_COLUMN_SAMPLE_COUNT,
        "BLOCK_16X64 column sample count"
    );
    assert_eq!(
        sum, LUMA_COLUMN_SAMPLE_SUM,
        "BLOCK_16X64 column sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        LUMA_COLUMN_FNV1A64,
        "BLOCK_16X64 column FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Bit-exact verification of the frame-origin chroma `DC_PRED` blocks (U and V)
/// against the AVM pre-filter oracle. With the AVM-faithful first-superblock parse
/// (the §8.3.2 `is_cfl` neighbour-context fix), the §5.20.3.1 SDP chroma tree
/// at the frame origin resolves to chroma `DC_PRED` with an `all_zero` residual
/// (verified against `inspect --uv_mode`), so the sink reconstructs the 32x32 U and
/// V origin transforms to the flat 10-bit no-neighbour DC fallback `512`. Both
/// planes' per-sample values, sums, and FNV-1a-64 checksums match the oracle —
/// 2048 chroma samples (1024 U + 1024 V) added to the asserted bit-exact region
/// beyond the 1024-sample luma column. The next chroma leaf (chroma `(32,0)`,
/// resolved `SMOOTH_PRED`) is DEFERRED, so it stays at the unreconstructed fill
/// value `0`; this asserts that deferral too, proving the sink never claims a chroma
/// sample it has not proven bit-exact. The U origin block reconstructs independently
/// of the deferred luma DC chain to its right (chroma `DC_PRED` reads only its own
/// off-frame edges, never the luma plane), so the wall at the non-square `TX_16X64`
/// luma keystone does not contaminate it.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_frame_origin_chroma_dc_blocks_reconstruct_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    for plane in [PlaneId::U, PlaneId::V] {
        let mut fnv = Fnv1a64::new();
        let mut sum: u64 = 0;
        let mut count = 0usize;
        for y in 0..CHROMA_ORIGIN_SIDE {
            for x in 0..CHROMA_ORIGIN_SIDE {
                let sample = sink.reconstructed_sample(plane, x, y).unwrap();
                assert_eq!(
                    sample, CHROMA_ORIGIN_FLAT,
                    "chroma {plane:?} origin ({x},{y}) must be {CHROMA_ORIGIN_FLAT}, got {sample}"
                );
                fnv.update_u16(sample);
                sum += u64::from(sample);
                count += 1;
            }
        }
        assert_eq!(
            count, CHROMA_ORIGIN_SAMPLE_COUNT,
            "chroma {plane:?} origin block sample count"
        );
        assert_eq!(
            sum, CHROMA_ORIGIN_SAMPLE_SUM,
            "chroma {plane:?} origin block sample sum must match the pre-filter oracle"
        );
        assert_eq!(
            fnv.finish(),
            CHROMA_ORIGIN_FNV1A64,
            "chroma {plane:?} origin FNV-1a-64 must match the pre-filter oracle (bit-exact)"
        );
    }

    assert_eq!(
        sink.reconstructed_sample(PlaneId::U, CHROMA_ORIGIN_SIDE, 0)
            .unwrap(),
        0,
        "the deferred SMOOTH chroma leaf at chroma (32,0) must stay unreconstructed"
    );

    let (luma4x4, chroma4x4) = sink.reconstructed_counts();
    assert_eq!(
        luma4x4 * 16,
        LUMA_RECON_SAMPLE_TOTAL,
        "verified luma region is the bit-exact DC + cardinal + IntrABC region"
    );
    assert_eq!(
        chroma4x4 * 16,
        2 * CHROMA_ORIGIN_SAMPLE_COUNT,
        "verified chroma region is the U+V 32x32 origin blocks (2048 samples)"
    );
}

/// Bit-exact verification of the FULL first-3-superblock luma DC region (x[0,192) x
/// y[0,128), 24576 samples) against the AVM pre-filter oracle — a 24x widening of
/// the original 1024-sample `BLOCK_16X64` column.
///
/// This is unlocked by fixing the MI(4,0) `TX_16X64` keystone with two
/// reconstruction fixes: (1) the §7.15.4 outer inverse transform now drives the
/// NON-SQUARE residual path, and (2) the §7.13.2.12 IBP DC modifier (ac0ej3 has
/// `enable_ibp == 1`) blends the MI(4,0) left edge columns toward the reconstructed
/// `BLOCK_16X64` left neighbour, producing the oracle's `65` step in the top-left 3
/// columns. Every DC block downstream bordered that keystone through the §7.13.2
/// edge-coverage guard, so the whole first-3-SB luma DC chain now reconstructs in
/// one shot. The region is `68` (origin leaf, 256 samples), `65` (the MI(4,0) IBP
/// step, 48 samples), and `64` (the rest); per-sample, sum, and FNV-1a-64 all
/// match the oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_first_three_superblock_luma_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    let mut mi40_step_samples = 0usize;
    for y in 0..LUMA_REGION_HEIGHT {
        for x in 0..LUMA_REGION_WIDTH {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            let in_mi40_step = (16..19).contains(&x) && y < BLOCK0_SIDE;
            let expected = if y < BLOCK0_SIDE && x < BLOCK0_SIDE {
                BLOCK0_FLAT_LUMA
            } else if in_mi40_step {
                MI40_IBP_STEP
            } else {
                LUMA_COLUMN_BELOW_ORIGIN
            };
            assert_eq!(
                sample, expected,
                "first-3-SB luma ({x},{y}) must be {expected}, got {sample}"
            );
            if in_mi40_step {
                mi40_step_samples += 1;
            }
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(mi40_step_samples, 48, "MI(4,0) IBP DC step sample count");
    assert_eq!(
        count, LUMA_REGION_SAMPLE_COUNT,
        "first-3-SB luma sample count"
    );
    assert_eq!(
        sum, LUMA_REGION_SAMPLE_SUM,
        "first-3-SB luma sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        LUMA_REGION_FNV1A64,
        "first-3-SB luma FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Bit-exact verification of the SB-column-3 `BLOCK_64X64 H_PRED` block (x[192,256)
/// x y[0,64)) against the AVM pre-filter oracle — the §7.13.3.18 IntrABC SOURCE
/// block that the DC-only sink could not reconstruct.
///
/// The block is cardinal `H_PRED` (pAngle 180, `AngleDeltaY == 0`): each row is the
/// §7.13.2.8 step-5 horizontal copy `pred[i][j] = LeftCol[i]` of the real
/// reconstructed left column (no above, no corner, no IDIF, no `useIBP` — pAngle 180
/// is excluded by the §7.13.2.7 `pAngle < 90 || pAngle > 180` gate). Its left column
/// (x=191) is the right edge of the already-reconstructed first-3-superblock DC
/// region (flat `64`), so the four `TX_32X32` `DCT_DCT` transforms reconstruct flat
/// `64` except the top-right transform (x[224,256) x y[0,32)), which carries a `+4`
/// DC residual over its (flat-`64`) left neighbour and so is flat `68`. Per-sample,
/// sum, and FNV-1a-64 all match the oracle. This is the first bit-exact DIRECTIONAL
/// (non-DC) ac0ej3 block, and it unblocks the IntrABC brick (which copies from it).
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_sb_column3_hpred_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for y in HPRED_BLOCK_Y..HPRED_BLOCK_Y + HPRED_BLOCK_SIDE {
        for x in HPRED_BLOCK_X..HPRED_BLOCK_X + HPRED_BLOCK_SIDE {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            let in_top_right = (224..256).contains(&x) && y < 32;
            let expected = if in_top_right {
                HPRED_TOP_RIGHT_STEP
            } else {
                HPRED_FLAT
            };
            assert_eq!(
                sample, expected,
                "H_PRED block luma ({x},{y}) must be {expected}, got {sample}"
            );
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert_eq!(count, HPRED_BLOCK_SAMPLE_COUNT, "H_PRED block sample count");
    assert_eq!(
        sum, HPRED_BLOCK_SAMPLE_SUM,
        "H_PRED block sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        HPRED_BLOCK_FNV1A64,
        "H_PRED block FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );

    assert_eq!(
        sink.reconstructed_sample(PlaneId::Y, 256, 0).unwrap(),
        68,
        "the frame-top V_PRED block at x[256,320) y=0 reconstructs to flat 68 via the §7.13.2.1 no-above fallback",
    );

    let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
    assert_eq!(
        luma4x4 * 16,
        LUMA_RECON_SAMPLE_TOTAL,
        "the parse-advanced walk reconstructs the bit-exact luma region"
    );
}

/// Bit-exact verification of the frame-top `TX_64X32` `V_PRED` leaf MI(64,0) →
/// x[256,320) x y[0,32) against the AVM pre-filter oracle — the §7.13.2.1
/// no-above single-neighbour edge fallback.
///
/// The block is NON-SQUARE (`log2_width == 6`, `log2_height == 5`) `V_PRED`
/// (pAngle 90, `AngleDeltaY == 0`), `all_zero` (zero residual), at the FRAME TOP
/// (`mi_row == 0`, so `haveAbove == 0`) with a reconstructed left neighbour (the
/// SB-column-3 H_PRED region, x=255 flat `68`). §7.13.2.1 (`haveAbove == 0 &&
/// haveLeft == 1`) synthesizes `AboveRow[i] = CurrFrame[plane][y][x-1]` — the flat
/// `68` left corner repeated across the W-wide synthesized above row — and the
/// §7.13.2.8 V_PRED copy reconstructs the whole block to flat `68`. This is the
/// frontier block whose admission CASCADES into the top-row x[256,448) y[0,64)
/// region (pinned by [`LUMA_RECON_REGION_FNV1A64`]). Pinned per value (every sample
/// `68`) so a transpose / wrong-fallback would change the sum / FNV.
const VPRED_TOP_BLOCK_X: usize = 256;
const VPRED_TOP_BLOCK_W: usize = 64;
const VPRED_TOP_BLOCK_H: usize = 32;
const VPRED_TOP_FLAT: u16 = 68;
const VPRED_TOP_SAMPLE_COUNT: usize = VPRED_TOP_BLOCK_W * VPRED_TOP_BLOCK_H;
const VPRED_TOP_SAMPLE_SUM: u64 = 139_264;
const VPRED_TOP_FNV1A64: u64 = 0x5ef5_adc7_8b18_e325;
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_frame_top_vpred_no_above_fallback_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    assert_luma_region_oracle(
        &sink,
        "frame-top V_PRED no-above fallback block",
        (VPRED_TOP_BLOCK_X, VPRED_TOP_BLOCK_W),
        (0, VPRED_TOP_BLOCK_H),
        |_x, _y| VPRED_TOP_FLAT,
        (
            VPRED_TOP_SAMPLE_COUNT,
            VPRED_TOP_SAMPLE_SUM,
            VPRED_TOP_FNV1A64,
        ),
    );
}

/// The two §7.13.2.2 PAETH (`PAETH_PRED`) `TX_64X64` `all_zero` leaves MI(64,16)
/// (x[256,320) y[64,128)) and MI(80,16) (x[320,384) y[64,128)), the first PAETH
/// blocks the sink reconstructs. Both are admitted in the two-sided
/// `haveAbove == 1 && haveLeft == 1` config, where §7.13.2.1 supplies the real
/// reconstructed above row, left column, AND corner
/// `AboveRow[-1] = CurrFrame[plane][y-1][x-1]`. MI(64,16) genuinely exercises the
/// Paeth predictor over a NON-flat left column (a mix of `64` / `68`: the
/// reconstructed x=255 edge) and corner `64`, yet every output sample resolves to
/// the flat `68` of the above row — bit-exact vs the oracle. MI(80,16) is admitted
/// once MI(64,16) cascades to give it a reconstructed left edge. Pinned per value
/// (combined contiguous x[256,384) y[64,128)) so a left/above/corner mix-up changes
/// the sum / FNV. Derived offline from `ac0_prefiltered.yuv`.
const PAETH_BLOCK_X: usize = 256;
const PAETH_BLOCK_W: usize = 128;
const PAETH_BLOCK_Y: usize = 64;
const PAETH_BLOCK_H: usize = 64;
const PAETH_FLAT: u16 = 68;
const PAETH_SAMPLE_COUNT: usize = PAETH_BLOCK_W * PAETH_BLOCK_H;
const PAETH_SAMPLE_SUM: u64 = 557_056;
const PAETH_FNV1A64: u64 = 0x0808_60ea_bffd_2325;
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_paeth_two_sided_blocks_reconstruct_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let left_edge: std::collections::BTreeSet<u16> = (PAETH_BLOCK_Y..PAETH_BLOCK_Y + PAETH_BLOCK_H)
        .map(|y| {
            sink.reconstructed_sample(PlaneId::Y, PAETH_BLOCK_X - 1, y)
                .unwrap()
        })
        .collect();
    assert!(
        left_edge.len() > 1,
        "PAETH left column must be non-flat to exercise the predictor, got {left_edge:?}"
    );

    assert_luma_region_oracle(
        &sink,
        "two-sided PAETH TX_64X64 blocks",
        (PAETH_BLOCK_X, PAETH_BLOCK_W),
        (PAETH_BLOCK_Y, PAETH_BLOCK_H),
        |_x, _y| PAETH_FLAT,
        (PAETH_SAMPLE_COUNT, PAETH_SAMPLE_SUM, PAETH_FNV1A64),
    );
}

/// Bit-exact verification of the FIRST §7.13.3.18 IntrABC block's `BLOCK_32X64`
/// luma TARGET (MI(16,56) → x[224,256) x y[64,128)) against the AVM pre-filter
/// oracle — the first break through the original mission IntrABC wall.
///
/// The §5.20.5.4 block vector is integer (row `-512` eighth-pel == `-64` samples,
/// col `0`) and the leaf is a `skip` block (zero residual), so §7.13.3.18 reduces to
/// a plain copy of the displaced `CurrFrame` SOURCE rectangle x[224,256) x y[0,64)
/// (the right half of the already-reconstructed SB-column-3 H_PRED block) into the
/// target. Two independent checks: (1) every target sample equals the SOURCE sample
/// directly above it by the integer DV (the copy is faithful), and (2) every target
/// sample matches the committed oracle constant (`68` over the top 32 rows copied
/// from the H_PRED top-right `TX_32X32`, `64` below), with the target's sum and
/// FNV-1a-64 matching the oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_first_intrabc_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    for row in 0..INTRABC_TARGET_HEIGHT {
        for col in 0..INTRABC_TARGET_WIDTH {
            let tx = INTRABC_TARGET_X + col;
            let ty = INTRABC_TARGET_Y + row;
            let target_sample = sink.reconstructed_sample(PlaneId::Y, tx, ty).unwrap();

            let sx = INTRABC_SOURCE_X + col;
            let sy = INTRABC_SOURCE_Y + row;
            let source_sample = sink.reconstructed_sample(PlaneId::Y, sx, sy).unwrap();
            assert_eq!(
                target_sample, source_sample,
                "IntrABC target ({tx},{ty}) must equal its DV source ({sx},{sy})"
            );

            let expected = if row < INTRABC_TOP_BAND_HEIGHT {
                HPRED_TOP_RIGHT_STEP
            } else {
                HPRED_FLAT
            };
            assert_eq!(
                target_sample, expected,
                "IntrABC target ({tx},{ty}) must be {expected}, got {target_sample}"
            );
            fnv.update_u16(target_sample);
            sum += u64::from(target_sample);
            count += 1;
        }
    }

    assert_eq!(count, INTRABC_SAMPLE_COUNT, "IntrABC target sample count");
    assert_eq!(
        sum, INTRABC_TARGET_SAMPLE_SUM,
        "IntrABC target sample sum must match the pre-filter oracle"
    );
    assert_eq!(
        fnv.finish(),
        INTRABC_TARGET_FNV1A64,
        "IntrABC target FNV-1a-64 must match the pre-filter reconstruction oracle (bit-exact)"
    );
}

/// Bit-exact verification of the first NON-skip §7.13.3.18 IntrABC RESIDUAL block —
/// the `BLOCK_32X64` target at MI(0,112) → x[448,480) x y[0,64) — against the AVM
/// pre-filter oracle. The block's integer DV (col `-32` samples) copies the flat-`68`
/// displaced SOURCE x[416,448) y[0,64) as the §7.13.2 PREDICTION, then each §5.20.7.27
/// residual transform leaf (the first `eob == 11`) ADDS its decoded residual onto that
/// predictor via the §7.14.4 / §7.15.4 / §7.14.3 dequant → inverse → Clip1-add path. The
/// reconstruction is therefore a genuine NON-FLAT block (it must NOT equal the flat-`68`
/// source copy alone): this test asserts (1) the region is non-flat — so the residual
/// actually ran — and (2) the target's count + sum + FNV-1a-64 match the oracle, pinning
/// the prediction+residual composition per value. A confident-wrong composition (the
/// wrong predictor, a missing residual, or a residual onto a fill value) changes the
/// sum / FNV even at the same sample count.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_first_intrabc_residual_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    let mut distinct = std::collections::BTreeSet::new();
    for row in 0..INTRABC_RESIDUAL_TARGET_HEIGHT {
        for col in 0..INTRABC_RESIDUAL_TARGET_WIDTH {
            let tx = INTRABC_RESIDUAL_TARGET_X + col;
            let ty = INTRABC_RESIDUAL_TARGET_Y + row;
            let sample = sink.reconstructed_sample(PlaneId::Y, tx, ty).unwrap();
            distinct.insert(sample);
            fnv.update_u16(sample);
            sum += u64::from(sample);
            count += 1;
        }
    }

    assert!(
        distinct.len() > 1,
        "IntrABC residual target must be NON-flat (the residual must have been added), \
         got a single value {distinct:?}"
    );
    assert_eq!(
        count, INTRABC_RESIDUAL_SAMPLE_COUNT,
        "IntrABC residual target sample count"
    );
    assert_eq!(
        sum, INTRABC_RESIDUAL_TARGET_SAMPLE_SUM,
        "IntrABC residual target sum must match the pre-filter oracle (prediction+residual)"
    );
    assert_eq!(
        fnv.finish(),
        INTRABC_RESIDUAL_TARGET_FNV1A64,
        "IntrABC residual target FNV-1a-64 must match the pre-filter oracle (bit-exact)"
    );
}

/// Bit-exact verification of the luma region the §7.12.2.1 step-8 SB-border IntrABC
/// SMVP candidate newly ADMITTS — x[128,224) x y[192,256) (6144 samples) — against
/// the AVM pre-filter oracle. This PINS the newly-admitted samples per value (sum +
/// FNV-1a-64 + the flat oracle value at every position), so the region cannot
/// reconstruct to wrong values while still passing the aggregate count assertion.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_step8_admitted_region_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    assert_luma_region_oracle(
        &sink,
        "step-8 admitted region",
        (STEP8_ADMITTED_REGION_X, STEP8_ADMITTED_REGION_WIDTH),
        (STEP8_ADMITTED_REGION_Y, STEP8_ADMITTED_REGION_HEIGHT),
        |_x, _y| STEP8_ADMITTED_FLAT,
        (
            STEP8_ADMITTED_SAMPLE_COUNT,
            STEP8_ADMITTED_SAMPLE_SUM,
            STEP8_ADMITTED_FNV1A64,
        ),
    );
}

/// Whole-region per-value oracle pin: with the §7.12.2.19 IntrABC ref-MV weight
/// sort modelled (per-candidate §7.12.2.6 weights + the max-weight-to-slot-0
/// reorder), the parse-advanced walk reconstructs `233472` bit-exact luma samples.
/// This walks EVERY MI unit the sink covered (not a fixed rectangle, since the
/// region is a non-rectangular union) and pins the aggregate count + sum + FNV-1a-64
/// against the AVM pre-filter oracle: a wrong reconstruction anywhere in the covered
/// region changes the sum / FNV even at the same sample count. The aggregate was
/// derived offline with ZERO per-sample mismatch vs the oracle luma plane
/// (`/tmp/pref.yuv`, md5 `f7959cb85a41dcf0e6ebf9179835da03`).
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_full_reconstructed_luma_region_matches_prefilter_oracle_aggregate() {
    let sink = reconstruct_ac0ej3_sink();

    let mut fnv = Fnv1a64::new();
    let mut sum: u64 = 0;
    let mut count = 0usize;
    sink.for_each_reconstructed_luma_sample(|_x, _y, sample| {
        fnv.update_u16(sample);
        sum += u64::from(sample);
        count += 1;
    })
    .expect("walk reconstructed luma region");

    assert_eq!(
        count, LUMA_RECON_SAMPLE_TOTAL,
        "reconstructed luma region sample count"
    );
    assert_eq!(
        sum, LUMA_RECON_REGION_SAMPLE_SUM,
        "reconstructed luma region sum must match the pre-filter oracle aggregate"
    );
    assert_eq!(
        fnv.finish(),
        LUMA_RECON_REGION_FNV1A64,
        "reconstructed luma region FNV-1a-64 must match the pre-filter oracle (bit-exact)"
    );
}

/// Bit-exact verification of the bottom-edge `TX_64X64 DC_PRED` block at MI(16,256)
/// (sample x[64,128) y[1024,1080)) whose LEFT reference column has only 56 in-frame
/// rows (the 64-tall block overhangs the 1080-tall luma frame by 8 rows). The
/// §7.13.2.1 frame-edge edge-extension (replicate the LAST in-frame left sample to the
/// block's full 64-tall nominal height, per AVM `reconintra.c:1191-1195`) lets the DC
/// primitive consume a full-length edge instead of erroring
/// `IntraPredictionEdgeLengthMismatch{expected:64,actual:56}`. The block reconstructs
/// its 56 in-frame rows to the flat oracle value `64` (the partial-edge block is NOT
/// errored), verified per-sample against the AVM pre-filter oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_bottom_edge_partial_left_edge_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();

    let mut count = 0usize;
    let mut sum: u64 = 0;
    for y in 1024..1080 {
        for x in 64..128 {
            let sample = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
            assert_eq!(
                sample, BOTTOM_EDGE_FLAT,
                "bottom-edge partial-left block luma ({x},{y}) must be {BOTTOM_EDGE_FLAT}, got {sample}"
            );
            sum += u64::from(sample);
            count += 1;
        }
    }
    assert_eq!(
        count, BOTTOM_EDGE_SAMPLE_COUNT,
        "bottom-edge block sample count"
    );
    assert_eq!(
        sum, BOTTOM_EDGE_SAMPLE_SUM,
        "bottom-edge partial-left block sum must match the pre-filter oracle"
    );
}

/// The AVM pre-filter oracle samples (row-major, `[y][x]`) for the §7.13.2.8
/// zone-1 one-sided IDIF leaf at MI(248,28), x[992,1000) y[112,120). A near-flat
/// `68` with a `66`/`67` right column produced by the IDIF reading the ramping
/// above-right. Derived offline from `ac0_prefiltered.yuv`. The right-column
/// gradient is the ASYMMETRIC signal that distinguishes a correct one-sided
/// projection (and `num4AboveRight` clamp) from a flat copy: a wrong filter phase
/// or above-right clamp changes those samples.
#[rustfmt::skip]
const ONE_SIDED_ZONE1_ORACLE: [[u16; 8]; 8] = [
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 67],
    [68, 68, 68, 68, 68, 68, 68, 67],
    [68, 68, 68, 68, 68, 68, 68, 67],
    [68, 68, 68, 68, 68, 68, 68, 66],
    [68, 68, 68, 68, 68, 68, 68, 66],
    [68, 68, 68, 68, 68, 68, 68, 66],
    [68, 68, 68, 68, 68, 68, 68, 66],
];
const ONE_SIDED_ZONE1_SAMPLE_COUNT: usize = 64;
/// Sum of the MI(248,28) zone-1 one-sided block (`[[u16; 8]; 8]`, row-major).
const ONE_SIDED_ZONE1_SAMPLE_SUM: u64 = 4_341;
/// FNV-1a-64 over the block (row-major, sample-major u16 LE).
const ONE_SIDED_ZONE1_FNV1A64: u64 = 0x3358_e0b7_861f_baea;

/// Bit-exact verification of the §7.13.2.8 ZONE-1 ONE-SIDED IDIF luma leaf at
/// MI(248,28) (sample x[992,1000) y[112,120), a `TX_8X8` `D67_PRED` block with
/// `AngleDeltaY == -3`, so `pAngle == Mode_To_Angle[D67] + (-3) * ANGLE_STEP ==
/// 67 - 9 == 58`... resolved to `pAngle 81` after the §5.20.5.3 `y_mode` neighbour
/// remap — a zone-1 above-reading angle whose `dx == Dr_Intra_Derivative[81] == 8`
/// projects up-and-right into the real reconstructed above-right). The leaf is the
/// proven no-edge-filter subset: ac0ej3's `enable_intra_edge_filter == 1` but the
/// §7.13.2.17 strength is `0` for this `w + h == 16`, `|angleAbove| == 9` block (a
/// genuine `av2_filter_intra_edge` no-op), no §7.13.2.7 corner filter (`w + h <
/// 24`), and `useIBP == 0` (the odd `AngleDeltaY` forces the §7.13.2.7 even-delta
/// gate off), so the raw-edge IDIF reproduces the spec output. Verified per-sample
/// against the AVM pre-filter oracle.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_one_sided_zone1_idif_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();
    assert_luma_region_oracle(
        &sink,
        "MI(248,28) zone-1 one-sided IDIF",
        (992, 8),
        (112, 8),
        |x, y| ONE_SIDED_ZONE1_ORACLE[y - 112][x - 992],
        (
            ONE_SIDED_ZONE1_SAMPLE_COUNT,
            ONE_SIDED_ZONE1_SAMPLE_SUM,
            ONE_SIDED_ZONE1_FNV1A64,
        ),
    );
}

/// The AVM pre-filter oracle samples (row-major, `[y][x]`) for the §7.13.2.8
/// zone-1 one-sided IDIF leaf at MI(148,168), x[592,608) y[672,688), with the
/// §7.13.2.18 edge filter AND §7.13.2.14 corner filter ACTIVE. A 16x16 `D45`-seed
/// leaf, `pAngle 58` (`AngleDeltaY` resolves the one-sided zone-1 angle whose
/// `dx == Dr_Intra_Derivative[58]` projects up-and-right), §7.13.2.17 `strength 3`
/// over the above edge (`filterType` from the real neighbour modes), corner filter
/// fires (`w + h == 32 >= 24`). The strong up-and-right GRADIENT (a 69→281 ramp) is
/// the ASYMMETRIC signal that distinguishes the correct filtered-edge projection
/// from the raw-edge one: a wrong strength, kernel phase, corner blend, or `numPx`
/// changes these samples. Derived offline from `/tmp/pref.yuv`.
#[rustfmt::skip]
const ONE_SIDED_EDGEFILT_ORACLE: [[u16; 16]; 16] = [
    [69, 70, 71, 72, 73, 74, 74, 75, 75, 75, 75, 75, 75, 75, 78, 84],
    [70, 72, 74, 76, 78, 79, 80, 81, 81, 81, 81, 81, 81, 83, 88, 97],
    [71, 74, 77, 79, 81, 83, 84, 85, 86, 86, 86, 86, 87, 90, 97, 108],
    [71, 75, 78, 81, 83, 85, 87, 88, 89, 89, 89, 89, 92, 97, 106, 120],
    [72, 75, 79, 82, 84, 86, 88, 90, 90, 91, 91, 93, 97, 105, 117, 132],
    [71, 75, 78, 81, 84, 86, 88, 90, 90, 91, 93, 96, 102, 112, 127, 143],
    [71, 74, 77, 80, 83, 85, 87, 89, 90, 91, 94, 99, 107, 121, 137, 155],
    [71, 73, 76, 79, 81, 83, 85, 88, 89, 92, 97, 105, 117, 133, 150, 168],
    [70, 72, 75, 77, 79, 81, 84, 86, 88, 93, 100, 111, 126, 145, 162, 183],
    [70, 72, 73, 75, 78, 80, 83, 86, 91, 98, 107, 122, 140, 159, 179, 199],
    [69, 71, 72, 74, 77, 79, 83, 87, 94, 104, 117, 134, 154, 175, 196, 216],
    [69, 70, 72, 74, 76, 79, 83, 90, 99, 112, 129, 149, 170, 192, 214, 233],
    [69, 70, 71, 74, 77, 80, 85, 93, 105, 122, 142, 164, 187, 211, 232, 249],
    [69, 70, 71, 74, 77, 81, 89, 100, 114, 134, 157, 181, 205, 229, 249, 264],
    [69, 70, 71, 74, 77, 83, 92, 106, 124, 146, 170, 195, 221, 244, 263, 274],
    [69, 70, 72, 74, 79, 85, 96, 113, 134, 158, 183, 209, 234, 256, 272, 281],
];
const ONE_SIDED_EDGEFILT_SAMPLE_COUNT: usize = 256;
/// Sum of the MI(148,168) edge-filter-active one-sided block (`[[u16; 16]; 16]`).
const ONE_SIDED_EDGEFILT_SAMPLE_SUM: u64 = 27_274;
/// FNV-1a-64 over the block (row-major, sample-major u16 LE).
const ONE_SIDED_EDGEFILT_FNV1A64: u64 = 0xfae6_cc1f_cfe6_6327;

/// Bit-exact verification of the §7.13.2.8 ZONE-1 ONE-SIDED IDIF luma leaf at
/// MI(148,168) (sample x[592,608) y[672,688)) whose §7.13.2.7 step-1 path runs the
/// §7.13.2.14 corner filter AND the §7.13.2.18 edge filter (`strength 3`) before
/// the prediction. This is the FIRST corner-filter + edge-filter-active one-sided
/// leaf the sink admits: the §7.13.2.15/16 per-edge `filterType` is derived from the
/// real decoded neighbour `is_smooth` modes, the §7.13.2.17 strength + §7.13.2.7
/// `numPx` drive `av2_filter_intra_edge`, and the corner blend rewrites the shared
/// corner from the reconstructed `LeftCol[0]`. Verified per-sample against the AVM
/// pre-filter oracle — a wrong filter/corner/numPx changes the strong gradient.
///
/// The same test also pins the §7.13.2.8 NON-SQUARE zone-1 one-sided IDIF leaf at
/// MI(220,44) (sample x[880,888) y[176,192)), an `8x16` `TX_8X16` block
/// (`log2_width == 3 != log2_height == 4`) — the FIRST non-square one-sided leaf the
/// sink admits after generalising the one-sided reconstructors + edge builders to
/// independent `Tx_Width`/`Tx_Height`. The §7.13.2.8 `maxBase == w + h - 1 == 23`,
/// the §7.13.2.1 `aboveLimit` keys on `w == 8` with the above-right capped at
/// `tx_size_wide_unit == mi_w == 2` MI units, and the active §7.13.2.18 edge filter
/// (`strength 1`) + §7.13.2.14 corner filter consume that full padded span. A wrong
/// non-square geometry, a `max_read`-bounded above-right undercount (mis-pad), or a
/// wrong wide-angle remap would shift the block's bottom-right ramp.
#[test]
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn ac0ej3_one_sided_edge_filter_corner_block_reconstructs_bit_exact_against_prefilter_oracle() {
    let sink = reconstruct_ac0ej3_sink();
    assert_luma_region_oracle(
        &sink,
        "MI(148,168) zone-1 one-sided edge-filter + corner IDIF",
        (592, 16),
        (672, 16),
        |x, y| ONE_SIDED_EDGEFILT_ORACLE[y - 672][x - 592],
        (
            ONE_SIDED_EDGEFILT_SAMPLE_COUNT,
            ONE_SIDED_EDGEFILT_SAMPLE_SUM,
            ONE_SIDED_EDGEFILT_FNV1A64,
        ),
    );
    assert_luma_region_oracle(
        &sink,
        "MI(220,44) non-square 8x16 zone-1 one-sided IDIF",
        (880, 8),
        (176, 16),
        |x, y| NONSQ_ZONE1_TALL_ORACLE[y - 176][x - 880],
        (
            NONSQ_ZONE1_TALL_SAMPLE_COUNT,
            NONSQ_ZONE1_TALL_SAMPLE_SUM,
            NONSQ_ZONE1_TALL_FNV1A64,
        ),
    );
}

/// The AVM pre-filter oracle samples (row-major, `[y][x]`) for the §7.13.2.8
/// NON-SQUARE zone-1 one-sided IDIF leaf at MI(220,44), x[880,888) y[176,192): an
/// `8x16` TALL `TX_8X16` block, `pAngle 81` (zone-1 above, `dx ==
/// Dr_Intra_Derivative[81] == 9`), `all_zero`, with the §7.13.2.18 edge filter
/// ACTIVE (`strength 1`, `|angleAbove| == 9`, `w + h == 24`) AND the §7.13.2.14
/// corner filter (`needAbove && needLeft && (w + h) >= 24`). The block is flat `68`
/// except its bottom-right ramp (69→74) where the projection reads the real
/// reconstructed above-right (capped at `tx_size_wide_unit == mi_w == 2` MI units,
/// `n_topright == 8` px) the active edge filter pads/smooths. This ASYMMETRIC ramp
/// is the signal that distinguishes the correct non-square `(w, h)` geometry — a
/// `max_read`-bounded above-right undercount would mis-pad the filtered edge and
/// shift the bottom rows. Derived offline from `/tmp/pref.yuv`.
#[rustfmt::skip]
const NONSQ_ZONE1_TALL_ORACLE: [[u16; 8]; 16] = [
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 68],
    [68, 68, 68, 68, 68, 68, 68, 69],
    [68, 68, 68, 68, 68, 68, 68, 69],
    [68, 68, 68, 68, 68, 68, 68, 70],
    [68, 68, 68, 68, 68, 68, 68, 71],
    [68, 68, 68, 68, 68, 68, 68, 72],
    [68, 68, 68, 68, 68, 68, 68, 73],
    [68, 68, 68, 68, 68, 68, 68, 73],
    [68, 68, 68, 68, 68, 68, 69, 74],
];
const NONSQ_ZONE1_TALL_SAMPLE_COUNT: usize = 128;
/// Sum of the MI(220,44) non-square `8x16` one-sided block (`[[u16; 8]; 16]`).
const NONSQ_ZONE1_TALL_SAMPLE_SUM: u64 = 8_732;
/// FNV-1a-64 over the block (row-major, sample-major u16 LE).
const NONSQ_ZONE1_TALL_FNV1A64: u64 = 0xc62f_e673_751d_4e4f;

/// ac0ej3 luma plane width (the §6.4.2 `1920x1080` 10-bit 4:2:0 frame).
const FULL_FRAME_WIDTH: usize = 1920;
/// ac0ej3 luma plane height.
const FULL_FRAME_HEIGHT: usize = 1080;

/// Loads the AVM pre-filter oracle luma plane (`SPLOT_AC0EJ3_PREFILTER_YUV` or
/// `/tmp/pref.yuv`): the first `1920 * 1080` u16 little-endian samples of the
/// planar 10-bit 4:2:0 dump (`--dump-prefiltered`). Returns the row-major luma
/// samples. Panics with a clear message when the file is missing or too short, so a
/// stale / unregenerated oracle is loud rather than silently mis-comparing.
fn load_prefilter_oracle_luma() -> Vec<u16> {
    let path =
        std::env::var("SPLOT_AC0EJ3_PREFILTER_YUV").unwrap_or_else(|_| "/tmp/pref.yuv".to_string());
    let bytes = std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "read AVM pre-filter oracle at {path} (set SPLOT_AC0EJ3_PREFILTER_YUV; \
             regenerate with avm inspect --dump-prefiltered): {err}"
        )
    });
    let luma_samples = FULL_FRAME_WIDTH * FULL_FRAME_HEIGHT;
    assert!(
        bytes.len() >= luma_samples * 2,
        "oracle {path} is {} bytes, need at least {} for the {FULL_FRAME_WIDTH}x{FULL_FRAME_HEIGHT} luma plane",
        bytes.len(),
        luma_samples * 2
    );
    bytes
        .chunks_exact(2)
        .take(luma_samples)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// DIAGNOSTIC harness (the maintainer's full-reconstruction pivot): reconstructs
/// EVERY luma block in decode order with the gates dropped, then per-sample diffs the
/// whole `1920x1080` luma plane against the AVM pre-filter oracle. Reports the
/// bit-exact %, the covered (written) %, and the FIRST decode-order block whose
/// samples diverge (MI / xy / mode / size / first splot-vs-oracle sample). Drives the
/// iterative FIND-AND-FIX loop; it does NOT change the shipped (gated) reconstruction
/// — the 16 oracle-pin tests above keep the 791,776-sample region bit-exact.
///
/// Gated behind the `SPLOT_AC0EJ3_FULL_RECON` env var (in addition to `#[ignore]`) so
/// a casual `--ignored` run that has no regenerated oracle does not fail; set the var
/// to run it: `SPLOT_AC0EJ3_FULL_RECON=1 cargo test ... -- --ignored full_recon`.
#[test]
#[ignore = "diagnostic; set SPLOT_AC0EJ3_FULL_RECON=1 and provide the AVM pre-filter oracle (/tmp/pref.yuv)"]
fn ac0ej3_full_decode_order_reconstruction_differs_against_prefilter_oracle() {
    if std::env::var("SPLOT_AC0EJ3_FULL_RECON").is_err() {
        eprintln!("SPLOT_AC0EJ3_FULL_RECON unset; skipping the full-recon differential");
        return;
    }
    let sink = reconstruct_ac0ej3_full_recon_sink();
    let oracle = load_prefilter_oracle_luma();

    let oracle_at = |x: usize, y: usize| oracle[y * FULL_FRAME_WIDTH + x];

    let mut written_total = 0usize;
    let mut written_exact = 0usize;
    let mut first_mismatch: Option<(usize, usize, u16, u16)> = None;
    for leaf in sink.full_recon_luma_log() {
        if !leaf.written {
            continue;
        }
        for dy in 0..leaf.height {
            for dx in 0..leaf.width {
                let (x, y) = (leaf.x + dx, leaf.y + dy);
                if x >= FULL_FRAME_WIDTH || y >= FULL_FRAME_HEIGHT {
                    continue;
                }
                let got = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
                let want = oracle_at(x, y);
                written_total += 1;
                if got == want {
                    written_exact += 1;
                } else if first_mismatch.is_none() {
                    first_mismatch = Some((x, y, got, want));
                }
            }
        }
    }

    let mut first_block: Option<(usize, usize)> = None;
    let mut first_clean_block: Option<(usize, usize)> = None;
    let mut first_unwritten: Option<usize> = None;
    let mut covered_samples = 0usize;
    let mut unwired: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for (idx, leaf) in sink.full_recon_luma_log().iter().enumerate() {
        if !leaf.written {
            *unwired.entry(leaf.mode).or_default() += 1;
            if first_unwritten.is_none() {
                first_unwritten = Some(idx);
            }
            continue;
        }
        let mut block_mismatch = 0usize;
        for dy in 0..leaf.height {
            for dx in 0..leaf.width {
                let (x, y) = (leaf.x + dx, leaf.y + dy);
                if x >= FULL_FRAME_WIDTH || y >= FULL_FRAME_HEIGHT {
                    continue;
                }
                covered_samples += 1;
                let got = sink.reconstructed_sample(PlaneId::Y, x, y).unwrap();
                if got != oracle_at(x, y) {
                    block_mismatch += 1;
                }
            }
        }
        if block_mismatch > 0 && first_block.is_none() {
            first_block = Some((idx, block_mismatch));
        }
        if block_mismatch > 0 && first_clean_block.is_none() {
            let neighbours_exact = |leaf: &super::wienerns_lr::FullReconLumaLeaf| {
                let span = leaf.width + leaf.height;
                let sample_ok = |x: usize, y: usize| {
                    x >= FULL_FRAME_WIDTH
                        || y >= FULL_FRAME_HEIGHT
                        || sink.reconstructed_sample(PlaneId::Y, x, y).unwrap() == oracle_at(x, y)
                };
                let above_ok = leaf.y == 0
                    || (0..span).all(|dx| {
                        leaf.x + dx >= FULL_FRAME_WIDTH || sample_ok(leaf.x + dx, leaf.y - 1)
                    });
                let left_ok = leaf.x == 0
                    || (0..span).all(|dy| {
                        leaf.y + dy >= FULL_FRAME_HEIGHT || sample_ok(leaf.x - 1, leaf.y + dy)
                    });
                let corner_ok = leaf.x == 0 || leaf.y == 0 || sample_ok(leaf.x - 1, leaf.y - 1);
                above_ok && left_ok && corner_ok
            };
            if neighbours_exact(leaf) {
                first_clean_block = Some((idx, block_mismatch));
            }
        }
    }

    let frame_samples = FULL_FRAME_WIDTH * FULL_FRAME_HEIGHT;
    let pct = |num: usize, den: usize| {
        if den == 0 {
            0.0
        } else {
            100.0 * num as f64 / den as f64
        }
    };
    eprintln!("==== ac0ej3 FULL DECODE-ORDER LUMA RECONSTRUCTION vs AVM pre-filter oracle ====");
    eprintln!(
        "frame {FULL_FRAME_WIDTH}x{FULL_FRAME_HEIGHT} = {frame_samples} luma samples; \
         leaves logged = {}",
        sink.full_recon_luma_log().len()
    );
    eprintln!(
        "written (predicted) samples = {written_total} ({:.2}% of frame); \
         bit-exact = {written_exact} ({:.2}% of written, {:.2}% of frame)",
        pct(written_total, frame_samples),
        pct(written_exact, written_total),
        pct(written_exact, frame_samples),
    );
    eprintln!("covered samples (re-counted) = {covered_samples}");
    if !unwired.is_empty() {
        eprintln!("unwired (fill) leaves by mode:");
        for (mode, n) in &unwired {
            eprintln!("    {mode}: {n} leaves");
        }
    }
    match (first_block, first_mismatch) {
        (Some((idx, block_mismatch)), Some((mx, my, got, want))) => {
            let leaf = sink.full_recon_luma_log()[idx];
            eprintln!(
                "FIRST decode-order mismatch: leaf #{idx} {} {}x{} MI({},{}) x[{},{}) y[{},{}) — \
                 {block_mismatch} mismatched samples; first at ({mx},{my}) splot={got} oracle={want}",
                leaf.mode,
                leaf.width,
                leaf.height,
                leaf.mi_col,
                leaf.mi_row,
                leaf.x,
                leaf.x + leaf.width,
                leaf.y,
                leaf.y + leaf.height,
            );
        }
        _ => {
            eprintln!("NO decode-order mismatch found among written leaves (full bit-exact!)");
        }
    }
    if let Some(idx) = first_unwritten {
        let leaf = sink.full_recon_luma_log()[idx];
        eprintln!(
            "FIRST decode-order UNWRITTEN (fill root) leaf: #{idx} {} {}x{} MI({},{}) \
             x[{},{}) y[{},{})",
            leaf.mode,
            leaf.width,
            leaf.height,
            leaf.mi_col,
            leaf.mi_row,
            leaf.x,
            leaf.x + leaf.width,
            leaf.y,
            leaf.y + leaf.height,
        );
    }
    if let Some((idx, n)) = first_clean_block {
        let leaf = sink.full_recon_luma_log()[idx];
        eprintln!(
            "FIRST clean-neighbour predictor mismatch: #{idx} {} {}x{} MI({},{}) \
             x[{},{}) y[{},{}) — {n} mismatched samples (neighbours bit-exact, so a real \
             predictor bug)",
            leaf.mode,
            leaf.width,
            leaf.height,
            leaf.mi_col,
            leaf.mi_row,
            leaf.x,
            leaf.x + leaf.width,
            leaf.y,
            leaf.y + leaf.height,
        );
    }
    eprintln!("================================================================================");
}
