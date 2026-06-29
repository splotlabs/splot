// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! ac0ej3 general-intra reconstruction bridge.
//!
//! Feature tracking: `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS`.
//!
//! The selectable transform-record walk
//! ([`super::tx_records::derive_wienerns_lr_selectable_transform_record_handoff`])
//! already decodes every general-intra block's §5.20.7.27 coefficients into a
//! populated [`LumaCoeffBlock`] using the SAME `decode_general_intra_plane_coeffs`
//! that `general_intra.rs` consumes, then discards the `quant` array after
//! recording `eob` / `skip_flag`. This module captures those decoded coefficients
//! and reconstructs the verified NON-IntrABC DC subset into a
//! [`CurrentFrameWorkspace`] in decode order, reusing the existing
//! [`crate::runtime_minimal_recon`] §7.13.2 / §7.14.4 / §7.15.4 / §7.14.3
//! primitives — exactly the prediction→residual→write pattern `general_intra.rs`
//! uses, but driven from the live ac0ej3 walk instead of a synthetic fixture.
//!
//! The bridge also reconstructs the verified §7.13.3.18 IntrABC subset: a `skip`
//! block's displaced copy IS the reconstruction, and a NON-skip integer-DV block's
//! displaced copy is the §7.13.2 prediction onto which each §5.20.7.27 residual
//! transform leaf adds its decoded residual (the same §7.14.4 / §7.15.4 / §7.14.3
//! path the DC / cardinal intra leaves use, over the IntrABC predictor).
//!
//! The bridge is a TEST instrument: the public `splot decode` path runs the walk
//! WITHOUT a sink, so it still fails closed at the §7.20.4 unpopulated-samples gate
//! and emits no frame. A region-verification test attaches a sink, runs the whole
//! walk, and asserts the populated workspace region is bit-exact against the AVM
//! pre-filter reconstruction oracle.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraRectBlockSize, PlaneId, PlaneRect, ReconSample,
};

use crate::Result;
#[cfg(test)]
use crate::runtime_minimal_recon::new_general_intra_workspace;
use crate::runtime_minimal_recon::{
    reconstruct_general_intra_block_rect_into,
    reconstruct_general_intra_cardinal_neighbour_block_into,
    reconstruct_general_intra_luma_paeth_neighbour_block_into,
    reconstruct_general_intra_one_sided_left_neighbour_block_into,
    reconstruct_general_intra_one_sided_neighbour_block_into,
    reconstruct_intrabc_block_residual_rect_into,
};
use crate::tile_payload::{
    IntraYMode, LumaCoeffBlock, SupportedChromaMode, SupportedDirectionalLumaMode,
};

use super::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use splot_core::span::ByteOffset;

/// AV2 §3 `MI_SIZE`: one mode-info unit spans four samples.
const MI_SIZE: usize = 4;

/// Per-block reconstruction parameters handed to the [`WienerNsLrReconSink`] as
/// the selectable walk decodes each general-intra block's coefficients. The luma
/// and chroma modes gate the verified DC subset, `qindex` is the §5.20.6.5 delta-Q
/// per-block dequant index, `luma_use_tcq` carries the §7.14.4 luma TCQ `dqDenom`
/// term, and `fsc_mode` gates out FSC leaves (the reconstruction primitive assumes
/// the non-FSC DCT_DCT path).
#[derive(Clone, Copy, Debug)]
pub(in crate::runtime_minimal) struct SelectableReconContext {
    pub(in crate::runtime_minimal) leaf_y_mode: Option<IntraYMode>,
    /// The leaf's resolved §7.13.2.8 directional-angle luma mode, or `None` for a
    /// non-directional (DC / SMOOTH / PAETH) leaf or any directional leaf with a
    /// non-zero §5.20.5.3 `AngleDeltaY` (the upstream `supported_directional_luma`
    /// already folds the `AngleDeltaY == 0` gate in). The sink admits only the
    /// CARDINAL subset (`V_PRED` pAngle 90 / `H_PRED` pAngle 180) — a pure §7.13.2.8
    /// step-4/step-5 sample copy with no IDIF, no corner, and no `useIBP` (which
    /// §7.13.2.7 gates on `pAngle < 90 || pAngle > 180`, excluding both cardinals).
    pub(in crate::runtime_minimal) directional_luma: Option<SupportedDirectionalLumaMode>,
    /// The leaf's §5.20.5.5 `MrlIndex` (the multi-reference-line distance). `0` for
    /// the immediate edge; `> 0` selects a farther reference line. The cardinal
    /// recon primitive is the `MrlIndex == 0` immediate-edge copy, so the sink
    /// DEFERS a cardinal leaf whose `mrl_index > 0` (it would otherwise copy the
    /// adjacent samples instead of the selected MRL reference line).
    pub(in crate::runtime_minimal) mrl_index: u8,
    /// The leaf's §5.20.5.3 `AngleDeltaY` (the signed angle-delta count, range
    /// `-MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA`). The sink combines it with the raw
    /// `leaf_y_mode`'s §9.2 `Mode_To_Angle` and `Mrl_Index_To_Delta[mrl_index]`
    /// (and the §5.20.7.29 wide-angle remap) to recover the §7.13.2.8 `pAngle` for
    /// the one-sided angular admission — UNLIKE `directional_luma`, which the
    /// upstream `supported_directional_luma` zeroes out for any non-zero
    /// `AngleDeltaY`. Carried separately so the sink can admit a one-sided angle
    /// whose `AngleDeltaY != 0` (e.g. the §9.2 D67/D203 seeds).
    pub(in crate::runtime_minimal) angle_delta_y: i8,
    pub(in crate::runtime_minimal) chroma_mode: Option<SupportedChromaMode>,
    pub(in crate::runtime_minimal) qindex: u32,
    pub(in crate::runtime_minimal) luma_use_tcq: bool,
    pub(in crate::runtime_minimal) fsc_mode: bool,
    /// Whether this leaf is a §5.20.5.3 `use_intrabc` block. An IntrABC leaf's
    /// §7.13.2 PREDICTION is the §7.13.3.18 displaced `CurrFrame` copy
    /// [`WienerNsLrReconSink::reconstruct_intrabc_block`] performs inside
    /// `read_intrabc_info`, BEFORE the residual leaves run — NOT a §7.13.2 intra
    /// prediction (an IntrABC leaf's `leaf_y_mode` is a placeholder `DC_PRED`,
    /// §5.20.5.3 reads no intra Y mode). So the residual path must NOT run its flat
    /// DC/cardinal intra prediction for an IntrABC leaf. Instead, for a NON-skip
    /// IntrABC block, the residual leaf adds its §5.20.7.27 residual onto the copied
    /// predictor ([`WienerNsLrReconSink::reconstruct_luma_transform`]'s `is_intrabc`
    /// branch); a `skip` IntrABC block carries no residual leaf (its copy already marked
    /// coverage).
    pub(in crate::runtime_minimal) is_intrabc: bool,
}

/// Reconstructs the verified NON-IntrABC general-intra DC subset of the ac0ej3
/// key frame into an owned [`CurrentFrameWorkspace`], in selectable-walk decode
/// order. Holding the workspace across the walk (including the walk's eventual
/// fail-closed IntrABC rejection) lets the region-verification test read the
/// samples reconstructed before the rejection point.
///
/// The sink is gated to the proven subset and DEFERS anything it cannot prove
/// bit-exact (over-rejecting is safe; a confident-wrong workspace sample is the
/// cardinal sin). A luma transform is reconstructed only when ALL hold: its leaf
/// signalled `DC_PRED`; the residual is the proven primitive kind (an `all_zero`
/// flat-DC block, or a square non-`all_zero` block with no §5.20.7.29 IST
/// secondary transform and no FSC — the rectangular-residual inverse transform is
/// not yet proven bit-exact); the frame carries no per-plane quantizer delta or
/// quantizer matrix (the primitive dequantizes with zero `QuantizerDeltas`); and
/// the §7.13.2 DC-prediction edges it reads are either genuinely off-frame
/// (spec-default predictor) or already reconstructed by this sink (never a
/// workspace fill value standing in for a deferred neighbour). A chroma group is
/// reconstructed only when the resolved §5.20.5.3 chroma mode is `DC_PRED` and the
/// same quant / edge-coverage guards hold. Everything else stays UNRECONSTRUCTED.
///
/// `reconstructed` is the per-plane MI-unit coverage map (`true` where the sink
/// wrote spec-correct samples) used both to gate DC-edge reads and to report the
/// verified region. `reconstructed_luma_4x4` / `reconstructed_chroma_4x4` count the
/// 4x4 units actually written.
pub(in crate::runtime_minimal) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
    bit_depth: BitDepth,
    /// Whether the frame's dequant matches the primitive's zero-delta assumption
    /// (no per-plane DC/AC quantizer delta, no quantizer matrix). When `false` the
    /// sink reconstructs nothing.
    quant_reconstructable: bool,
    /// The §5.3 `enable_ibp` sequence flag. A DC_PRED leaf that is not 4x4 (and,
    /// for chroma, not `UV_CFL_PRED`) blends its §7.13.2.10 flat DC edge rows /
    /// columns toward the reconstructed neighbours via the §7.13.2.12 IBP DC
    /// modifier when this is set. (ac0ej3's sequence enables IBP.)
    enable_ibp: bool,
    /// The §5.4.5 `enable_intra_edge_filter` sequence flag. When set, the §7.13.2.7
    /// intra edge-filter / corner-filter step runs before the §7.13.2.8 single
    /// directional prediction for any non-cardinal angle (`pAngle != 90 && pAngle
    /// != 180`, `MrlIndex == 0`). The sink models only the §7.13.2.8 prediction
    /// over the UNFILTERED edge, so it admits a one-sided angular leaf with this
    /// flag set ONLY when the §7.13.2.17 edge-filter strength is `0` (a genuine
    /// no-op: `av2_filter_intra_edge` returns early) AND no §7.13.2.7 corner filter
    /// applies; otherwise the filtered edge is unmodelled and the leaf DEFERS.
    /// (ac0ej3's sequence sets this flag.)
    enable_intra_edge_filter: bool,
    /// Per-plane MI-unit coverage (`coverage[plane]`, row-major over the plane's MI
    /// grid): luma plane 0, chroma U plane 1, chroma V plane 2. U and V are tracked
    /// SEPARATELY — a reconstructed U must not let a deferred V block pass the
    /// DC-edge guard (4:2:0 U and V share MI dimensions but not reconstruction
    /// state). `true` where the sink has written spec-correct samples.
    coverage: [PlaneCoverage; 3],
    reconstructed_luma_4x4: usize,
    reconstructed_chroma_4x4: usize,
    /// Luma TARGET rectangles of NON-skip §7.13.3.18 IntrABC blocks whose displaced
    /// copy this sink has already written into the workspace as the §7.13.2
    /// prediction, but whose §5.20.7.27 residual is added later by the per-transform
    /// [`Self::reconstruct_luma_transform`] leaves (in decode order, AFTER this
    /// prelude copy). A residual leaf is admitted as an IntrABC residual-add ONLY
    /// when its transform rect lies within one of these rects: that proves the copied
    /// prediction underneath it is the spec-correct displaced source (the
    /// `reconstruct_intrabc_block` integer-DV / source-covered gate already held), so
    /// the leaf adds its residual onto a real predictor rather than a fill value. A
    /// SKIP IntrABC block (no residual) marks its coverage at copy time and records
    /// nothing here.
    pending_intrabc_predictions: Vec<PlaneRect>,
}

