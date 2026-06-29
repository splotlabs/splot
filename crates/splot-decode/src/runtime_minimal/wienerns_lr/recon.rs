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
    IbpSecondary, OneSidedEdgeFilter, reconstruct_general_intra_block_rect_into,
    reconstruct_general_intra_cardinal_neighbour_block_into,
    reconstruct_general_intra_luma_paeth_neighbour_block_into,
    reconstruct_general_intra_one_sided_ibp_luma_block_into,
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
    /// Per-MI-unit decoded luma Y-mode (`y_modes[mi_row * cols + mi_col]`), the
    /// §5.20.5.3 `YModes[r][c]` the §7.13.2.15/16 `get_filter_type_above` /
    /// `get_filter_type_left` neighbour-smooth pick reads. Recorded for EVERY luma
    /// block the walk decodes (admitted OR deferred — a deferred neighbour is still
    /// `AvailU`/`AvailL`), independent of `covered`. `None` where no block has been
    /// decoded into the unit yet (`AvailU`/`AvailL == 0`, an off-frame neighbour).
    /// Only the luma plane (index 0) populates this; chroma `get_filt_type` reads a
    /// separate `UVSmooth` state this sink does not model.
    y_modes: Vec<Option<IntraYMode>>,
}

impl PlaneCoverage {
    #[cfg(test)]
    fn new(width_samples: usize, height_samples: usize) -> Self {
        let cols = width_samples.div_ceil(MI_SIZE);
        let rows = height_samples.div_ceil(MI_SIZE);
        let cells = cols.saturating_mul(rows);
        Self {
            cols,
            rows,
            covered: vec![false; cells],
            y_modes: vec![None; cells],
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

    /// The length of the contiguous run of reconstructed MI units starting at the
    /// origin-adjacent cell of a §7.13.2.1 reference edge, stopping at the first
    /// uncovered (or off-grid) cell — the §5.20.2.3 / AVM `count_top_right_avail` /
    /// `count_bottom_left_avail` contiguous-availability scan (BREAK on the first
    /// uncovered cell; an interior hole truncates the run, it is NOT skipped).
    ///
    /// The edge starts at `(start_col, start_row)` and advances by `(step_col,
    /// step_row)` per cell for `len` cells (`step_row == 1, step_col == 0` for a
    /// left column read top-to-bottom; `step_col == 1, step_row == 0` for an above
    /// row read left-to-right). The returned count is in `0..=len`: `0` means even
    /// the origin-adjacent cell is uncovered.
    fn covered_run_len(
        &self,
        start_col: usize,
        start_row: usize,
        step_col: usize,
        step_row: usize,
        len: usize,
    ) -> usize {
        let mut run = 0usize;
        for i in 0..len {
            let c = start_col + i * step_col;
            let r = start_row + i * step_row;
            if !self.is_covered(c, r) {
                break;
            }
            run += 1;
        }
        run
    }

    /// Records the §5.20.5.3 decoded `YModes` value for every IN-GRID MI unit of the
    /// `mi_w` x `mi_h` block at `(mi_col, mi_row)`. Called for EVERY luma block the
    /// walk decodes (admitted or deferred), so a later block's §7.13.2.15/16
    /// neighbour-smooth pick reads the real decoded neighbour mode.
    fn record_y_mode(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
        mode: IntraYMode,
    ) {
        for r in mi_row..mi_row.saturating_add(mi_h) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if !self.off_grid(c, r)
                    && let Some(slot) = self.y_modes.get_mut(r * self.cols + c)
                {
                    *slot = Some(mode);
                }
            }
        }
    }

    /// The §5.20.5.3 `YModes[mi_row][mi_col]` decoded into this MI unit, or `None`
    /// when off-grid or no block has been decoded there yet (`AvailU`/`AvailL == 0`).
    fn y_mode_at(&self, mi_col: usize, mi_row: usize) -> Option<IntraYMode> {
        if self.off_grid(mi_col, mi_row) {
            return None;
        }
        self.y_modes
            .get(mi_row * self.cols + mi_col)
            .copied()
            .flatten()
    }
}