/// Row-major MI-unit reconstruction coverage for one plane grid.
struct PlaneCoverage {
    cols: usize,
    rows: usize,
    covered: Vec<bool>,
}

impl PlaneCoverage {
    #[cfg(test)]
    fn new(width_samples: usize, height_samples: usize) -> Self {
        let cols = width_samples.div_ceil(MI_SIZE);
        let rows = height_samples.div_ceil(MI_SIZE);
        Self {
            cols,
            rows,
            covered: vec![false; cols.saturating_mul(rows)],
        }
    }

    /// Whether the MI unit at `(mi_col, mi_row)` is off this plane's grid.
    const fn off_grid(&self, mi_col: usize, mi_row: usize) -> bool {
        mi_col >= self.cols || mi_row >= self.rows
    }

    fn is_covered(&self, mi_col: usize, mi_row: usize) -> bool {
        if self.off_grid(mi_col, mi_row) {
            return false;
        }
        self.covered
            .get(mi_row * self.cols + mi_col)
            .copied()
            .unwrap_or(false)
    }

    /// Marks the block's MI footprint as reconstructed and returns the number of
    /// IN-FRAME MI cells (== 4x4 luma units) it covered. A transform overhanging a
    /// partial frame-edge superblock marks only its in-frame cells (the off-grid
    /// overhang is dropped, mirroring the in-frame-only sample write), so the count
    /// the caller adds to its 4x4 tally stays equal to the canonical covered region.
    fn mark(&mut self, mi_col: usize, mi_row: usize, mi_w: usize, mi_h: usize) -> usize {
        let mut marked = 0usize;
        for r in mi_row..mi_row.saturating_add(mi_h) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                // `off_grid` rejects a column at/past the row stride so a wide block's
                // overhang cannot alias into the next row's cells.
                if !self.off_grid(c, r)
                    && let Some(slot) = self.covered.get_mut(r * self.cols + c)
                {
                    *slot = true;
                    marked += 1;
                }
            }
        }
        marked
    }

    /// Whether EVERY MI unit of the `mi_w` x `mi_h` block at `(mi_col, mi_row)` is
    /// already reconstructed by the sink. Used to gate an IntrABC copy: a source
    /// rectangle may be copied only when all of its samples come from spec-correct
    /// reconstruction, never a workspace fill value standing in for a deferred block.
    /// A block extending off this plane's grid is not fully covered.
    fn region_fully_covered(&self, mi_col: usize, mi_row: usize, mi_w: usize, mi_h: usize) -> bool {
        for r in mi_row..mi_row.saturating_add(mi_h) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if !self.is_covered(c, r) {
                    return false;
                }
            }
        }
        true
    }
}

/// AV2 §5 `ANGLE_STEP`: degrees of angle change per unit `AngleDeltaY`.
const ANGLE_STEP: i32 = 3;
/// AV2 §5.20.7.27 `Mrl_Index_To_Delta[4]` (the multi-reference-line angle nudge).
const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
/// AV2 §5.20.7.29 WAIP wide-angle remap thresholds (`WAIP_WH_RATIO_*_THRES`), from
/// the §3 symbol table.
const WAIP_WH_RATIO_2_THRES: i32 = 61;
const WAIP_WH_RATIO_4_THRES: i32 = 73;
const WAIP_WH_RATIO_8_THRES: i32 = 82;
const WAIP_WH_RATIO_16_THRES: i32 = 86;

/// AV2 §5.20.7.29 `wide_angle_mapping(mode, w, h, pAngle)`: for `is_inter == 0`,
/// remaps a directional `pAngle` whose block is sufficiently tall (add 180) or
/// wide (subtract 180) so the projection points into the longer edge. Returns the
/// (possibly remapped) `pAngle`. `w`/`h` are the §5.20.5.3 `Tx_Width`/`Tx_Height`
/// (transform dimensions in samples), matching AVM `wide_angle_mapping`
/// (`reconintra.h`) which keys on `tx_size_wide`/`tx_size_high`. A square block is
/// never remapped.
fn wide_angle_mapping(w: u32, h: u32, p_angle: i32) -> i32 {
    let w = w as i32;
    let h = h as i32;
    if (h == 2 * w && p_angle < WAIP_WH_RATIO_2_THRES)
        || (h == 4 * w && p_angle < WAIP_WH_RATIO_4_THRES)
        || (h == 8 * w && p_angle < WAIP_WH_RATIO_8_THRES)
        || (h == 16 * w && p_angle < WAIP_WH_RATIO_16_THRES)
    {
        return 180 + p_angle;
    }
    if (w == 2 * h && p_angle > 270 - WAIP_WH_RATIO_2_THRES)
        || (w == 4 * h && p_angle > 270 - WAIP_WH_RATIO_4_THRES)
        || (w == 8 * h && p_angle > 270 - WAIP_WH_RATIO_8_THRES)
        || (w == 16 * h && p_angle > 270 - WAIP_WH_RATIO_16_THRES)
    {
        return p_angle - 180;
    }
    p_angle
}

/// AV2 §7.13.2.17 intra edge filter strength selection process. Returns the
/// edge-filter strength `0..=3` for a `w` x `h` transform, `filter_type` (0 or 1,
/// from §7.13.2.15/16 — `1` when the relevant neighbour uses a smooth mode), and
/// `delta` (the §7.13.2.7 `angleAbove = pAngle - 90` / `angleLeft = pAngle - 180`).
/// Strength `0` means `av2_filter_intra_edge` is a no-op, so the §7.13.2.8
/// prediction over the UNFILTERED edge is bit-exact. Transcribed VERBATIM from the
/// committed spec mirror `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-17`.
fn intra_edge_filter_strength(w: u32, h: u32, filter_type: u8, delta: i32) -> u8 {
    let d = delta.unsigned_abs();
    let blk_wh = w + h;
    let mut strength = 0u8;
    if filter_type == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
                strength = 1;
            }
        } else if blk_wh <= 16 {
            if d >= 40 {
                strength = 1;
            }
        } else if blk_wh <= 24 {
            if d >= 8 {
                strength = 1;
            }
            if d >= 16 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else if blk_wh <= 32 {
            strength = 1;
            if d >= 4 {
                strength = 2;
            }
            if d >= 32 {
                strength = 3;
            }
        } else {
            strength = 3;
        }
    } else if blk_wh <= 8 {
        if d >= 40 {
            strength = 1;
        }
        if d >= 64 {
            strength = 2;
        }
    } else if blk_wh <= 16 {
        if d >= 20 {
            strength = 1;
        }
        if d >= 48 {
            strength = 2;
        }
    } else if blk_wh <= 24 {
        if d >= 4 {
            strength = 3;
        }
    } else {
        strength = 3;
    }
    strength
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    /// Allocates a sink whose workspace is sized to the ac0ej3 frame (a positive
    /// multiple of 64 in both dimensions for the gated tier), with 4:2:0 chroma
    /// derived internally. `T` matches the active sequence bit depth (§6.4.1):
    /// `u16` for the 10-bit ac0ej3 stream. Only the test-only sink driver
    /// constructs a sink; the public decode path threads `None`.
    #[cfg(test)]
    pub(in crate::runtime_minimal) fn new(
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
        quant_reconstructable: bool,
        enable_ibp: bool,
        enable_intra_edge_filter: bool,
    ) -> Result<Self> {
        // 4:2:0 chroma planes are half the luma dimensions in each axis.
        let chroma_width = luma_width.div_ceil(2);
        let chroma_height = luma_height.div_ceil(2);
        Ok(Self {
            workspace: new_general_intra_workspace::<T>(luma_width, luma_height, bit_depth)?,
            bit_depth,
            quant_reconstructable,
            enable_ibp,
            enable_intra_edge_filter,
            coverage: [
                PlaneCoverage::new(luma_width, luma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
            ],
            reconstructed_luma_4x4: 0,
            reconstructed_chroma_4x4: 0,
            pending_intrabc_predictions: Vec::new(),
        })
    }

    /// The coverage-grid index for a plane: luma 0, chroma U 1, chroma V 2. U and
    /// V are SEPARATE so a reconstructed U cannot satisfy a deferred V's DC-edge
    /// guard (4:2:0 U and V share MI dimensions but not reconstruction state).
    const fn coverage_index(plane_id: PlaneId) -> usize {
        match plane_id {
            PlaneId::Y => 0,
            PlaneId::U => 1,
            PlaneId::V => 2,
        }
    }

    /// Whether the transform rect `[x, x+width) x [y, y+height)` lies entirely
    /// within a NON-skip §7.13.3.18 IntrABC block whose displaced-copy prediction
    /// this sink already wrote into the workspace (recorded in
    /// [`Self::pending_intrabc_predictions`]). When `true`, the copied samples under
    /// the transform are the spec-correct displaced predictor (the
    /// `reconstruct_intrabc_block` integer-DV / source-covered gate held at copy
    /// time), so a residual leaf may add its §5.20.7.27 residual onto them. A leaf
    /// whose rect is NOT inside any pending prediction (the copy was deferred —
    /// fractional DV, uncovered source, or non-reconstructable quant) DEFERS, never
    /// adding a residual onto a fill value.
    fn rect_within_pending_intrabc_prediction(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> bool {
        let (Some(right), Some(bottom)) = (x.checked_add(width), y.checked_add(height)) else {
            return false;
        };
        self.pending_intrabc_predictions.iter().any(|rect| {
            let (Some(rect_right), Some(rect_bottom)) = (
                rect.x().checked_add(rect.width()),
                rect.y().checked_add(rect.height()),
            ) else {
                return false;
            };
            x >= rect.x() && y >= rect.y() && right <= rect_right && bottom <= rect_bottom
        })
    }

    /// Whether the §7.13.2.12 IBP DC modifier is invoked for a DC_PRED block of the
    /// given §7.15.4 transform dimensions on this plane.
    ///
    /// Per §7.13.2 (the prediction-dispatch IBP gate) the modifier runs when
    /// `enable_ibp == 1`, `useDip == 0`, `mode == DC_PRED`, `!(w == 4 && h == 4)`,
    /// and `plane == 0 || UVMode != UV_CFL_PRED`. The caller has already established
    /// `mode == DC_PRED` (the sink admits only DC luma / chroma) and `useDip == 0`
    /// (DIP is deferred), and the sink never admits a `UV_CFL_PRED` chroma leaf, so
    /// here it reduces to `enable_ibp && !(w == 4 && h == 4)`.
    const fn ibp_dc_applies(&self, log2_width: u32, log2_height: u32) -> bool {
        self.enable_ibp && !(log2_width == 2 && log2_height == 2)
    }

    /// Whether every §7.13.2 DC-prediction edge MI unit a block at `(mi_col,
    /// mi_row)` of `mi_w` x `mi_h` MI units reads is safe to predict from: the
    /// above row (`mi_row - 1`) and left column (`mi_col - 1`) must each be either
    /// genuinely off-frame (the spec default predictor is correct there) or already
    /// reconstructed by this sink. A neighbour that EXISTS on-grid but was deferred
    /// (still the workspace fill value) makes the prediction wrong, so the block is
    /// deferred. Frame-origin / frame-edge blocks with no on-grid neighbour pass.
    fn dc_edges_reconstructed(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        // Above row: the MI units directly above the block's top edge.
        if let Some(above) = mi_row.checked_sub(1) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if !coverage.off_grid(c, above) && !coverage.is_covered(c, above) {
                    return false;
                }
            }
        }
        // Left column: the MI units directly left of the block's left edge.
        if let Some(left) = mi_col.checked_sub(1) {
            for r in mi_row..mi_row.saturating_add(mi_h) {
                if !coverage.off_grid(left, r) && !coverage.is_covered(left, r) {
                    return false;
                }
            }
        }
        true
    }

    /// Whether the §7.13.2.8 cardinal prediction a block at `(mi_col, mi_row)` of
    /// `mi_w` x `mi_h` MI units reads is reconstructable bit-exact. A cardinal mode
    /// reads exactly ONE edge — `H_PRED` (pAngle 180) the left column (`pred[i][j] =
    /// LeftCol[i]`), `V_PRED` (pAngle 90) the above row (`pred[i][j] = AboveRow[j]`)
    /// — with no corner, no IDIF, and no `useIBP`.
    ///
    /// This admits two cases:
    /// * the REAL edge: every MI unit of the read edge (the left column `mi_col - 1`
    ///   for H, the above row `mi_row - 1` for V) exists on-grid AND is reconstructed
    ///   by this sink;
    /// * the §7.13.2.1 single-neighbour FALLBACK: the read edge is off-grid
    ///   (`haveAbove == 0` for V at `mi_row == 0`, `haveLeft == 0` for H at
    ///   `mi_col == 0`) but the ORTHOGONAL neighbour is reconstructed, so §7.13.2.1
    ///   synthesizes the missing edge as a flat repeat of one orthogonal sample
    ///   (`AboveRow[i] = CurrFrame[y][x-1]` for V; `LeftCol[i] = CurrFrame[y-1][x]`
    ///   for H — bit-exact, as the wrapper performs).
    ///
    /// It still DEFERS the §7.13.2.1 NO-neighbour midpoint fallback (both edges
    /// off-grid, e.g. the frame-origin block): that emits a synthetic midpoint, not a
    /// neighbour copy, and is a separate unmodelled path.
    fn cardinal_edge_reconstructed(
        &self,
        direction: IntraCardinalDirection,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        let above_row_covered = |row_above: usize| {
            (mi_col..mi_col.saturating_add(mi_w))
                .all(|c| !coverage.off_grid(c, row_above) && coverage.is_covered(c, row_above))
        };
        let left_col_covered = |col_left: usize| {
            (mi_row..mi_row.saturating_add(mi_h))
                .all(|r| !coverage.off_grid(col_left, r) && coverage.is_covered(col_left, r))
        };
        match direction {
            // H_PRED reads the left column `mi_col - 1`. Off-grid left
            // (`mi_col == 0`): §7.13.2.1 synthesizes `LeftCol` from the above row
            // (`mi_row - 1`) when that is reconstructed.
            IntraCardinalDirection::Horizontal => match mi_col.checked_sub(1) {
                Some(left) => left_col_covered(left),
                None => match mi_row.checked_sub(1) {
                    Some(above) => above_row_covered(above),
                    None => false,
                },
            },
            // V_PRED reads the above row `mi_row - 1`. Off-grid above
            // (`mi_row == 0`): §7.13.2.1 synthesizes `AboveRow` from the left column
            // (`mi_col - 1`) when that is reconstructed.
            IntraCardinalDirection::Vertical => match mi_row.checked_sub(1) {
                Some(above) => above_row_covered(above),
                None => match mi_col.checked_sub(1) {
                    Some(left) => left_col_covered(left),
                    None => false,
                },
            },
        }
    }

    /// Whether the §7.13.2.2 PAETH prediction a block at `(mi_col, mi_row)` of
    /// `mi_w` x `mi_h` MI units reads is reconstructable bit-exact. PAETH reads the
    /// §7.13.2.1 above row `AboveRow[0..w)`, the left column `LeftCol[0..h)`, AND the
    /// shared corner `AboveRow[-1]`. The sink admits PAETH only in the fully-proven
    /// `haveAbove == 1 && haveLeft == 1` config, where the corner is the real
    /// reconstructed diagonal sample `CurrFrame[plane][y-1][x-1]` (no §7.13.2.1
    /// single-sided edge synthesis — the AVM oracle shows PAETH does not match the
    /// naive single-neighbour fallback, so those configs DEFER). This requires EVERY
    /// MI unit of the above row (`mi_row - 1`), the left column (`mi_col - 1`), AND
    /// the diagonal corner unit `(mi_col - 1, mi_row - 1)` to exist on-grid and be
    /// reconstructed by this sink.
    fn paeth_neighbours_reconstructed(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        let (Some(above), Some(left)) = (mi_row.checked_sub(1), mi_col.checked_sub(1)) else {
            // A frame-top or frame-left block has no real above/left neighbour, so
            // the corner-bearing two-sided config does not hold; defer.
            return false;
        };
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        // Above row: every MI unit directly above the block's top edge.
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return false;
        }
        // Left column: every MI unit directly left of the block's left edge.
        if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r)) {
            return false;
        }
        // The §7.13.2.1 corner unit `AboveRow[-1] = CurrFrame[plane][y-1][x-1]`.
        covered(left, above)
    }

    /// Attempts to reconstruct a §7.13.2.8 ONE-SIDED angular luma leaf (zone-1
    /// `pAngle < 90`, reads the above row + above-right; zone-3 `pAngle > 180`,
    /// reads the left column + below-left). Returns `Ok(true)` when the leaf was
    /// reconstructed bit-exact, `Ok(false)` when it was DEFERRED (the caller then
    /// falls through to the `_ => Ok(())` defer). NEVER writes an unproven
    /// prediction.
    ///
    /// The §5.20.5.3 `pAngle` is recovered from the raw directional `mode`'s §9.2
    /// `Mode_To_Angle`, `angle_delta_y`, and `Mrl_Index_To_Delta[mrl_index]`, then
    /// the §5.20.7.29 `wide_angle_mapping` remap (for `is_inter == 0`). A leaf is
    /// ADMITTED only when ALL hold (otherwise DEFER):
    /// * the recovered `pAngle` is one-sided (`0 < pAngle < 90` or
    ///   `180 < pAngle < 270`) with a §9.2 derivative entry;
    /// * the transform is SQUARE (`log2_width == log2_height`) — the one-sided
    ///   reconstructor is square-only;
    /// * `mrl_index == 0` (the IDIF edge is the immediate reference line);
    /// * `useIBP == 0` (§7.13.2.7 gates the IBP secondary blend on `applyIbp &&
    ///   even angleDelta && plane 0 && one-sided pAngle && MrlIndex == 0`; the blend
    ///   is unmodelled, so any leaf where it fires DEFERS);
    /// * the §7.13.2.7 corner filter does NOT fire (`!(applyIbp && (w + h) >= 24)`)
    ///   — the corner-filtered `AboveRow[-1]` / `LeftCol[-1]` is unmodelled;
    /// * the §7.13.2.7 edge filter is a §7.13.2.17 strength-`0` no-op for BOTH
    ///   filter types (when `enable_intra_edge_filter == 1`) — the filtered edge is
    ///   unmodelled, so only a no-op-filter angle is reproducible over the raw edge;
    /// * EVERY §7.13.2.1 edge sample the projection reads is reconstructed by this
    ///   sink: the corner, the in-block edge, AND the above-right (zone-1) /
    ///   below-left (zone-3) the projection walks into — counted EXACTLY from
    ///   `IntraDirectionalAngle::max_one_sided_edge_read_index`, so an under-covered
    ///   neighbour DEFERS rather than reading a fill sentinel.
    #[allow(clippy::too_many_arguments)]
    fn try_reconstruct_one_sided_angular(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        mode: IntraYMode,
        angle_delta_y: i8,
        mrl_index: u8,
        block: &LumaCoeffBlock,
        qindex: u32,
        use_tcq: bool,
        mi_w: usize,
        mi_h: usize,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        // The one-sided reconstructor is SQUARE-only.
        if log2_width != log2_height {
            return Ok(false);
        }
        // §5.20.5.5 multi-reference line: the IDIF edge is the immediate line only.
        if mrl_index != 0 {
            return Ok(false);
        }
        let Some(nominal) = mode.mode_to_angle() else {
            return Ok(false);
        };
        let w = 1u32 << log2_width;
        let h = 1u32 << log2_height;
        // §5.20.5.3 / §7.13.2.8 pAngle = Mode_To_Angle[mode] + AngleDeltaY *
        // ANGLE_STEP + Mrl_Index_To_Delta[MrlIndex], then §5.20.7.29 wide-angle
        // remap (is_inter == 0 for a general-intra leaf).
        let mrl_delta = MRL_INDEX_TO_DELTA[usize::from(mrl_index).min(3)];
        let nominal_angle = i32::from(nominal) + i32::from(angle_delta_y) * ANGLE_STEP + mrl_delta;
        let p_angle = wide_angle_mapping(w, h, nominal_angle);
        // One-sided range (zone-1 reads above, zone-3 reads left). The cardinals
        // (90/180) and the zone-2 middle band (90 < p < 180) are handled elsewhere.
        let one_sided = (0 < p_angle && p_angle < 90) || (180 < p_angle && p_angle < 270);
        if !one_sided {
            return Ok(false);
        }
        let Ok(p_angle_u16) = u16::try_from(p_angle) else {
            return Ok(false);
        };
        let Ok(angle) = IntraDirectionalAngle::try_from_p_angle(p_angle_u16) else {
            return Ok(false);
        };
        // §7.13.2.7 applyIbp = enable_ibp && not4x4; useIBP additionally needs an
        // EVEN angleDelta, plane 0, a one-sided pAngle, and MrlIndex == 0. When
        // useIBP fires, the §7.13.2.9 IBP secondary blend (unmodelled) changes the
        // output — DEFER. (angleDelta is the SIGNED AngleDeltaY count.)
        let not4x4 = !(w == 4 && h == 4);
        let apply_ibp = self.enable_ibp && not4x4;
        let angle_delta_even = angle_delta_y % 2 == 0;
        if apply_ibp && angle_delta_even {
            // useIBP == 1 (one-sided pAngle + MrlIndex == 0 already hold).
            return Ok(false);
        }
        // §7.13.2.7 corner filter: `(applyIbp || (90 < p < 180)) && (w + h) >= 24`
        // rewrites `AboveRow[-1]` / `LeftCol[-1]`. For a one-sided angle the trigger
        // is `applyIbp && (w + h) >= 24`; the corner-filtered sample is unmodelled.
        if apply_ibp && (w + h) >= 24 {
            return Ok(false);
        }
        // §7.13.2.7 edge filter (runs for non-cardinal angles when
        // enable_intra_edge_filter == 1 && MrlIndex == 0): the §7.13.2.17 strength
        // must be 0 (a genuine `av2_filter_intra_edge` no-op) for the relevant edge,
        // for BOTH §7.13.2.15/16 filter types (so the result is independent of the
        // unmodelled neighbour-smooth state). zone-1 filters the above edge with
        // `angleAbove = pAngle - 90`; zone-3 the left edge with `angleLeft =
        // pAngle - 180`.
        if self.enable_intra_edge_filter {
            let delta = if p_angle < 90 {
                p_angle - 90
            } else {
                p_angle - 180
            };
            if intra_edge_filter_strength(w, h, 0, delta) != 0
                || intra_edge_filter_strength(w, h, 1, delta) != 0
            {
                return Ok(false);
            }
        }
        let Ok(block_size) = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        ) else {
            return Ok(false);
        };
        // The furthest logical edge index the projection reads (`base + 2`, capped at
        // maxBase). The above-right (zone-1) / below-left (zone-3) it walks into must
        // be reconstructed.
        let Ok(max_read) = angle.max_one_sided_edge_read_index(block_size) else {
            return Ok(false);
        };
        let side = 1usize << log2_width;
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        match angle.required_edge() {
            IntraDirectionalAngleEdge::Above => {
                // zone-1: verify the corner + above row + above-right are covered, and
                // count the above-right 4x4 units that span up to `max_read`.
                let Some(num4_above_right) =
                    self.one_sided_above_coverage(mi_col, mi_row, mi_w, side, max_read)
                else {
                    return Ok(false);
                };
                reconstruct_general_intra_one_sided_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    p_angle_u16,
                    PlaneId::Y,
                    x,
                    y,
                    log2_width,
                    qindex,
                    num4_above_right,
                    use_tcq,
                    self.bit_depth,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_one_sided_above_write",
                    )
                })?;
            }
            IntraDirectionalAngleEdge::Left => {
                let Some(num4_below_left) =
                    self.one_sided_left_coverage(mi_col, mi_row, mi_h, side, max_read)
                else {
                    return Ok(false);
                };
                reconstruct_general_intra_one_sided_left_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    p_angle_u16,
                    PlaneId::Y,
                    x,
                    y,
                    log2_width,
                    qindex,
                    num4_below_left,
                    use_tcq,
                    self.bit_depth,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_one_sided_left_write",
                    )
                })?;
            }
        }
        Ok(true)
    }

    /// zone-1 above-edge coverage guard. Verifies the §7.13.2.1 corner unit
    /// `(mi_col - 1, mi_row - 1)`, the above row `mi_row - 1` over the block's
    /// `mi_w` columns, AND the above-right units the projection walks into are all
    /// reconstructed by this sink, then returns the §7.13.2.1 `num4AboveRight` (in
    /// luma 4x4 units) to pass to the reconstructor. Returns `None` (defer) when any
    /// required unit is off-grid or uncovered.
    ///
    /// `max_read` is the furthest logical edge index the projection reads (sample
    /// `x + max_read`); the above-right span is the units from the block's right
    /// edge up to (and including) the one containing `x + max_read`. Because every
    /// returned unit is COVERED (hence §5.20.2.3 `BlockDecoded`), the real
    /// §5.20.7.25 `count_top_right_avail` is at least this count, so the §7.13.2.1
    /// `aboveLimit` (real and ours) both reach `x + max_read` without the spec clamp
    /// — the read stays inside the verified region and is bit-exact.
    fn one_sided_above_coverage(
        &self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        side: usize,
        max_read: usize,
    ) -> Option<usize> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let above = mi_row.checked_sub(1)?;
        let corner = mi_col.checked_sub(1)?;
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        // Corner `AboveRow[-1]` and the in-block above row.
        if !covered(corner, above) {
            return None;
        }
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return None;
        }
        // Above-right: the units from the block's right edge (`mi_col + mi_w`) up to
        // the MI unit containing the furthest read sample column `x + max_read`
        // (`x = mi_col * MI_SIZE`). `max_read` beyond the in-block span `side - 1` is
        // the above-right reach.
        let right_edge_mi = mi_col.checked_add(mi_w)?;
        if max_read < side {
            // The projection never reads past the block's own above row.
            return Some(0);
        }
        let furthest_col = mi_col.checked_mul(MI_SIZE)?.checked_add(max_read)?;
        let furthest_unit = furthest_col / MI_SIZE;
        let mut num4_above_right = 0usize;
        for unit in right_edge_mi..=furthest_unit {
            if !covered(unit, above) {
                return None;
            }
            num4_above_right += 1;
        }
        Some(num4_above_right)
    }

    /// zone-3 left-edge coverage guard, the symmetric mirror of
    /// [`Self::one_sided_above_coverage`]. Verifies the §7.13.2.1 corner unit
    /// `(mi_col - 1, mi_row)` (the top of the left column for a `haveAbove == 0`
    /// position, or the diagonal corner generally — both reduce to the left-column
    /// reconstructed sample), the left column `mi_col - 1` over the block's `mi_h`
    /// rows, AND the below-left units the projection walks into, then returns the
    /// §7.13.2.1 `num4BelowLeft`. Returns `None` (defer) when any required unit is
    /// off-grid or uncovered.
    fn one_sided_left_coverage(
        &self,
        mi_col: usize,
        mi_row: usize,
        mi_h: usize,
        side: usize,
        max_read: usize,
    ) -> Option<usize> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let left = mi_col.checked_sub(1)?;
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        // The in-block left column (the corner `LeftCol[-1]` is its top sample for a
        // `haveAbove == 0` block; verifying the full column covers it).
        if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r)) {
            return None;
        }
        let bottom_edge_mi = mi_row.checked_add(mi_h)?;
        if max_read < side {
            return Some(0);
        }
        let furthest_row = mi_row.checked_mul(MI_SIZE)?.checked_add(max_read)?;
        let furthest_unit = furthest_row / MI_SIZE;
        let mut num4_below_left = 0usize;
        for unit in bottom_edge_mi..=furthest_unit {
            if !covered(left, unit) {
                return None;
            }
            num4_below_left += 1;
        }
        Some(num4_below_left)
    }

    /// Reconstructs one luma transform block at the given MI position into the
    /// workspace, reading the §7.13.2 prediction from the partially-built frame's
    /// reconstructed neighbours and adding the decoded residual (a flat prediction
    /// for an `all_zero` block). The block is DEFERRED (returns `Ok(())` without
    /// writing — never wrong samples claimed correct) unless ALL of the proven
    /// subset holds:
    /// * the frame dequant matches the primitive's zero-`QuantizerDeltas`
    ///   assumption (`quant_reconstructable`);
    /// * the residual is the proven primitive kind ([`residual_is_reconstructable`]:
    ///   an `all_zero` flat block, or a non-`all_zero` block — square OR non-square,
    ///   any eob — that applies no §7.15.3 secondary transform (no IST syntax or a
    ///   `sec_tx_type == 0` no-op IST leaf) and is not FSC; the retained
    ///   `block.plane_tx_type` drives the §7.15.4 primary inverse, so a non-`DCT_DCT`
    ///   leaf reconstructs with its real `Transform_1d_Type[PlaneTxType]` kernels);
    /// * the leaf mode is one the sink can predict bit-exact AND its required
    ///   §7.13.2 prediction neighbours are off-frame or already reconstructed:
    ///   - `DC_PRED` (the §7.13.2.10 flat DC, with the §7.13.2.12 IBP DC modifier
    ///     when [`Self::ibp_dc_applies`]): both the above row and left column must
    ///     be off-frame or covered ([`Self::dc_edges_reconstructed`]);
    ///   - cardinal `H_PRED` (pAngle 180, `directional ==
    ///     Some(Horizontal)`): the §7.13.2.8 step-5 left-column copy. The left
    ///     column must be present and covered ([`Self::cardinal_left_reconstructed`]);
    ///     it reads ONLY the left column (no above, no corner, no IDIF, no `useIBP`),
    ///     and the leaf must use the immediate edge (`mrl_index == 0` — the primitive
    ///     reads the adjacent left/above samples, not a §5.20.5.5 multi-reference
    ///     line). The cardinal recon primitive is fully RECTANGULAR (the copy is
    ///     per-row) and the retained `block.plane_tx_type` drives the §7.15.4 inverse,
    ///     so a non-`all_zero` residual of ANY eob reconstructs with its real tx-type
    ///     (see [`residual_is_reconstructable`]);
    ///   - cardinal `V_PRED` (pAngle 90, `directional == Some(Vertical)`): the
    ///     §7.13.2.8 step-4 above-row copy. The above row must be present and covered
    ///     ([`Self::cardinal_above_reconstructed`]); same rectangular-aware
    ///     `mrl_index == 0` gate.
    ///
    /// Every OTHER mode (the §7.13.2.8 angular modes D45/D67/D113/D135/D157/D203,
    /// PAETH, SMOOTH, and any directional mode with a non-zero `AngleDeltaY` — which
    /// the upstream `supported_directional_luma` already maps to `None`) is DEFERRED.
    ///
    /// `use_tcq` carries the §7.14.4 luma TCQ `dqDenom` term; `qindex` is the
    /// per-block dequant index (the §5.20.6.5 `DeltaQState.current_q_index`);
    /// `fsc_mode` is the leaf's FSC flag; `mrl_index` is the leaf's §5.20.5.5
    /// `MrlIndex` (the multi-reference-line distance, `0` for the immediate edge).
    /// `mi_col` / `mi_row` are the transform's §3 MI coordinates and `tx_size` its
    /// §5.20.6 `TxSize` index.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn reconstruct_luma_transform(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        tx_size: usize,
        block: &LumaCoeffBlock,
        leaf_y_mode: Option<IntraYMode>,
        directional: Option<SupportedDirectionalLumaMode>,
        mrl_index: u8,
        angle_delta_y: i8,
        qindex: u32,
        use_tcq: bool,
        fsc_mode: bool,
        is_intrabc: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.quant_reconstructable {
            // Defer a frame whose dequant the primitive cannot honor.
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return Ok(());
        };
        if !residual_is_reconstructable(block, fsc_mode) {
            return Ok(());
        }
        if is_intrabc {
            // A §7.13.3.18 IntrABC leaf's luma prediction is the displaced `CurrFrame`
            // copy `reconstruct_intrabc_block` already wrote into the workspace (for a
            // non-skip block, recorded in `pending_intrabc_predictions`), NOT a §7.13.2
            // intra prediction — the `leaf_y_mode` is a §5.20.5.3 placeholder `DC_PRED`
            // (no intra Y mode is read for IntrABC). This leaf adds its §5.20.7.27
            // residual onto that copied predictor and marks its own coverage. The
            // residual is gated by `residual_is_reconstructable` above (no real IST, no
            // FSC); a skip IntrABC block carries no residual leaf here (its copy already
            // marked coverage). The copy must have landed: the leaf's rect must lie
            // inside a pending IntrABC prediction — otherwise the copy was deferred
            // (fractional DV / uncovered source / non-reconstructable quant) and adding
            // a residual onto the fill value would be the cardinal sin, so DEFER.
            return self.reconstruct_intrabc_residual_leaf(
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                block,
                qindex,
                use_tcq,
                tile_offset,
            );
        }
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        // The cardinal direction the sink can predict bit-exact, or `None` for DC /
        // every deferred mode. Only a CARDINAL `H_PRED` / `V_PRED` directional leaf
        // is admitted here; the angular modes (D45/D135/...) stay deferred.
        let cardinal = match directional {
            Some(SupportedDirectionalLumaMode::Horizontal) => {
                Some(IntraCardinalDirection::Horizontal)
            }
            Some(SupportedDirectionalLumaMode::Vertical) => Some(IntraCardinalDirection::Vertical),
            _ => None,
        };
        match (leaf_y_mode, cardinal) {
            (Some(IntraYMode::DC_PRED), _) => {
                if !self.dc_edges_reconstructed(PlaneId::Y, mi_col, mi_row, mi_w, mi_h) {
                    // A DC-prediction edge neighbour exists on-grid but was deferred;
                    // its workspace samples are the fill value, not reconstruction, so
                    // the DC prediction would be wrong. Defer this block too.
                    return Ok(());
                }
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                let ibp_dc = self.ibp_dc_applies(log2_width, log2_height);
                reconstruct_general_intra_block_rect_into(
                    &mut self.workspace,
                    block,
                    PlaneId::Y,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    use_tcq,
                    ibp_dc,
                    self.bit_depth,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_write",
                    )
                })?;
            }
            (_, Some(direction)) => {
                // Cardinal `H_PRED` / `V_PRED`: a §7.13.2.8 pure sample copy of the
                // real reconstructed left column (H) / above row (V). The cardinal
                // recon primitive is now fully RECTANGULAR (independent width/height:
                // V copies the W-wide above row into every one of the H rows; H fills
                // each of the H rows with one left sample), but it still reads the
                // IMMEDIATE edge (`MrlIndex == 0`), so defer a §5.20.5.5 multi-reference
                // line (`mrl_index > 0`): the primitive copies the ADJACENT left/above
                // samples, not the selected MRL reference line, so it would write the
                // wrong prediction. A non-`all_zero` residual of ANY eob now
                // reconstructs with its retained `block.plane_tx_type` (the §7.15.4
                // primary inverse resolves the real `Transform_1d_Type[PlaneTxType]`
                // kernels), so the former non-square `eob > 1` cardinal defer is gone —
                // `residual_is_reconstructable` already cleared the FSC / real-IST
                // gates above.
                if mrl_index != 0 {
                    return Ok(());
                }
                if !self.cardinal_edge_reconstructed(
                    direction,
                    PlaneId::Y,
                    mi_col,
                    mi_row,
                    mi_w,
                    mi_h,
                ) {
                    return Ok(());
                }
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                reconstruct_general_intra_cardinal_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    direction,
                    PlaneId::Y,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    use_tcq,
                    self.bit_depth,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_cardinal_write",
                    )
                })?;
            }
            // §7.13.2.2 PAETH (`PAETH_PRED`, IntraYMode 12, non-directional): admitted
            // ONLY for an `all_zero` leaf whose §7.13.2.1 above row AND left column are
            // BOTH real reconstructed neighbours (`haveAbove == 1 && haveLeft == 1`),
            // so the corner `AboveRow[-1] = CurrFrame[plane][y-1][x-1]` and both edges
            // are the genuine reconstructed samples — no §7.13.2.1 single-sided
            // synthesis (which the oracle shows PAETH does not match here) and no
            // residual (a non-`all_zero` PAETH would re-introduce the §5.20.7.29
            // tx-type / IST question). Every other PAETH config DEFERS.
            (Some(mode), None) if mode.is_paeth() && block.all_zero && mrl_index == 0 => {
                if !self.paeth_neighbours_reconstructed(PlaneId::Y, mi_col, mi_row, mi_w, mi_h) {
                    return Ok(());
                }
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    &mut self.workspace,
                    PlaneId::Y,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    self.bit_depth,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_paeth_write",
                    )
                })?;
            }
            // §7.13.2.8 ONE-SIDED angular luma (zone-1 `pAngle < 90`, reads the above
            // row + above-right; zone-3 `pAngle > 180`, reads the left column +
            // below-left), admitted ONLY for the proven no-IDIF-edge-filter,
            // no-useIBP, neighbour-covered subset. Every other angular leaf (the
            // zone-2 middle band, a filtered-edge angle, a `useIBP` blend, a non-zero
            // `MrlIndex`, or an uncovered neighbour) DEFERS.
            (Some(mode), None)
                if mode.is_directional()
                    && self.try_reconstruct_one_sided_angular(
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        mode,
                        angle_delta_y,
                        mrl_index,
                        block,
                        qindex,
                        use_tcq,
                        mi_w,
                        mi_h,
                        tile_offset,
                    )? => {}
            // Non-DC, non-cardinal luma (SMOOTH / non-admitted PAETH / angular /
            // a one-sided angle the guard could not prove): defer rather than emit
            // an unproven prediction.
            _ => return Ok(()),
        }
        // Count only the IN-FRAME 4x4 units `mark` actually covered: a frame-edge
        // transform writes (and so reconstructs) only its in-frame samples, so the
        // 4x4 tally must drop the off-frame overhang to stay equal to the canonical
        // covered region the oracle aggregate walks.
        let marked =
            self.coverage[Self::coverage_index(PlaneId::Y)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_luma_4x4 = self.reconstructed_luma_4x4.saturating_add(marked);
        Ok(())
    }

    /// Reconstructs one §7.13.3.18 NON-skip IntrABC luma residual transform leaf: adds
    /// the decoded §5.20.7.27 residual onto the displaced-copy prediction
    /// [`Self::reconstruct_intrabc_block`] already wrote into the workspace for this
    /// block, then marks the leaf's coverage.
    ///
    /// DEFERRED (returns `Ok(())` without writing) when the leaf's transform rect is
    /// NOT inside a pending IntrABC prediction: that means the whole-block copy was
    /// itself deferred (a fractional DV, an uncovered source, or a non-reconstructable
    /// quant), so the samples under the transform are the workspace fill value, and
    /// adding a residual onto them would claim an unreconstructed predictor as correct.
    /// The caller has already cleared the §5.20.7.29 real-IST / FSC residual gates
    /// ([`residual_is_reconstructable`]) and the integer-DV gate (the copy only records
    /// a pending prediction for an integer-DV, source-covered block), so the residual is
    /// the proven primitive kind over a real predictor.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_intrabc_residual_leaf(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        block: &LumaCoeffBlock,
        qindex: u32,
        use_tcq: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        let width = 1usize << log2_width;
        let height = 1usize << log2_height;
        if !self.rect_within_pending_intrabc_prediction(x, y, width, height) {
            // The whole-block IntrABC copy was deferred (or this leaf overhangs it),
            // so the predictor under the transform is the fill value — never add a
            // residual onto it.
            return Ok(());
        }
        reconstruct_intrabc_block_residual_rect_into(
            &mut self.workspace,
            block,
            PlaneId::Y,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            use_tcq,
            self.bit_depth,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_intrabc_residual_write",
            )
        })?;
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        let marked =
            self.coverage[Self::coverage_index(PlaneId::Y)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_luma_4x4 = self.reconstructed_luma_4x4.saturating_add(marked);
        Ok(())
    }

    /// Reconstructs one chroma (U or V) transform block at the given chroma-plane
    /// sample position into the workspace. The block is DEFERRED unless ALL of the
    /// proven subset holds: `chroma_mode` is `DC_PRED` (chroma never uses the
    /// §7.14.4 TCQ term); the frame dequant matches the zero-`QuantizerDeltas`
    /// assumption (`quant_reconstructable`); the residual is the proven primitive
    /// kind ([`residual_is_reconstructable`]: `all_zero` flat-DC, or a square
    /// non-`all_zero` block with no IST — chroma is never FSC); and the §7.13.2
    /// DC-prediction edges are off-frame or already reconstructed by this sink. The
    /// `(x, y)` sample position must be MI-aligned (chroma transforms are).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn reconstruct_chroma_transform(
        &mut self,
        plane_id: PlaneId,
        chroma_tx: usize,
        x: usize,
        y: usize,
        block: &LumaCoeffBlock,
        chroma_mode: Option<SupportedChromaMode>,
        qindex: u32,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if chroma_mode != Some(SupportedChromaMode::Dc) || !self.quant_reconstructable {
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(chroma_tx) else {
            return Ok(());
        };
        // Chroma is never an FSC leaf.
        if !residual_is_reconstructable(block, false) {
            return Ok(());
        }
        let (mi_col, mi_row) = (x / MI_SIZE, y / MI_SIZE);
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if !self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h) {
            return Ok(());
        }
        // The sink admits only DC chroma (never `UV_CFL_PRED`), so the §7.13.2.12
        // IBP DC gate reduces to `enable_ibp && !(w == 4 && h == 4)` for chroma too.
        let ibp_dc = self.ibp_dc_applies(log2_width, log2_height);
        reconstruct_general_intra_block_rect_into(
            &mut self.workspace,
            block,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            // Chroma never uses the §7.14.4 TCQ dqDenom term (luma DCT_DCT only).
            false,
            ibp_dc,
            self.bit_depth,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_write",
            )
        })?;
        let marked = self.coverage[Self::coverage_index(plane_id)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_chroma_4x4 = self.reconstructed_chroma_4x4.saturating_add(marked);
        Ok(())
    }

    /// Reconstructs one §7.13.3.18 IntrABC luma block into the workspace by copying
    /// the displaced predictor rectangle from the partially-built `CurrFrame` and
    /// adding the (zero, for a skip block) residual.
    ///
    /// The IntrABC block-vector parse already derived and bounds-checked the integer
    /// luma `source` / `target` rectangles ([`super::intrabc_records::IntrabcPredictionGeometry`]);
    /// the §7.13.3.18 block-inter-prediction path with `refIdx == -1` and an integer
    /// block vector reduces to a plain `w` x `h` sample copy of `CurrFrame` at
    /// `(x + dvX, y + dvY)` (the BILINEAR filter has zero fractional taps), which the
    /// [`CurrentFrameWorkspace::copy_rect_within_plane`] integer-vector primitive
    /// performs (snapshotting the source before the target write).
    ///
    /// For a `skip` block (zero residual) the displaced copy IS the reconstruction:
    /// it marks coverage final. For a NON-skip block the displaced copy is only the
    /// §7.13.2 PREDICTION; this records the target rect in
    /// [`Self::pending_intrabc_predictions`] WITHOUT marking coverage, and the
    /// per-transform [`Self::reconstruct_luma_transform`] leaves (decoded AFTER this
    /// prelude copy, in decode order) add the §5.20.7.27 residual onto the copied
    /// predictor and mark their own coverage. So the displaced copy runs for both
    /// skip and non-skip; only coverage timing differs.
    ///
    /// The block is DEFERRED (returns `Ok(())` without writing — never wrong samples
    /// claimed correct) unless ALL of the proven subset holds:
    /// * the frame dequant matches the zero-`QuantizerDeltas` assumption
    ///   (`quant_reconstructable`);
    /// * the block vector is INTEGER (`source` and `target` have the same shape) — a
    ///   fractional BILINEAR IntrABC predictor needs a convolution path, not a copy;
    /// * EVERY source MI unit is already reconstructed by this sink — copying an
    ///   unreconstructed (fill) source sample is the cardinal sin.
    ///
    /// `source` / `target` are the §7.13.3.18 luma copy rectangles (sample units).
    pub(in crate::runtime_minimal) fn reconstruct_intrabc_block(
        &mut self,
        source: PlaneRect,
        target: PlaneRect,
        skip_flag: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.quant_reconstructable {
            // Defer a frame whose dequant the primitive cannot honor.
            return Ok(());
        }
        // An integer block vector keeps the predictor a same-shape copy; a fractional
        // vector widens the source by a BILINEAR border (deferred — needs convolution).
        if source.size() != target.size() {
            return Ok(());
        }
        // The source rectangle's covered-MI span must be computed from the actual
        // sample EXTENT, not a floored width: a NON-4x4-aligned integer source offset
        // (the parser can produce e.g. a -504 eighth-pel == -63px vector) makes a
        // source straddle a trailing partial MI unit that `width / MI_SIZE` would drop.
        // Ceil the right/bottom edge and floor the left/top so EVERY MI unit the source
        // touches is checked (codex finding 1) — otherwise an unreconstructed trailing
        // MI could be copied as fill and marked bit-exact. ac0ej3's 4x4-aligned source
        // (x=224, width=32) is unchanged: floor(224/4)=56, ceil(256/4)=64, mi_w=8.
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let src_mi_col = source.x() / MI_SIZE;
        let src_mi_row = source.y() / MI_SIZE;
        let src_mi_w = (source.x() + source.width()).div_ceil(MI_SIZE) - src_mi_col;
        let src_mi_h = (source.y() + source.height()).div_ceil(MI_SIZE) - src_mi_row;
        if !coverage.region_fully_covered(src_mi_col, src_mi_row, src_mi_w, src_mi_h) {
            // A source MI unit is off-grid or still the workspace fill value; copying
            // it would claim an unreconstructed sample as correct. Defer this block.
            return Ok(());
        }
        self.workspace
            .copy_rect_within_plane(PlaneId::Y, source, target)
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_intrabc_copy",
                )
            })?;
        if !skip_flag {
            // The copy is only the §7.13.2 prediction; the §5.20.7.27 residual leaves
            // (decoded after this prelude, in decode order) own the coverage. Record
            // the target rect so a residual leaf can prove its predictor is the real
            // displaced copy before adding its residual.
            self.pending_intrabc_predictions.push(target);
            return Ok(());
        }
        let (tgt_mi_col, tgt_mi_row) = (target.x() / MI_SIZE, target.y() / MI_SIZE);
        let (tgt_mi_w, tgt_mi_h) = (target.width() / MI_SIZE, target.height() / MI_SIZE);
        let marked = self.coverage[Self::coverage_index(PlaneId::Y)]
            .mark(tgt_mi_col, tgt_mi_row, tgt_mi_w, tgt_mi_h);
        self.reconstructed_luma_4x4 = self.reconstructed_luma_4x4.saturating_add(marked);
        Ok(())
    }

    /// Reads a reconstructed sample for the region-verification test. Out-of-range
    /// or unreconstructed coordinates return the workspace fill value through the
    /// checked workspace path.
    #[cfg(test)]
    pub(in crate::runtime_minimal) fn reconstructed_sample(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
    ) -> Result<T> {
        Ok(self.workspace.reconstructed_sample(plane_id, x, y)?)
    }

    /// The number of 4x4 luma / chroma units reconstructed so far (test reporting).
    #[cfg(test)]
    pub(in crate::runtime_minimal) const fn reconstructed_counts(&self) -> (usize, usize) {
        (self.reconstructed_luma_4x4, self.reconstructed_chroma_4x4)
    }

    /// Visits every luma sample the sink has RECONSTRUCTED (per the MI-unit coverage
    /// map), in row-major sample order, invoking `visit(x, y, sample)`. Used by the
    /// region-verification test to pin the whole reconstructed luma region against
    /// the AVM pre-filter oracle PER VALUE (not by count alone): an uncovered MI unit
    /// (a deferred / fill region) is skipped, so only spec-reconstructed samples are
    /// visited.
    #[cfg(test)]
    pub(in crate::runtime_minimal) fn for_each_reconstructed_luma_sample(
        &self,
        mut visit: impl FnMut(usize, usize, T),
    ) -> Result<()> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        for mi_row in 0..coverage.rows {
            for mi_col in 0..coverage.cols {
                if !coverage.is_covered(mi_col, mi_row) {
                    continue;
                }
                for dy in 0..MI_SIZE {
                    for dx in 0..MI_SIZE {
                        let x = mi_col * MI_SIZE + dx;
                        let y = mi_row * MI_SIZE + dy;
                        visit(x, y, self.reconstructed_sample(PlaneId::Y, x, y)?);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Drives the ac0ej3 `TX_MODE_SELECT` selectable transform-record walk with a
/// reconstruction sink attached and returns the populated sink, for the
/// region-verification test. The walk reconstructs the verified NON-IntrABC DC
/// region into the sink's workspace in decode order, then (for the ac0ej3 stream)
/// fails closed at the first active IntrABC block — the returned sink retains
/// everything reconstructed before that point, so the test can compare the first
/// superblock against the pre-filter reconstruction oracle. The public decode path
/// never calls this (it runs the handoff with no sink and emits no frame). This is
/// a 10-bit (`u16`) driver: the ac0ej3 sequence is 10-bit 4:2:0.
#[cfg(test)]
pub(in crate::runtime_minimal) fn reconstruct_ac0ej3_selectable_intra_region(
    bytes: &[u8],
    options: crate::DecodeOptions,
    plan: &crate::DecodeStreamPlan,
    key_candidate: &crate::DecodePlannedObu,
    key_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &splot_core::headers::sequence::SequenceHeader,
    core: &splot_core::headers::frame::FrameHeaderCore,
) -> Result<WienerNsLrReconSink<u16>> {
    let frame_size = core.frame_size.ok_or_else(|| {
        super::super::unsupported_at(
            "missing_frame_size_for_recon",
            key_envelope.offset,
            "ac0ej3 reconstruction bridge requires the parsed frame size",
        )
    })?;
    let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())?;
    // §5.4.5 `enable_ibp`: the selectable tool gate (unlike `fixed_largest`) admits
    // `enable_ibp`, so a DC_PRED leaf must run the §7.13.2.12 IBP DC modifier when
    // the sequence enables it. ac0ej3's intra config has `enable_ibp == 1`.
    let enable_ibp = sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp);
    // §5.4.5 `enable_intra_edge_filter`: when set, the §7.13.2.7 edge / corner
    // filter runs for non-cardinal angles; the sink admits a one-sided angular leaf
    // only when that filter is a §7.13.2.17 strength-0 no-op (and no corner filter).
    // ac0ej3's sequence sets this flag.
    let enable_intra_edge_filter = sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_intra_edge_filter);
    let mut sink = WienerNsLrReconSink::<u16>::new(
        frame_size.width as usize,
        frame_size.height as usize,
        bit_depth,
        frame_quant_reconstructable(core),
        enable_ibp,
        enable_intra_edge_filter,
    )?;
    // The walk reconstructs into the sink in decode order. With the AVM-faithful
    // §5.20.3.1 SDP chroma partition plane (plane 1 for the chroma tree) and the
    // §8.3.2 `is_cfl` neighbour-context fix (the chroma `is_cfl` CDF is keyed by the
    // above/left `UVCfls` neighbours, not a hardcoded `ctx == 0`), the parse stays
    // entropy-synced past the IntrABC ref-stack wall and — with the §5.20.7.27 context
    // write AND the §5.20 `reset_block_context` write both now clamped to the frame
    // edge — past the bottom-edge skipped transforms. The selectable transform-record
    // handoff now runs to COMPLETION (`Ok`): the verified subset is reconstructed in
    // decode order and the out-of-subset IntrABC/general-intra blocks are
    // conservatively deferred to their fill value (never a confident-wrong sample), so
    // no per-block frontier is raised (see `EXPECTED_RECON_FRONTIER_REASON`); the owned
    // sink retains the verified reconstructed region.
    // `EXPECTED_RECON_FRONTIER_REASON` is the DEFENSIVE net: if a regression
    // re-introduces an earlier handoff frontier or desync, swallow ONLY that one known
    // reason — every other error is propagated so the test fails loudly instead of
    // silently passing on a partial walk.
    match super::tx_records::derive_wienerns_lr_selectable_transform_record_handoff(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        sequence,
        core,
        Some(&mut sink),
    ) {
        Ok(_) => Ok(sink),
        Err(crate::error::DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == EXPECTED_RECON_FRONTIER_REASON =>
        {
            Ok(sink)
        }
        Err(other) => Err(other),
    }
}

/// The single fail-closed reason the ac0ej3 selectable walk is expected to stop on
/// after reconstructing the verified region; the test driver swallows only this one
/// and propagates every other error. The §7.12.2.19 IntrABC ref-MV weight sort is
/// now modelled (per-candidate §7.12.2.6 weights + the max-weight-to-slot-0 reorder,
/// threading the real `enable_drl_reorder` flag), so the `BLOCK_64X32` MI(192,112)
/// block — which has TWO distinct spatial candidates ((-1024,0) step 7 + (-512,0)
/// step 8) and so triggers the §7.12.2.19 sort — admits its ref-MV stack faithfully
/// (a no-op swap; slot 0 keeps (-1024,0), drl=1 selects (-512,0), bit-exact vs
/// avmdec) instead of deferring, as do its downstream IntrABC siblings. The IntrABC
/// ref-stack wall is now fully cleared and the §5.20.6.1 `LumaTxSizes` frame-array
/// fill drops out-of-frame tx cells (no more MI(256,0) `out_of_bounds`). The
/// §5.20.4.1 SDP chroma-reference MI-size write (chroma leaves and the chroma plane
/// of shared luma+chroma leaves now write `MiSizes[1]` over the `ChromaMiSize`
/// footprint, not the per-leaf luma geometry) removed the former MI(240,240) §8.3.2
/// `do_split` left-context desync, so the over-read `bitstream_desync` is gone. The
/// §5.20.7.27 coefficient context-line WRITE is now clamped to the frame edge
/// (modelling AVM `av2_set_entropy_contexts`, `av2/common/blockd.c:138-166`): the
/// skipped TX_64X64 luma transforms on the tile bottom edge — whose 16-tall left
/// span overhangs the tile by up to one transform extent — write `culLevel` /
/// `dcCategory` over only their on-tile rows instead of erroring on the overhang,
/// matching AVM's SB-local entropy lines (the OR-reduce reads already clamp). The
/// walk now advances bit-exact through those bottom-edge transforms. The §7.13.2.1
/// edge extension (a transform overhanging the frame bottom/right edge-extends its
/// clamped in-frame left column / above row to the block's full nominal height/width
/// by replicating the last in-frame sample, per AVM `av2/common/reconintra.c:1191-1195`)
/// lets the bottom-edge `TX_64X64` `DC_PRED` blocks — whose 56-row in-frame left
/// column previously errored `IntraPredictionEdgeLengthMismatch{expected:64,actual:56}`
/// — reconstruct their in-frame samples bit-exact. The §5.20.6.1 IntrABC
/// `record_block` mode-info fill is now ALSO clamped to the frame edge (modelling AVM
/// §5.20.3.2 `block_coded(r,c) { r < MiRows && c < MiCols }`,
/// 05-syntax-structures.md:9621): the non-IntrABC `BLOCK_128X64` leaf at MI(256,0)
/// whose nominal 16-tall MI footprint overhangs the 270-row MI grid by 2 MI rows
/// records only its 14 in-frame MI rows instead of erroring `..._intrabc_block_bounds`,
/// so the walk advances past that former frontier and the bottom partial-SB row's
/// in-frame samples reconstruct (region grows `267776` → `273152`). The ref-MV bank
/// `update_after_block` still uses the NOMINAL block size, NOT the frame-clamped
/// extent, so the §7.12.2 `remain_hits` budget stays synced (no re-introduced EC
/// desync).
///
/// The §6.19.7.12 IntrABC PREDICTION-GEOMETRY target is now ALSO clamped to the
/// visible region (modelling the same AVM §5.20.3.2 `block_coded`): the bottom-edge
/// `BLOCK_16X64` IntrABC block at MI(256,56) — whose nominal 64-tall target overhangs
/// the 1080-row luma frame by 8 rows — derives an EFFECTIVE 16x56 in-frame target
/// (and a congruent 16x56 source) instead of erroring `intrabc_target_bounds`, so the
/// parse advances past that former frontier. The block's own RECONSTRUCTION is now
/// ADMITTED: its real DV `(row=-1024, col=0)` (an integer -128px VERTICAL displacement
/// whose source sits in the PREVIOUS superblock row) is validated by AVM
/// `av2_is_dv_valid` via the §7.13.3.18 `allow_global_intrabc` wavefront branch
/// (ac0ej3 sets `allow_global_intrabc==1`), NOT the same-superblock
/// `av2_is_dv_in_local_range` branch. That global wavefront branch is now WIRED into
/// [`super::intrabc_records`]'s `intrabc_dv_proven_valid` (the local same-SB subset is
/// tried first, then the intra-only global branch), and with the upstream §5.20.5.5
/// y-mode reorder gate fixed the admitted copy plus the regular-intra cascade it
/// re-ignites is per-sample bit-exact vs the oracle. A source the global branch cannot
/// prove still defers — never a confident-wrong sample. The §5.20 `reset_block_context`
/// write is now ALSO clamped to the frame edge
/// (modelling AVM `av2_set_entropy_contexts` / `av2_reset_entropy_context`,
/// `av2/common/blockd.c`, and the §5.20.3.2 `block_coded` model): the bottom-edge
/// SKIPPED transforms at MI(256,0) — whose nominal 16-tall MI footprint overhangs the
/// 270-row MI grid by 2 — zero only their on-frame context cells instead of erroring
/// `skipped_context_reset`, and the §5.20.6.1 PC-Wiener `LrTxSkip` FilterClass grid
/// retention drops those same off-frame MI cells. With both clamps the reconstruction
/// sink walk now runs the selectable transform-record handoff to COMPLETION (`Ok`) —
/// every block in the verified subset is reconstructed in decode order and the
/// remaining IntrABC/general-intra blocks outside the subset are conservatively
/// deferred (their fill value, never a confident-wrong sample), so the handoff no
/// longer raises a per-block frontier. The parse-only public-decode path advances
/// PAST this same point and stops at the §7.20.4 `live_frame_samples_unpopulated`
/// gate (decoded CurrFrame / CdefFrame samples are still unpopulated for
/// storage-backed FilterClass retention). `EXPECTED_RECON_FRONTIER_REASON` stays as a
/// DEFENSIVE net: if a regression re-introduces an earlier frontier or desync inside
/// the handoff, the swallow matches ONLY this one reason and every other error (and
/// the now-expected `Ok`) is handled distinctly, so the test fails loudly rather than
/// silently passing on a partial walk. The verified region is committed regardless, so
/// it stays bit-exact vs the oracle.
#[cfg(test)]
const EXPECTED_RECON_FRONTIER_REASON: &str =
    "unsupported_wienerns_lr_selectable_live_frame_samples_unpopulated";

/// Whether the frame's §5.18.6 quantization matches the reconstruction primitive's
/// zero-`QuantizerDeltas` assumption: no per-plane DC/AC quantizer delta and no
/// quantizer matrix. When `false` the sink must reconstruct nothing (the primitive
/// would dequantize with the wrong DC/AC quantizers), so the gate defers — the safe
/// choice. ac0ej3's verified frame has no such delta.
#[cfg(test)]
fn frame_quant_reconstructable(core: &splot_core::headers::frame::FrameHeaderCore) -> bool {
    let deltas_zero = core.quantization_params.as_ref().is_none_or(|q| {
        q.delta_q_y_dc == 0
            && q.delta_q_u_dc == 0
            && q.delta_q_u_ac == 0
            && q.delta_q_v_dc == 0
            && q.delta_q_v_ac == 0
    });
    let no_qmatrix = core
        .setup_qm_params
        .as_ref()
        .is_none_or(|qm| !qm.using_qmatrix);
    deltas_zero && no_qmatrix
}

/// Maps a §5.20.6 `TxSize` index to its `(log2_width, log2_height)` sample
/// dimensions via the §9 `Tx_Width` / `Tx_Height` log2 tables, or `None` when the
/// index is outside the 19-entry table range.
fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    let w = u32::try_from(*TX_WIDTH_LOG2.get(tx_size)?).ok()?;
    let h = u32::try_from(*TX_HEIGHT_LOG2.get(tx_size)?).ok()?;
    Some((w, h))
}

/// The §3 sample-space `(x, y)` origin of a luma MI position, overflow-checked.
fn luma_sample_origin(
    mi_col: usize,
    mi_row: usize,
    tile_offset: ByteOffset,
) -> Result<(usize, usize)> {
    let x = mi_col.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_recon_luma_x_overflow",
        )
    })?;
    let y = mi_row.checked_mul(MI_SIZE).ok_or_else(|| {
        wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            "unsupported_wienerns_lr_selectable_transform_records_recon_luma_y_overflow",
        )
    })?;
    Ok((x, y))
}