/// Which §7.13.2.1 reference edge a one-sided filter is being assembled for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EdgeOrientation {
    /// The above row `AboveRow[..]` (zone-1 read edge / zone-3 IBP secondary edge).
    Above,
    /// The left column `LeftCol[..]` (zone-3 read edge / zone-1 IBP secondary edge).
    Left,
}

/// The resolved §7.13.2.7 inputs for ONE one-sided reference edge, fed to
/// [`WienerNsLrReconSink::assemble_one_sided_edge_filter`]: the edge orientation,
/// the §7.13.2.15/16 `filterType` (smooth-neighbour flag), the §7.13.2.7
/// `angleAbove`/`angleLeft` delta, and whether the far extension (above-right /
/// below-left) is needed for the §7.13.2.18 sweep span.
#[derive(Clone, Copy, Debug)]
struct OneSidedEdgeSpec {
    orientation: EdgeOrientation,
    filter_type: bool,
    angle_delta: i32,
    need_far: bool,
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
#[allow(clippy::if_same_then_else)]
fn intra_edge_filter_strength(w: u32, h: u32, filter_type: u8, delta: i32) -> u8 {
    let d = delta.unsigned_abs();
    let blk_wh = w + h;
    let mut strength = 0u8;
    if filter_type == 0 {
        if blk_wh <= 8 {
            if d >= 56 {
                strength = 1;
            }
        } else if blk_wh <= 12 {
            if d >= 40 {
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
        if let Some(above) = mi_row.checked_sub(1) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if !coverage.off_grid(c, above) && !coverage.is_covered(c, above) {
                    return false;
                }
            }
        }
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
    ///   `mi_col == 0`) and the cardinal predictor fills the WHOLE block with ONE
    ///   orthogonal sample — the origin-adjacent reference `left_ref[0] =
    ///   CurrFrame[y][x-1]` for V, `above_ref[0] = CurrFrame[y-1][x]` for H. This is
    ///   AVM `av2_build_intra_predictors_high`'s `(!need_left && n_top_px == 0)` /
    ///   `(!need_above && n_left_px == 0)` fast path (`reconintra.c:1150-1163`), which
    ///   reads ONLY `left_ref[0]`/`above_ref[0]` (V_PRED has `need_left == 0`, H_PRED
    ///   has `need_above == 0`, so the rest of the orthogonal edge is never read). The
    ///   fallback therefore admits as soon as the ORIGIN-ADJACENT orthogonal cell is
    ///   reconstructed, even when the rest of that edge is still deferred (the
    ///   over-strict full-edge gate would defer the partial-edge case spuriously).
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
        let fallback_origin_covered = |start_col: usize, start_row: usize| {
            coverage.covered_run_len(start_col, start_row, 0, 0, 1) >= 1
        };
        match direction {
            IntraCardinalDirection::Horizontal => match mi_col.checked_sub(1) {
                Some(left) => left_col_covered(left),
                None => match mi_row.checked_sub(1) {
                    Some(above) => fallback_origin_covered(mi_col, above),
                    None => false,
                },
            },
            IntraCardinalDirection::Vertical => match mi_row.checked_sub(1) {
                Some(above) => above_row_covered(above),
                None => match mi_col.checked_sub(1) {
                    Some(left) => fallback_origin_covered(left, mi_row),
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
            return false;
        };
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return false;
        }
        if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r)) {
            return false;
        }
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
    ///   `180 < pAngle < 270`) with a §9.2 derivative entry — recovered AFTER the
    ///   §5.20.7.29 wide-angle remap, whose tall-block (`h == k*w`) / wide-block
    ///   (`w == k*h`) wrap branches FIRE for non-square transforms (inert for
    ///   square), so a leaf can become one-sided only after the remap;
    /// * the transform is SQUARE OR NON-SQUARE — §7.13.2.8 is non-square-aware
    ///   (`w == Tx_Width`, `h == Tx_Height` independent; `maxBase == w + h - 1`,
    ///   `aboveLimit` keys on `w`, `leftLimit` keys on `h`), so the IDIF predictor
    ///   and edge builders consume the real `(w, h)`;
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
        if mrl_index != 0 {
            return Ok(false);
        }
        let Some(nominal) = mode.mode_to_angle() else {
            return Ok(false);
        };
        let w = 1u32 << log2_width;
        let h = 1u32 << log2_height;
        let mrl_delta = MRL_INDEX_TO_DELTA[usize::from(mrl_index).min(3)];
        let nominal_angle = i32::from(nominal) + i32::from(angle_delta_y) * ANGLE_STEP + mrl_delta;
        let p_angle = wide_angle_mapping(w, h, nominal_angle);
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
        let not4x4 = !(w == 4 && h == 4);
        let apply_ibp = self.enable_ibp && not4x4;
        let angle_delta_even = angle_delta_y % 2 == 0;
        let use_ibp = apply_ibp && angle_delta_even;
        if use_ibp {
            return self.try_reconstruct_one_sided_ibp_angular(
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                angle,
                p_angle,
                p_angle_u16,
                block,
                qindex,
                use_tcq,
                mi_w,
                mi_h,
                tile_offset,
            );
        }
        let Some(edge_filter) = self.resolve_one_sided_edge_filter(
            mi_col,
            mi_row,
            w,
            h,
            p_angle,
            apply_ibp,
            tile_offset,
        )?
        else {
            return Ok(false);
        };
        let Ok(block_size) = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        ) else {
            return Ok(false);
        };
        let Ok(max_read) = angle.max_one_sided_edge_read_index(block_size) else {
            return Ok(false);
        };
        let w = w as usize;
        let h = h as usize;
        let edge_filter_active = edge_filter.strength != 0 || edge_filter.corner_opposite.is_some();
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        match angle.required_edge() {
            IntraDirectionalAngleEdge::Above => {
                let Some(num4_above_right) = self.one_sided_above_coverage(
                    mi_col,
                    mi_row,
                    mi_w,
                    w,
                    max_read,
                    edge_filter_active,
                ) else {
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
                    log2_height,
                    qindex,
                    num4_above_right,
                    use_tcq,
                    self.bit_depth,
                    edge_filter,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_one_sided_above_write",
                    )
                })?;
            }
            IntraDirectionalAngleEdge::Left => {
                let Some(num4_below_left) = self.one_sided_left_coverage(
                    mi_col,
                    mi_row,
                    mi_h,
                    h,
                    max_read,
                    edge_filter_active,
                ) else {
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
                    log2_height,
                    qindex,
                    num4_below_left,
                    use_tcq,
                    self.bit_depth,
                    edge_filter,
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

    /// Attempts to reconstruct a §7.13.2.9 `useIBP` one-sided angular luma leaf:
    /// the §7.13.2.8 primary prediction at `pAngle` blended with the secondary
    /// prediction at `secondAngle = pAngle ± 180` (the OPPOSITE edge), per the
    /// §7.13.2.9 IBP weights. Returns `Ok(true)` when reconstructed bit-exact,
    /// `Ok(false)` when DEFERRED. The caller has already established `useIBP`
    /// (`applyIbp && even angleDelta && plane 0 && one-sided pAngle && MrlIndex 0`).
    ///
    /// A useIBP leaf reads BOTH the primary edge (zone-1 above + above-right /
    /// zone-3 left + below-left) AND the OPPOSITE edge for the secondary prediction
    /// (zone-1 left + below-left / zone-3 above + above-right). Both edges + the
    /// shared corner must be reconstructed; an under-covered edge DEFERS. The
    /// §7.13.2.9 blend is a validated no-op when the leaf's mode is not in the
    /// `is_ibp_enabled` set, but AVM still reads/filters BOTH edges, so the dual
    /// coverage requirement holds regardless.
    #[allow(clippy::too_many_arguments)]
    fn try_reconstruct_one_sided_ibp_angular(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        angle: IntraDirectionalAngle,
        p_angle: i32,
        p_angle_u16: u16,
        block: &LumaCoeffBlock,
        qindex: u32,
        use_tcq: bool,
        mi_w: usize,
        mi_h: usize,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let w = 1u32 << log2_width;
        let h = 1u32 << log2_height;
        let zone1 = p_angle < 90;
        let second_angle_i = if zone1 { p_angle + 180 } else { p_angle - 180 };
        let Ok(second_angle) = u16::try_from(second_angle_i) else {
            return Ok(false);
        };
        let Ok(second_dir_angle) = IntraDirectionalAngle::try_from_p_angle(second_angle) else {
            return Ok(false);
        };
        let Ok(block_size) = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        ) else {
            return Ok(false);
        };
        let Some(primary_edge_filter) =
            self.resolve_one_sided_edge_filter(mi_col, mi_row, w, h, p_angle, true, tile_offset)?
        else {
            return Ok(false);
        };
        let Some(secondary_edge_filter) =
            self.resolve_ibp_secondary_edge_filter(mi_col, mi_row, w, h, p_angle, tile_offset)?
        else {
            return Ok(false);
        };
        let Ok(primary_max_read) = angle.max_one_sided_edge_read_index(block_size) else {
            return Ok(false);
        };
        let Ok(secondary_max_read) = second_dir_angle.max_one_sided_edge_read_index(block_size)
        else {
            return Ok(false);
        };
        let w_us = w as usize;
        let h_us = h as usize;
        let primary_active =
            primary_edge_filter.strength != 0 || primary_edge_filter.corner_opposite.is_some();
        let secondary_active =
            secondary_edge_filter.strength != 0 || secondary_edge_filter.corner_opposite.is_some();
        let (primary_num4, secondary_num4) = if zone1 {
            let Some(primary_num4) = self.one_sided_above_coverage(
                mi_col,
                mi_row,
                mi_w,
                w_us,
                primary_max_read,
                primary_active,
            ) else {
                return Ok(false);
            };
            let Some(secondary_num4) = self.one_sided_left_coverage(
                mi_col,
                mi_row,
                mi_h,
                h_us,
                secondary_max_read,
                secondary_active,
            ) else {
                return Ok(false);
            };
            (primary_num4, secondary_num4)
        } else {
            let Some(primary_num4) = self.one_sided_left_coverage(
                mi_col,
                mi_row,
                mi_h,
                h_us,
                primary_max_read,
                primary_active,
            ) else {
                return Ok(false);
            };
            let Some(secondary_num4) = self.one_sided_above_coverage(
                mi_col,
                mi_row,
                mi_w,
                w_us,
                secondary_max_read,
                secondary_active,
            ) else {
                return Ok(false);
            };
            (primary_num4, secondary_num4)
        };
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        reconstruct_general_intra_one_sided_ibp_luma_block_into(
            &mut self.workspace,
            block,
            p_angle_u16,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            primary_num4,
            primary_edge_filter,
            IbpSecondary {
                second_angle,
                edge_filter: secondary_edge_filter,
                num4_far: secondary_num4,
            },
            use_tcq,
            self.bit_depth,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_luma_one_sided_ibp_write",
            )
        })?;
        Ok(true)
    }

    /// Resolves the §7.13.2.7 step-1 edge-filter / corner-filter inputs for a
    /// one-sided IDIF leaf into an [`OneSidedEdgeFilter`], or `None` to DEFER.
    ///
    /// When `enable_intra_edge_filter == 0` the §7.13.2.7 step is entirely skipped,
    /// so the default no-op filter is returned (the raw §7.13.2.1 edge feeds the
    /// §7.13.2.8 prediction unchanged). Otherwise the per-edge §7.13.2.17 strength
    /// is derived from the REAL §7.13.2.15/16 `is_smooth` neighbour modes recorded
    /// in the coverage map:
    /// * `applyIbp == 1`: `filterTypeAbove = is_smooth(above)`, `filterTypeLeft =
    ///   is_smooth(left)` (the per-edge pick), with the apply-IBP `angleAbove`/
    ///   `angleLeft` ±180 wrap and the `needRight`/`needBottom` ORs;
    /// * `applyIbp == 0`: `filterType = is_smooth(above) | is_smooth(left)` seeded
    ///   into both edges.
    ///
    /// An off-grid neighbour contributes `is_smooth == 0` (matching AVM's `ab ?
    /// is_smooth : 0`). The §7.13.2.14 corner filter fires when `needAbove &&
    /// needLeft && (w + h) >= 24`; its `corner_opposite` is the reconstructed
    /// OPPOSITE-edge `[0]` sample (`LeftCol[0]` zone-1 / `AboveRow[0]` zone-3), which
    /// MUST be covered — DEFER when it is off-grid or uncovered (the corner would
    /// read a fill value). `numPx` clamps the read span to the plane storage
    /// (`Min(w, maxX - x + 1)` / `Min(h, maxY - y + 1)`).
    #[allow(clippy::too_many_arguments)]
    fn resolve_one_sided_edge_filter(
        &self,
        mi_col: usize,
        mi_row: usize,
        w: u32,
        h: u32,
        p_angle: i32,
        apply_ibp: bool,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        if !self.enable_intra_edge_filter {
            return Ok(Some(OneSidedEdgeFilter::default()));
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let smooth = |mode: Option<IntraYMode>| mode.is_some_and(IntraYMode::is_smooth);
        let above_smooth = smooth(
            mi_row
                .checked_sub(1)
                .and_then(|r| coverage.y_mode_at(mi_col, r)),
        );
        let left_smooth = smooth(
            mi_col
                .checked_sub(1)
                .and_then(|c| coverage.y_mode_at(c, mi_row)),
        );
        let zone1 = p_angle < 90;
        let (mut need_above, mut need_left) = if zone1 { (true, false) } else { (false, true) };
        let (mut filter_type_above, mut filter_type_left) = (above_smooth, left_smooth);
        let mut angle_above = p_angle - 90;
        let mut angle_left = p_angle - 180;
        let mut need_right = zone1;
        let mut need_bottom = !zone1;
        if apply_ibp {
            need_above = true;
            need_left = true;
            need_right |= p_angle > 180;
            need_bottom |= p_angle < 90;
            if angle_above > 90 {
                angle_above -= 180;
            }
            if angle_left < -90 {
                angle_left += 180;
            }
        } else {
            let filter_type = above_smooth || left_smooth;
            filter_type_above = filter_type;
            filter_type_left = filter_type;
        }
        let corner_applies = need_above && need_left && (w + h) >= 24;
        let read_edge = if zone1 {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Above,
                filter_type: filter_type_above,
                angle_delta: angle_above,
                need_far: need_right,
            }
        } else {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Left,
                filter_type: filter_type_left,
                angle_delta: angle_left,
                need_far: need_bottom,
            }
        };
        self.assemble_one_sided_edge_filter(
            read_edge,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )
    }

    /// Resolves the §7.13.2.7 step-1 filter for the IBP SECONDARY (opposite) edge of
    /// a `useIBP` one-sided leaf: a zone-1 leaf (primary reads above) blends with a
    /// secondary §7.13.2.8 prediction at `secondAngle = pAngle + 180` reading the
    /// LEFT edge, so the left edge must be filtered with `filterTypeLeft` / `angleLeft`
    /// / `needBottom` — and symmetrically for zone-3. Mirrors the per-edge AVM filter
    /// (`av2_build_intra_predictors_high`, the `apply_ibp` branch filtering BOTH
    /// edges) so the secondary predictor reads the same filtered opposite column AVM
    /// does. Returns `None` (defer) when the corner's opposite sample is uncovered.
    fn resolve_ibp_secondary_edge_filter(
        &self,
        mi_col: usize,
        mi_row: usize,
        w: u32,
        h: u32,
        p_angle: i32,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        if !self.enable_intra_edge_filter {
            return Ok(Some(OneSidedEdgeFilter::default()));
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let smooth = |mode: Option<IntraYMode>| mode.is_some_and(IntraYMode::is_smooth);
        let above_smooth = smooth(
            mi_row
                .checked_sub(1)
                .and_then(|r| coverage.y_mode_at(mi_col, r)),
        );
        let left_smooth = smooth(
            mi_col
                .checked_sub(1)
                .and_then(|c| coverage.y_mode_at(c, mi_row)),
        );
        let zone1 = p_angle < 90;
        let mut angle_above = p_angle - 90;
        let mut angle_left = p_angle - 180;
        if angle_above > 90 {
            angle_above -= 180;
        }
        if angle_left < -90 {
            angle_left += 180;
        }
        let need_right = zone1 || p_angle > 180;
        let need_bottom = !zone1 || p_angle < 90;
        let corner_applies = (w + h) >= 24;
        let secondary_edge = if zone1 {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Left,
                filter_type: left_smooth,
                angle_delta: angle_left,
                need_far: need_bottom,
            }
        } else {
            OneSidedEdgeSpec {
                orientation: EdgeOrientation::Above,
                filter_type: above_smooth,
                angle_delta: angle_above,
                need_far: need_right,
            }
        };
        self.assemble_one_sided_edge_filter(
            secondary_edge,
            corner_applies,
            w,
            h,
            mi_col,
            mi_row,
            tile_offset,
        )
    }

    /// Assembles a [`OneSidedEdgeFilter`] for one edge (above or left) from its
    /// resolved §7.13.2.7 spec: the §7.13.2.17 strength, the §7.13.2.7 `numPx`
    /// storage clamp, and the §7.13.2.14 corner's opposite-edge `[0]` sample read
    /// diagonally — an above edge reads `LeftCol[0] = CurrFrame[y][x-1]`, a left
    /// edge reads `AboveRow[0] = CurrFrame[y-1][x]`.
    /// Shared by the read-edge ([`Self::resolve_one_sided_edge_filter`]) and the
    /// IBP secondary-edge ([`Self::resolve_ibp_secondary_edge_filter`]) resolution.
    /// Returns `None` (defer) when the corner fires but its opposite sample is
    /// off-grid or uncovered.
    #[allow(clippy::too_many_arguments)]
    fn assemble_one_sided_edge_filter(
        &self,
        edge: OneSidedEdgeSpec,
        corner_applies: bool,
        w: u32,
        h: u32,
        mi_col: usize,
        mi_row: usize,
        tile_offset: ByteOffset,
    ) -> Result<Option<OneSidedEdgeFilter>> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let (strength_a, strength_b, primary, secondary) = match edge.orientation {
            EdgeOrientation::Above => (w, h, w, h),
            EdgeOrientation::Left => (h, w, h, w),
        };
        let strength = intra_edge_filter_strength(
            strength_a,
            strength_b,
            u8::from(edge.filter_type),
            edge.angle_delta,
        );
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        let plane = self.workspace.plane(PlaneId::Y)?;
        let storage = plane.storage_size();
        let (origin, max_axis) = match edge.orientation {
            EdgeOrientation::Above => (x, storage.width()),
            EdgeOrientation::Left => (y, storage.height()),
        };
        let in_block = (max_axis.saturating_sub(origin)).min(primary as usize);
        let num_px = in_block
            .checked_add(if edge.need_far { secondary as usize } else { 0 })
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_one_sided_edge_filter_numpx_overflow",
                )
            })?;
        let corner_opposite = if corner_applies {
            let (opp_col, opp_row, sample_x, sample_y) = match edge.orientation {
                EdgeOrientation::Above => (
                    mi_col.checked_sub(1),
                    Some(mi_row),
                    x.checked_sub(1),
                    Some(y),
                ),
                EdgeOrientation::Left => (
                    Some(mi_col),
                    mi_row.checked_sub(1),
                    Some(x),
                    y.checked_sub(1),
                ),
            };
            let (Some(opp_col), Some(opp_row)) = (opp_col, opp_row) else {
                return Ok(None);
            };
            if coverage.off_grid(opp_col, opp_row) || !coverage.is_covered(opp_col, opp_row) {
                return Ok(None);
            }
            let (Some(sx), Some(sy)) = (sample_x, sample_y) else {
                return Ok(None);
            };
            Some(
                self.workspace
                    .reconstructed_sample(PlaneId::Y, sx, sy)?
                    .to_u16(),
            )
        } else {
            None
        };
        Ok(Some(OneSidedEdgeFilter {
            strength,
            num_px,
            corner_opposite,
        }))
    }

    /// zone-1 above-edge coverage guard. Verifies the §7.13.2.1 corner unit
    /// `(mi_col - 1, mi_row - 1)`, the above row `mi_row - 1` over the block's
    /// `mi_w` columns, AND every above-right sample the §7.13.2.7 edge filter /
    /// §7.13.2.8 projection consume are all reconstructed by this sink, then returns
    /// the §7.13.2.1 `num4AboveRight` (in luma 4x4 units) to pass to the
    /// reconstructor. Returns `None` (defer) when any required unit is off-grid or
    /// uncovered.
    ///
    /// AVM `has_top_right` (`reconintra.c`) caps the available above-right at
    /// `px_top_right = Min(consecutively-coded MI units, tx_size_wide_unit)`, where
    /// `tx_size_wide_unit == mi_w`, then `build_intra_predictors` PADS the remaining
    /// above-right slots (up to `txwpx + txhpx`) with the last real sample BEFORE the
    /// §7.13.2.18 edge filter smooths them. So when the edge filter is active it
    /// consumes the FULL `mi_w` above-right span (NOT just the `max_read` projection
    /// reach): we then require ALL `mi_w` above-right units COVERED so our real
    /// above-right and pad boundary match AVM's exactly (`edge_filter_active`).
    /// When the edge filter is a no-op, only the §7.13.2.8 projection reads matter,
    /// so the `max_read`-bounded covered span suffices.
    ///
    /// `width` is the block's sample WIDTH (`Tx_Width`): the in-block above row is
    /// `AboveRow[0..width)`; `max_read < width` means the projection stays in-block.
    fn one_sided_above_coverage(
        &self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        width: usize,
        max_read: usize,
        edge_filter_active: bool,
    ) -> Option<usize> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let above = mi_row.checked_sub(1)?;
        let corner = mi_col.checked_sub(1)?;
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if !covered(corner, above) {
            return None;
        }
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return None;
        }
        let right_edge_mi = mi_col.checked_add(mi_w)?;
        let mut num4_above_right = 0usize;
        for offset in 0..mi_w {
            let unit = right_edge_mi.checked_add(offset)?;
            if covered(unit, above) {
                num4_above_right += 1;
            } else {
                break;
            }
        }
        if edge_filter_active {
            if num4_above_right < mi_w {
                return None;
            }
        } else {
            let covered_extent = width.checked_add(num4_above_right.checked_mul(MI_SIZE)?)?;
            if max_read >= width && max_read >= covered_extent {
                return None;
            }
        }
        Some(num4_above_right)
    }

    /// zone-3 left-edge coverage guard, the symmetric mirror of
    /// [`Self::one_sided_above_coverage`]. Verifies the §7.13.2.1 corner unit
    /// `(mi_col - 1, mi_row)` (the top of the left column for a `haveAbove == 0`
    /// position, or the diagonal corner generally — both reduce to the left-column
    /// reconstructed sample), the left column `mi_col - 1` over the block's `mi_h`
    /// rows, AND the below-left samples the §7.13.2.7 edge filter / §7.13.2.8
    /// projection consume, then returns the §7.13.2.1 `num4BelowLeft`. Returns `None`
    /// (defer) when any required unit is off-grid or uncovered.
    ///
    /// AVM `has_bottom_left` caps the below-left at `px_bottom_left = Min(coded MI
    /// units, tx_size_high_unit == mi_h)`, then pads; the active §7.13.2.18 edge
    /// filter consumes the full `mi_h` below-left span. So when the edge filter is
    /// active we require ALL `mi_h` below-left units COVERED (pad boundary matches
    /// AVM); when it is a no-op only the `max_read`-bounded projection reads matter.
    ///
    /// `height` is the block's sample HEIGHT (`Tx_Height`): the in-block left column
    /// is `LeftCol[0..height)`; `max_read < height` means the projection stays
    /// in-block.
    fn one_sided_left_coverage(
        &self,
        mi_col: usize,
        mi_row: usize,
        mi_h: usize,
        height: usize,
        max_read: usize,
        edge_filter_active: bool,
    ) -> Option<usize> {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let left = mi_col.checked_sub(1)?;
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r)) {
            return None;
        }
        let bottom_edge_mi = mi_row.checked_add(mi_h)?;
        let mut num4_below_left = 0usize;
        for offset in 0..mi_h {
            let unit = bottom_edge_mi.checked_add(offset)?;
            if covered(left, unit) {
                num4_below_left += 1;
            } else {
                break;
            }
        }
        if edge_filter_active {
            if num4_below_left < mi_h {
                return None;
            }
        } else {
            let covered_extent = height.checked_add(num4_below_left.checked_mul(MI_SIZE)?)?;
            if max_read >= height && max_read >= covered_extent {
                return None;
            }
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
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return Ok(());
        };
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if let Some(mode) = leaf_y_mode {
            self.coverage[Self::coverage_index(PlaneId::Y)]
                .record_y_mode(mi_col, mi_row, mi_w, mi_h, mode);
        }
        if !residual_is_reconstructable(block, fsc_mode) {
            return Ok(());
        }
        if is_intrabc {
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
            _ => return Ok(()),
        }
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
        if !residual_is_reconstructable(block, false) {
            return Ok(());
        }
        let (mi_col, mi_row) = (x / MI_SIZE, y / MI_SIZE);
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if !self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h) {
            return Ok(());
        }
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
            return Ok(());
        }
        if source.size() != target.size() {
            return Ok(());
        }
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let src_mi_col = source.x() / MI_SIZE;
        let src_mi_row = source.y() / MI_SIZE;
        let src_mi_w = (source.x() + source.width()).div_ceil(MI_SIZE) - src_mi_col;
        let src_mi_h = (source.y() + source.height()).div_ceil(MI_SIZE) - src_mi_row;
        if !coverage.region_fully_covered(src_mi_col, src_mi_row, src_mi_w, src_mi_h) {
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
    let enable_ibp = sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp);
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
/// and propagates every other error (including the now-expected `Ok` when the walk
/// runs to completion), so a regression that re-introduces an earlier frontier fails
/// the test loudly. The §7.20.4 `live_frame_samples_unpopulated` gate is where the
/// parse-only public-decode path stops (decoded CurrFrame / CdefFrame samples are
/// still unpopulated for storage-backed FilterClass retention).
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
    let real_ist = block.intra_ist.is_some_and(|ist| ist.sec_tx_type != 0);
    !(real_ist || fsc_mode)
}

#[cfg(test)]
#[path = "recon_tests.rs"]
mod tests;