/// The MI-unit `(width, height)` of a transform with the given log2 sample
/// dimensions (one MI unit spans `MI_SIZE` samples; a transform is at least one MI
/// unit per axis).
fn mi_extent(log2_width: u32, log2_height: u32) -> (usize, usize) {
    let mi_w = (1usize << log2_width >> 2).max(1);
    let mi_h = (1usize << log2_height >> 2).max(1);
    (mi_w, mi_h)
}

/// Whether a block's residual is the kind [`reconstruct_general_intra_block_rect_into`]
/// reconstructs bit-exact.
///
/// The primitive composes the §7.14.4 dequantization, the §7.15.4 / §7.15.4.1
/// inverse transform, and the §7.14.3 residual addition over the `DCT_DCT`
/// no-secondary-transform path with zero `QuantizerDeltas`. An `all_zero`
/// (`txb_skip`) block is always safe: there is no residual, so the output is the
/// bare §7.13.2 flat DC prediction. A non-`all_zero` block is admitted when it
/// applies no §7.15.3 secondary inverse transform — either no §5.20.7.29 IST
/// syntax at all, or IST syntax with `sec_tx_type == 0` (the §7.15.3 secondary
/// transform is a NO-OP, so the leaf reconstructs through the identical DCT_DCT
/// residual path; the reconstruction primitive never consults `intra_ist`) — and
/// is not an FSC leaf. A REAL IST leaf (`sec_tx_type != 0`) still defers (the
/// §7.15.3 secondary transform is unimplemented).
///
/// Both SQUARE and NON-square (rectangular) non-`all_zero` residuals are now
/// admitted: `LumaCoeffBlock` retains the real §3 `PlaneTxType`, and the §7.15.4
/// primary inverse transform ([`inverse_transform_2d_outer`]) resolves the actual
/// `Transform_1d_Type[PlaneTxType]` row/col kernels for it (the §7.15.4.1
/// `Adjusted_Tx_Size` per-side `Min(log2, 5)` cap, the `Abs(log2W - log2H)`
/// odd-ratio `Round2(x * 2896, 12)` √2 rescale, and the nearest-neighbour
/// duplication for any original side over 32 all apply for every type). Intra
/// passes use `use_ddt == false`, so the inter-only DDT/DDTX substitution never
/// applies. The DC-only (`eob == 1`) rectangular case stays proven bit-exact for the
/// ac0ej3 `TX_16X64` `DC_PRED` leaf at MI(4,0) (x[16,32), y[0,64)), and an `eob > 1`
/// block — square or non-square, any DCT/ADST/FLIPADST/IDTX/cardinal type — now
/// reconstructs with its REAL tx-type instead of the former hardcoded `DCT_DCT`.
/// The REAL-IST (`sec_tx_type != 0`) / FSC / quant-delta / incomplete-neighbour
/// gates stay intact; everything else is deferred.
fn residual_is_reconstructable(block: &LumaCoeffBlock, fsc_mode: bool) -> bool {
    if block.all_zero {
        return true;
    }
    // §7.15.3: a `sec_tx_type == 0` IST leaf applies NO secondary inverse
    // transform — it reconstructs via the identical §7.14.4 / §7.15.4 primary
    // residual path as a non-IST leaf (the primitive never consults `intra_ist`).
    // Only a REAL IST leaf (`sec_tx_type != 0`) needs the unimplemented §7.15.3
    // secondary transform, so defer ONLY that. A no-op-IST leaf must still satisfy
    // every other residual condition below (the FSC / real-IST gates), exactly
    // like a non-IST leaf.
    let real_ist = block.intra_ist.is_some_and(|ist| ist.sec_tx_type != 0);
    // The §7.15.4 primary inverse now resolves the retained `block.plane_tx_type`
    // for both square and non-square blocks at any eob, so the former non-square
    // `eob > 1` tx-type defer is gone. The FSC / real-IST gates remain.
    !(real_ist || fsc_mode)
}

#[cfg(test)]
#[path = "recon_tests.rs"]
mod tests;
