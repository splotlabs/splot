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
use splot_recon::math::{approx_divide, clip3, resolve_division, round2, round2_signed};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, IntraCardinalDirection, IntraDirectionalAngle,
    IntraDirectionalAngleEdge, IntraRectBlockSize, IntraSmoothMode, PlaneId, PlaneRect,
    ReconSample, predict_intra_dc_rect_value, predict_intra_dc_subsampled_rect_value,
};

use crate::Result;
use crate::runtime_minimal::inter::mv_scaling::PlaneScaling;
use crate::runtime_minimal_recon::{
    IbpSecondary, MHCCP_BITS, MHCCP_PARAM_COUNT, MhccpRefs, OneSidedAboveMrl, OneSidedEdgeFilter,
    TwoSidedMiddleEdgeFilters, derive_mhccp_params, mul_fixed32_adapt, new_general_intra_workspace,
    reconstruct_general_intra_block_rect_into,
    reconstruct_general_intra_cardinal_mrl_luma_block_into,
    reconstruct_general_intra_cardinal_neighbour_block_into,
    reconstruct_general_intra_chroma_block_into,
    reconstruct_general_intra_chroma_smooth_available_edges_into,
    reconstruct_general_intra_luma_dc_rect_block_with_ist_into,
    reconstruct_general_intra_luma_paeth_neighbour_block_into,
    reconstruct_general_intra_luma_smooth_rect_block_into,
    reconstruct_general_intra_middle_neighbour_rect_block_into,
    reconstruct_general_intra_mrl_secondary_above_block_into,
    reconstruct_general_intra_mrl_secondary_left_block_into,
    reconstruct_general_intra_one_sided_ibp_luma_block_into,
    reconstruct_general_intra_one_sided_left_neighbour_block_into,
    reconstruct_general_intra_one_sided_neighbour_block_into,
    reconstruct_general_intra_two_sided_middle_luma_mrl_block_into,
    reconstruct_intrabc_block_residual_rect_into,
};
use crate::tile_payload::{
    CflIndex, CflParams, FrameCdfSubset, GeneralIntraResidualError, IntraYMode, LumaCoeffBlock,
    LumaTransformTypeContext, SupportedChromaMode, SupportedDirectionalLumaMode,
    reconstruct_general_intra_block_rect_with_prediction,
};

use super::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use splot_core::span::ByteOffset;

/// AV2 §3 `MI_SIZE`: one mode-info unit spans four samples.
const MI_SIZE: usize = 4;
const CFL_FILTERS_420: [[[i64; 3]; 3]; 3] = [
    [[0, 0, 0], [0, 2, 2], [0, 2, 2]],
    [[0, 0, 0], [1, 2, 1], [1, 2, 1]],
    [[0, 1, 0], [1, 4, 1], [0, 1, 0]],
];
const CFL_ALPHA_SHIFT: u32 = 11;
const CFL_ALPHA_SCALE: i64 = 32;
const CFL_DERIVED_ALPHA_SHIFT: u8 = 8;
const NUM_REF_SAM_CFL: usize = 8;

pub(in crate::runtime_minimal) struct Ac0ej3SelectableIntraRegion {
    pub(in crate::runtime_minimal) sink: WienerNsLrReconSink<u16>,
    pub(in crate::runtime_minimal) frame_cdfs: FrameCdfSubset,
}

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
    /// The leaf's §5.20.5.5 `MrlIndex` (the multi-reference-line distance). `0` is the
    /// immediate edge; `> 0` selects a farther reference line. A non-zero `MrlIndex`
    /// nudges the §7.13.2.8 `pAngle` off the raw `Mode_To_Angle` by
    /// `Mrl_Index_To_Delta[MrlIndex]`, so a cardinal `V_PRED` / `H_PRED` leaf becomes a
    /// genuine directional projection on the offset line — routed to the one-sided
    /// angular path. The verified MRL admission is the ZONE-1 (above) projection; the
    /// zone-3, zone-2 middle, and `MrlIndex`-3 exact-cardinal MRL cases still DEFER.
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
    /// The optional §5.20.5.5 `mrl_sec_index`. `Some(1)` means a non-4x4
    /// `MrlIndex > 0` directional luma block averages the primary MRL prediction
    /// with a second prediction from the immediate reference line.
    pub(in crate::runtime_minimal) mrl_sec_index: Option<u8>,
    pub(in crate::runtime_minimal) chroma_mode: Option<SupportedChromaMode>,
    pub(in crate::runtime_minimal) cfl_params: Option<CflParams>,
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
    /// The §5.4.4 `cfl_ds_filter_index` sequence value used by §7.13.5 luma
    /// downsampling; value `3` aliases filter `0`.
    cfl_ds_filter_index: u8,
    luma_width: usize,
    luma_height: usize,
    /// The §5.20.5.5 superblock side in luma 4x4 MI units (`mib_size`: 16/32/64 for a
    /// 64/128/256 superblock). Drives the §7.13.2.1 `is_sb_boundary == (mi_row %
    /// mib_size == 0)` MRL above-line rule (`aboveMrlIndex == sbBoundary ? 0 : MrlIndex`).
    sb_mib: usize,
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
    pending_chroma_transforms: Vec<PendingChromaTransform>,
    deblock_blocks: Vec<super::super::deblock::DeblockBlock>,
    chroma_deblock_blocks: [Vec<super::super::deblock::DeblockBlock>; 2],
    cdef_grid: Option<super::super::cdef::CdefUnitGrid>,
    ccso_grid: Option<super::super::ccso::CcsoUnitGrid>,
    tx_skip_grid: Option<super::WienerNsLrTxSkipGrid>,
    lr_source_blocks: Vec<crate::tile_payload::WienerNsLrSourceBlock>,
    lr_unit_filters: Vec<crate::tile_payload::WienerNsLrUnitFilter>,
    /// Per-luma-MI-unit AV2 §7.13.2.1 far-edge availability (`num4AboveRight`,
    /// `num4BelowLeft`, in luma 4x4 units), recorded at PER-TRANSFORM granularity by
    /// [`Self::record_block_decoded_far_edge`] — each transform's own
    /// `tx_size_wide_unit` / `tx_size_high_unit` and `(row_off, col_off)` within the
    /// coding block, faithful to AVM `has_top_right` / `has_bottom_left`
    /// (`av2/common/reconintra.c`), NOT the enclosing partition block's counts.
    /// `far_edge_avail[mi_row * cols + mi_col]` is `None` until a transform is
    /// recorded into that unit. This is the durable AVM-availability infrastructure:
    /// the sink can query the real AVM above-right / below-left count per transform
    /// (see [`Self::block_decoded_far_edge`]) instead of re-deriving it from the
    /// conservative coverage map. Recorded for EVERY luma transform the walk decodes;
    /// the (still coverage-gated) directional admission path does not yet consume it
    /// (the `num4AboveRight > 0` read-or-pad is not yet proven bit-exact against the
    /// AVM oracle, so admitting on it alone would risk a confident-wrong sample).
    far_edge_avail: FarEdgeAvailGrid,
    /// DIAGNOSTIC-ONLY full-reconstruction mode (set by [`Self::into_full_recon`],
    /// driven only by the `SPLOT_AC0EJ3_FULL_RECON` ignored harness). When `true`,
    /// [`Self::reconstruct_luma_transform`] DROPS the conservative §7.13.2 edge-coverage
    /// gates and reconstructs EVERY luma leaf in decode order, sourcing the §7.13.2.1
    /// `num4AboveRight` / `num4BelowLeft` read-or-pad bound from the per-transform
    /// [`Self::far_edge_avail`] (AVM `has_top_right` / `has_bottom_left`) instead of the
    /// coverage map. NEVER changes the shipped (`false`) path; `false` for every shipped
    /// sink (the 16 ignored oracle-pin tests still drive the gated sink).
    full_recon: bool,
    /// DIAGNOSTIC-ONLY decode-order log of luma leaves, appended by
    /// [`Self::reconstruct_luma_transform`] only when [`Self::full_recon`] is set (empty
    /// otherwise). The `SPLOT_AC0EJ3_FULL_RECON` harness replays it to find the FIRST
    /// decode-order block whose samples diverge from the AVM pre-filter oracle.
    full_recon_luma_log: Vec<FullReconLumaLeaf>,
}

/// One decode-order luma leaf recorded by the full-reconstruction diagnostic (see
/// [`WienerNsLrReconSink::full_recon_luma_log`]): sample-space origin `(x, y)`, MI
/// origin `(mi_col, mi_row)`, sample `(width, height)`, a §7.13.2 mode label, and
/// `written` (a real predictor vs. the workspace fill value left for an unwired mode).
/// Only the test harness reads the fields; production records but never inspects them
/// (the log stays empty since `full_recon` is always `false`).
#[derive(Clone, Copy)]
#[cfg_attr(not(test), allow(dead_code))]
pub(in crate::runtime_minimal) struct FullReconLumaLeaf {
    pub mi_col: usize,
    pub mi_row: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub mode: &'static str,
    pub written: bool,
}

#[derive(Clone)]
struct PendingChromaTransform {
    plane_id: PlaneId,
    chroma_tx: usize,
    x: usize,
    y: usize,
    block: LumaCoeffBlock,
    chroma_mode: Option<SupportedChromaMode>,
    angle_delta_y: i8,
    cfl_params: Option<CflParams>,
    num4_above_right: usize,
    num4_below_left: usize,
    qindex: u32,
    tile_offset: ByteOffset,
}

/// Row-major per-luma-MI AV2 §7.13.2.1 far-edge availability grid, populated from
/// the live §5.20.2.3 `BlockDecoded` state threaded through the tree-walk callback.
struct FarEdgeAvailGrid {
    cols: usize,
    rows: usize,
    /// `Some((num4_above_right, num4_below_left))` per luma MI unit once a block has
    /// been decoded there; `None` for not-yet-decoded units.
    avail: Vec<Option<(u32, u32)>>,
}

impl FarEdgeAvailGrid {
    fn new(width_samples: usize, height_samples: usize) -> Self {
        let cols = width_samples.div_ceil(MI_SIZE);
        let rows = height_samples.div_ceil(MI_SIZE);
        let cells = cols.saturating_mul(rows);
        Self {
            cols,
            rows,
            avail: vec![None; cells],
        }
    }

    const fn off_grid(&self, mi_col: usize, mi_row: usize) -> bool {
        mi_col >= self.cols || mi_row >= self.rows
    }

    /// Records the §7.13.2.1 far-edge counts over every IN-GRID MI unit of the
    /// `mi_w` x `mi_h` TRANSFORM at `(mi_col, mi_row)`. The counts are the same value
    /// for every unit of the transform (a per-transform AVM `has_top_right` /
    /// `has_bottom_left` read), so a later query at any covered unit returns the
    /// transform's availability.
    fn record(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
        num4_above_right: u32,
        num4_below_left: u32,
    ) {
        for r in mi_row..mi_row.saturating_add(mi_h) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if !self.off_grid(c, r)
                    && let Some(slot) = self.avail.get_mut(r * self.cols + c)
                {
                    *slot = Some((num4_above_right, num4_below_left));
                }
            }
        }
    }

    /// The recorded per-transform §7.13.2.1 far-edge availability
    /// (`num4AboveRight`, `num4BelowLeft`) for the luma MI unit at `(mi_col,
    /// mi_row)`, or `None` when no transform has been recorded there yet.
    fn get(&self, mi_col: usize, mi_row: usize) -> Option<(u32, u32)> {
        if self.off_grid(mi_col, mi_row) {
            return None;
        }
        self.avail
            .get(mi_row * self.cols + mi_col)
            .copied()
            .flatten()
    }
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
    y_modes: Vec<Option<IntraYMode>>,
    chroma_modes: Vec<Option<SupportedChromaMode>>,
}

impl PlaneCoverage {
    fn new(width_samples: usize, height_samples: usize) -> Self {
        let cols = width_samples.div_ceil(MI_SIZE);
        let rows = height_samples.div_ceil(MI_SIZE);
        let cells = cols.saturating_mul(rows);
        Self {
            cols,
            rows,
            covered: vec![false; cells],
            y_modes: vec![None; cells],
            chroma_modes: vec![None; cells],
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

    fn fully_covered(&self) -> bool {
        self.covered.iter().all(|covered| *covered)
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
        record_plane_mode(
            self.cols,
            self.rows,
            &mut self.y_modes,
            (mi_col, mi_row),
            (mi_w, mi_h),
            mode,
        );
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

    fn record_chroma_mode(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
        mode: SupportedChromaMode,
    ) {
        record_plane_mode(
            self.cols,
            self.rows,
            &mut self.chroma_modes,
            (mi_col, mi_row),
            (mi_w, mi_h),
            mode,
        );
    }

    fn chroma_mode_at(&self, mi_col: usize, mi_row: usize) -> Option<SupportedChromaMode> {
        if self.off_grid(mi_col, mi_row) {
            return None;
        }
        self.chroma_modes
            .get(mi_row * self.cols + mi_col)
            .copied()
            .flatten()
    }
}

fn record_plane_mode<T: Copy>(
    cols: usize,
    rows: usize,
    slots: &mut [Option<T>],
    origin: (usize, usize),
    size: (usize, usize),
    mode: T,
) {
    let (mi_col, mi_row) = origin;
    let (mi_w, mi_h) = size;
    for r in mi_row..mi_row.saturating_add(mi_h) {
        for c in mi_col..mi_col.saturating_add(mi_w) {
            if c < cols
                && r < rows
                && let Some(slot) = slots.get_mut(r * cols + c)
            {
                *slot = Some(mode);
            }
        }
    }
}

/// AV2 §7.13.2.17 intra edge filter strength selection process. Returns the
/// edge-filter strength `0..=3` for a `w` x `h` transform, `filter_type` (0 or 1,
/// from §7.13.2.15/16 — `1` when the relevant neighbour uses a smooth mode), and
/// `delta` (the §7.13.2.7 `angleAbove = pAngle - 90` / `angleLeft = pAngle - 180`).
/// Strength `0` means `av2_filter_intra_edge` is a no-op, so the §7.13.2.8
/// prediction over the UNFILTERED edge is bit-exact. Transcribed VERBATIM from the
/// committed spec mirror `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-17`.
#[allow(clippy::if_same_then_else)]
pub(in crate::runtime_minimal) fn intra_edge_filter_strength(
    w: u32,
    h: u32,
    filter_type: u8,
    delta: i32,
) -> u8 {
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
    /// `u16` for the 10-bit ac0ej3 stream.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn new(
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
        quant_reconstructable: bool,
        enable_ibp: bool,
        enable_intra_edge_filter: bool,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    ) -> Result<Self> {
        Ok(Self::with_workspace(
            new_general_intra_workspace::<T>(luma_width, luma_height, bit_depth)?,
            luma_width,
            luma_height,
            bit_depth,
            quant_reconstructable,
            enable_ibp,
            enable_intra_edge_filter,
            cfl_ds_filter_index,
            sb_mib,
        ))
    }

    /// Wraps an already-reconstructed workspace so the caller can run the
    /// shared §7.2 final filter chain over it: feed the filter state via the
    /// `set_*` / deblock-record methods, then finish with
    /// [`Self::apply_final_filters_and_freeze`]. The intra-reconstruction
    /// state (coverage, IntrABC, IBP flags) stays inert.
    pub(in crate::runtime_minimal) fn for_final_filtering(
        workspace: CurrentFrameWorkspace<T>,
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
    ) -> Self {
        Self::with_workspace(
            workspace,
            luma_width,
            luma_height,
            bit_depth,
            false,
            false,
            false,
            0,
            16,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_workspace(
        workspace: CurrentFrameWorkspace<T>,
        luma_width: usize,
        luma_height: usize,
        bit_depth: BitDepth,
        quant_reconstructable: bool,
        enable_ibp: bool,
        enable_intra_edge_filter: bool,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    ) -> Self {
        let chroma_width = luma_width.div_ceil(2);
        let chroma_height = luma_height.div_ceil(2);
        Self {
            workspace,
            bit_depth,
            quant_reconstructable,
            enable_ibp,
            enable_intra_edge_filter,
            cfl_ds_filter_index,
            luma_width,
            luma_height,
            sb_mib,
            coverage: [
                PlaneCoverage::new(luma_width, luma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
            ],
            reconstructed_luma_4x4: 0,
            reconstructed_chroma_4x4: 0,
            pending_intrabc_predictions: Vec::new(),
            pending_chroma_transforms: Vec::new(),
            deblock_blocks: Vec::new(),
            chroma_deblock_blocks: [Vec::new(), Vec::new()],
            cdef_grid: None,
            ccso_grid: None,
            tx_skip_grid: None,
            lr_source_blocks: Vec::new(),
            lr_unit_filters: Vec::new(),
            far_edge_avail: FarEdgeAvailGrid::new(luma_width, luma_height),
            full_recon: false,
            full_recon_luma_log: Vec::new(),
        }
    }

    /// Freezes the sink workspace into a decoded frame for runtime output.
    #[allow(dead_code)]
    pub(in crate::runtime_minimal) fn into_frame(mut self) -> Result<DecodedFrame<T>> {
        self.replay_pending_chroma_transforms()?;
        Ok(self.workspace.freeze()?)
    }

    /// Retains decoded transform geometry used by the post-tile deblocking pass.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn record_deblock_block(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        block_col: usize,
        block_row: usize,
        n4w: usize,
        n4h: usize,
        luma_tx: usize,
        chroma_tx: Option<usize>,
        qindex: u32,
        skip: bool,
    ) {
        self.deblock_blocks
            .push(super::super::deblock::DeblockBlock {
                r: mi_row,
                c: mi_col,
                block_r: block_row,
                block_c: block_col,
                chroma_base_r: mi_row,
                chroma_base_c: mi_col,
                n4w,
                n4h,
                luma_tx,
                chroma_tx,
                qindex,
                skip,
            });
    }

    pub(in crate::runtime_minimal) fn record_chroma_deblock_block(
        &mut self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        chroma_tx: usize,
        qindex: u32,
    ) {
        let Some((plane_index, block)) =
            chroma_transform_deblock_block(plane_id, x, y, chroma_tx, qindex)
        else {
            return;
        };
        self.chroma_deblock_blocks[plane_index].push(block);
    }

    /// Hands over externally accumulated § 7.17 deblock geometry (luma list +
    /// per-plane chroma lists) for the final filter chain.
    pub(in crate::runtime_minimal) fn set_deblock_blocks(
        &mut self,
        luma: Vec<super::super::deblock::DeblockBlock>,
        chroma: [Vec<super::super::deblock::DeblockBlock>; 2],
    ) {
        self.deblock_blocks = luma;
        self.chroma_deblock_blocks = chroma;
    }

    /// Retains the selectable walk's parsed CDEF unit grid for the final filter
    /// chain. The plain [`Self::into_frame`] freeze intentionally ignores this
    /// state so existing prefilter differentials keep observing `CurrFrame`
    /// before in-loop filters.
    pub(in crate::runtime_minimal) fn set_cdef_grid(
        &mut self,
        grid: Option<super::super::cdef::CdefUnitGrid>,
    ) {
        self.cdef_grid = grid;
    }

    /// Retains the selectable walk's parsed CCSO block-enable grid for the final
    /// filter chain.
    pub(in crate::runtime_minimal) fn set_ccso_grid(
        &mut self,
        grid: Option<super::super::ccso::CcsoUnitGrid>,
    ) {
        self.ccso_grid = grid;
    }

    /// Retains the frame's §7.20.4 `LrTxSkip` grid for PC-Wiener subclass
    /// derivation during final luma restoration.
    pub(in crate::runtime_minimal) fn set_tx_skip_grid(
        &mut self,
        grid: Option<super::WienerNsLrTxSkipGrid>,
    ) {
        self.tx_skip_grid = grid;
    }

    /// Retains active loop-restoration source blocks from the full selectable
    /// walk for final LR filtering.
    pub(in crate::runtime_minimal) fn set_lr_source_blocks(
        &mut self,
        blocks: Vec<crate::tile_payload::WienerNsLrSourceBlock>,
    ) {
        self.lr_source_blocks = blocks;
    }

    /// Retains entropy-coded per-unit Wiener NS filters from the full selectable
    /// walk for final LR filtering.
    pub(in crate::runtime_minimal) fn set_lr_unit_filters(
        &mut self,
        filters: Vec<crate::tile_payload::WienerNsLrUnitFilter>,
    ) {
        self.lr_unit_filters = filters;
    }

    /// Completes the intra reconstruction (pending chroma replays + the
    /// full-recon coverage check) before the final filter chain runs.
    pub(in crate::runtime_minimal) fn finish_intra_reconstruction(
        &mut self,
        offset: ByteOffset,
    ) -> Result<()> {
        self.replay_pending_chroma_transforms()?;
        self.ensure_full_recon_coverage_complete(offset)
    }

    /// Runs the §7.2 in-loop filter chain (deblock → CDEF → CCSO → LR) over
    /// the reconstructed workspace and freezes the filtered frame.
    pub(in crate::runtime_minimal) fn into_filtered_frame(
        mut self,
        core: &splot_core::headers::frame::FrameHeaderCore,
        deblock_quant_deltas: super::super::deblock::DeblockQuantDeltas,
        offset: ByteOffset,
    ) -> Result<DecodedFrame<T>> {
        dump_prefilter_frame_for_diagnostics(&self.workspace, self.luma_width, self.luma_height);
        let mi_rows = self.luma_height.div_ceil(MI_SIZE);
        let mi_cols = self.luma_width.div_ceil(MI_SIZE);
        if let Some(filter) = core.deblocking_filter_params
            && filter.apply_deblocking_filter != [false; 4]
        {
            super::super::deblock::deblock_general_intra_frame(
                &mut self.workspace,
                &self.deblock_blocks,
                [
                    &self.chroma_deblock_blocks[0],
                    &self.chroma_deblock_blocks[1],
                ],
                mi_rows,
                mi_cols,
                filter,
                deblock_quant_deltas,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_deblock_filter",
                )
            })?;
        }
        let lr_plane_active = |plane_index: usize| {
            core.lr_params.as_ref().is_some_and(|lr| {
                lr.planes.get(plane_index).is_some_and(|plane| {
                    plane.restoration_type
                        == splot_core::headers::frame::FrameRestorationType::WienerNonsep
                })
            }) && self
                .lr_source_blocks
                .iter()
                .any(|block| block.plane == plane_index)
        };
        let luma_lr_active = lr_plane_active(PlaneId::Y.index());
        let u_lr_active = lr_plane_active(PlaneId::U.index());
        let v_lr_active = lr_plane_active(PlaneId::V.index());
        let any_lr_active = luma_lr_active || u_lr_active || v_lr_active;
        let deblocked_luma = if any_lr_active || self.ccso_grid.is_some() {
            self.plane_snapshot(
                PlaneId::Y,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_luma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let deblocked_u = if u_lr_active {
            self.plane_snapshot(
                PlaneId::U,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let deblocked_v = if v_lr_active {
            self.plane_snapshot(
                PlaneId::V,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_deblocked_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let cdef_skip_grid = self.cdef_skip_grid(core, mi_rows, mi_cols, offset)?;
        if let (Some(grid), Some(strengths)) = (
            self.cdef_grid.as_ref(),
            super::super::cdef::cdef_frame_strengths(core),
        ) {
            super::super::cdef::cdef_general_intra_frame_indexed(
                &mut self.workspace,
                &strengths,
                grid,
                cdef_skip_grid.as_ref(),
                mi_rows,
                mi_cols,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_cdef_filter",
                )
            })?;
        }
        if let Some(grid) = self.ccso_grid.as_ref() {
            super::super::ccso::ccso_frame(
                &mut self.workspace,
                &deblocked_luma,
                core,
                grid,
                mi_rows,
                mi_cols,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    offset,
                    "unsupported_wienerns_lr_selectable_transform_records_ccso_filter",
                )
            })?;
        }
        let cdef_luma = if any_lr_active {
            self.plane_snapshot(
                PlaneId::Y,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_luma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let cdef_u = if u_lr_active {
            self.plane_snapshot(
                PlaneId::U,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let cdef_v = if v_lr_active {
            self.plane_snapshot(
                PlaneId::V,
                offset,
                "unsupported_wienerns_lr_selectable_transform_records_cdef_chroma_snapshot",
            )?
        } else {
            Vec::new()
        };
        let lr_source_blocks = core::mem::take(&mut self.lr_source_blocks);
        let lr_unit_filters = core::mem::take(&mut self.lr_unit_filters);
        self.apply_luma_lr(
            core,
            offset,
            &lr_source_blocks,
            &lr_unit_filters,
            &deblocked_luma,
            &cdef_luma,
        )?;
        self.apply_chroma_lr(
            core,
            offset,
            PlaneId::U,
            &lr_source_blocks,
            &lr_unit_filters,
            &deblocked_u,
            &cdef_u,
            &deblocked_luma,
            &cdef_luma,
        )?;
        self.apply_chroma_lr(
            core,
            offset,
            PlaneId::V,
            &lr_source_blocks,
            &lr_unit_filters,
            &deblocked_v,
            &cdef_v,
            &deblocked_luma,
            &cdef_luma,
        )?;
        Ok(self.workspace.freeze()?)
    }

    fn plane_snapshot(
        &self,
        plane: PlaneId,
        offset: ByteOffset,
        reason: &'static str,
    ) -> Result<Vec<u16>> {
        self.workspace
            .samples(plane)
            .map_err(|_| wienerns_lr_selectable_transform_record_error_reason(offset, reason))
            .map(|samples| samples.iter().map(|sample| sample.to_u16()).collect())
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

    #[allow(clippy::too_many_arguments)]
    fn defer_chroma_transform(
        &mut self,
        plane_id: PlaneId,
        chroma_tx: usize,
        x: usize,
        y: usize,
        block: &LumaCoeffBlock,
        chroma_mode: Option<SupportedChromaMode>,
        angle_delta_y: i8,
        cfl_params: Option<CflParams>,
        num4_above_right: usize,
        num4_below_left: usize,
        qindex: u32,
        tile_offset: ByteOffset,
    ) {
        self.pending_chroma_transforms.push(PendingChromaTransform {
            plane_id,
            chroma_tx,
            x,
            y,
            block: block.clone(),
            chroma_mode,
            angle_delta_y,
            cfl_params,
            num4_above_right,
            num4_below_left,
            qindex,
            tile_offset,
        });
    }

    fn replay_pending_chroma_transforms(&mut self) -> Result<()> {
        loop {
            let pending = core::mem::take(&mut self.pending_chroma_transforms);
            if pending.is_empty() {
                return Ok(());
            }
            let before = self.reconstructed_chroma_4x4;
            for transform in pending {
                self.reconstruct_chroma_transform(
                    transform.plane_id,
                    transform.chroma_tx,
                    transform.x,
                    transform.y,
                    &transform.block,
                    transform.chroma_mode,
                    transform.angle_delta_y,
                    transform.cfl_params,
                    transform.num4_above_right,
                    transform.num4_below_left,
                    transform.qindex,
                    transform.tile_offset,
                )?;
            }
            if self.pending_chroma_transforms.is_empty() {
                return Ok(());
            }
            if self.reconstructed_chroma_4x4 == before {
                return Ok(());
            }
        }
    }

    fn ensure_full_recon_coverage_complete(&self, offset: ByteOffset) -> Result<()> {
        if !self.full_recon {
            return Ok(());
        }
        if self.pending_chroma_transforms.is_empty()
            && self.coverage.iter().all(PlaneCoverage::fully_covered)
        {
            return Ok(());
        }
        Err(full_recon_deferred_leaf_error(offset))
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
    /// `mode == DC_PRED` and `useDip == 0`; CfL uses its own base-prediction path
    /// because `UVMode == UV_CFL_PRED` disables this modifier.
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

    fn smooth_edge_availability_samples(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> Option<(usize, usize)> {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        let left_run = mi_col
            .checked_sub(1)
            .map_or(0, |left| coverage.covered_run_len(left, mi_row, 0, 1, mi_h));
        let above_run = mi_row.checked_sub(1).map_or(0, |above| {
            coverage.covered_run_len(mi_col, above, 1, 0, mi_w)
        });
        if left_run == 0 && above_run == 0 && (mi_col > 0 || mi_row > 0) {
            return None;
        }
        Some((left_run * MI_SIZE, above_run * MI_SIZE))
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

    /// AV2 §7.13.2.1 `aboveMrlIndex == sbBoundary ? 0 : MrlIndex`, where `sbBoundary
    /// == (mi_row % mib_size == 0)` (AVM `is_sb_boundary == (mi_row % cm->mib_size ==
    /// 0 && row_off == 0)`; for a per-transform leaf `mi_row` is the transform's
    /// absolute MI row, so `row_off == 0` folds in). At a superblock-row boundary the
    /// above line is forced to the immediate edge — the lines above are a different SB.
    fn above_mrl_index(&self, mi_row: usize, mrl_index: usize) -> usize {
        if self.sb_mib != 0 && mi_row.is_multiple_of(self.sb_mib) {
            0
        } else {
            mrl_index
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
    ///
    /// §5.20.5.5 MULTI-REFERENCE-LINE: a non-zero `MrlIndex` shifts the §7.13.2.8
    /// `pAngle` off the raw `Mode_To_Angle` by `Mrl_Index_To_Delta[MrlIndex]`, so a
    /// cardinal V_PRED / H_PRED leaf becomes a real angle on the offset reference line.
    /// The ZONE-1 (above) MRL projection is admitted (the edge builder threads the
    /// `aboveMrlIndex == sbBoundary ? 0 : MrlIndex` row offset and the widened
    /// `maxBase`); the §7.13.2.7 edge/corner filter and IBP blend are skipped at
    /// `MrlIndex > 0` (AVM `mrl_index == 0` gates). The ZONE-3 (left) MRL, the zone-2
    /// middle band, and the `MrlIndex`-3 exact-cardinal still DEFER: the zone-3
    /// primitive is AVM-exact in isolation (`zone3_d203_mrl_index_1...`) but a
    /// near-cardinal `pAngle == 181` interior leaf is off by 1 in its last projected
    /// row against the stream oracle — unresolved.
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
        mrl_sec_index: Option<u8>,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let mrl = usize::from(mrl_index);
        let Some(nominal) = mode.mode_to_angle() else {
            return Ok(false);
        };
        let w = 1u32 << log2_width;
        let h = 1u32 << log2_height;
        let mrl_delta = MRL_INDEX_TO_DELTA[mrl.min(3)];
        let nominal_angle = i32::from(nominal) + i32::from(angle_delta_y) * ANGLE_STEP + mrl_delta;
        let p_angle = wide_angle_mapping(w, h, nominal_angle);
        let not4x4 = !(w == 4 && h == 4);
        if 90 < p_angle && p_angle < 180 {
            if mrl != 0 {
                return self.try_reconstruct_two_sided_middle_mrl(
                    mi_col,
                    mi_row,
                    log2_width,
                    log2_height,
                    p_angle,
                    block,
                    qindex,
                    use_tcq,
                    mi_w,
                    mi_h,
                    mrl,
                    mrl_sec_index == Some(1) && not4x4,
                    LumaTransformTypeContext::with_mrl_indices(
                        mode,
                        angle_delta_y,
                        mrl_index,
                        mrl_sec_index,
                    ),
                    tile_offset,
                );
            }
            return self.try_reconstruct_two_sided_middle(
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                p_angle,
                block,
                qindex,
                use_tcq,
                mi_w,
                mi_h,
                LumaTransformTypeContext::with_mrl_indices(
                    mode,
                    angle_delta_y,
                    mrl_index,
                    mrl_sec_index,
                ),
                tile_offset,
            );
        }
        let one_sided = (0 < p_angle && p_angle < 90) || (180 < p_angle && p_angle < 270);
        if mrl != 0 && (p_angle == 90 || p_angle == 180) {
            let direction = if p_angle == 90 {
                IntraCardinalDirection::Vertical
            } else {
                IntraCardinalDirection::Horizontal
            };
            let secondary_mrl = mrl_sec_index == Some(1) && not4x4;
            if !self.full_recon
                && !self.cardinal_mrl_edge_reconstructed(
                    direction,
                    mi_col,
                    mi_row,
                    mi_w,
                    mi_h,
                    mrl,
                    secondary_mrl,
                )
            {
                return Ok(false);
            }
            let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
            let above_mrl_index = self.above_mrl_index(mi_row, mrl);
            reconstruct_general_intra_cardinal_mrl_luma_block_into(
                &mut self.workspace,
                block,
                direction,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                mrl,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_luma_cardinal_mrl_write",
                )
            })?;
            return Ok(true);
        }
        if !one_sided {
            return Ok(false);
        }
        let Ok(p_angle_u16) = u16::try_from(p_angle) else {
            return Ok(false);
        };
        let Ok(angle) = IntraDirectionalAngle::try_from_p_angle(p_angle_u16) else {
            return Ok(false);
        };
        let apply_ibp = self.enable_ibp && not4x4;
        let angle_delta_even = angle_delta_y % 2 == 0;
        let use_ibp = apply_ibp && angle_delta_even && mrl == 0; // §7.13.2.7 IBP needs MrlIndex == 0
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
                LumaTransformTypeContext::with_mrl_indices(
                    mode,
                    angle_delta_y,
                    mrl_index,
                    mrl_sec_index,
                ),
                tile_offset,
            );
        }
        let edge_filter = if mrl == 0 {
            match self.resolve_one_sided_edge_filter(
                mi_col,
                mi_row,
                w,
                h,
                p_angle,
                apply_ibp,
                tile_offset,
            )? {
                Some(filter) => filter,
                None => return Ok(false),
            }
        } else {
            OneSidedEdgeFilter::default()
        };
        let Ok(block_size) = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        ) else {
            return Ok(false);
        };
        let Ok(max_read) = angle.max_one_sided_edge_read_index(block_size, mrl) else {
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
                let above_mrl_inputs = OneSidedAboveMrl {
                    mrl_index: mrl,
                    above_mrl_index: self.above_mrl_index(mi_row, mrl),
                };
                let secondary_mrl = mrl != 0 && mrl_sec_index == Some(1) && not4x4;
                let result = if secondary_mrl {
                    reconstruct_general_intra_mrl_secondary_above_block_into(
                        &mut self.workspace,
                        block,
                        p_angle_u16,
                        x,
                        y,
                        log2_width,
                        log2_height,
                        qindex,
                        num4_above_right,
                        above_mrl_inputs,
                        use_tcq,
                        self.bit_depth,
                    )
                } else {
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
                        above_mrl_inputs,
                        use_tcq,
                        Some(LumaTransformTypeContext::with_mrl_indices(
                            mode,
                            angle_delta_y,
                            mrl_index,
                            mrl_sec_index,
                        )),
                        self.bit_depth,
                        edge_filter,
                    )
                };
                result.map_err(|_| {
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
                let have_above = self.left_leaf_has_above_row(mi_col, mi_row);
                let secondary_mrl = mrl != 0 && mrl_sec_index == Some(1) && not4x4;
                let result = if secondary_mrl {
                    reconstruct_general_intra_mrl_secondary_left_block_into(
                        &mut self.workspace,
                        block,
                        p_angle_u16,
                        x,
                        y,
                        log2_width,
                        log2_height,
                        qindex,
                        num4_below_left,
                        have_above,
                        mrl,
                        use_tcq,
                        self.bit_depth,
                    )
                } else {
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
                        have_above,
                        mrl,
                        use_tcq,
                        Some(LumaTransformTypeContext::with_mrl_indices(
                            mode,
                            angle_delta_y,
                            mrl_index,
                            mrl_sec_index,
                        )),
                        self.bit_depth,
                        edge_filter,
                    )
                };
                result.map_err(|_| {
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
        luma_context: LumaTransformTypeContext,
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
        let Ok(primary_max_read) = angle.max_one_sided_edge_read_index(block_size, 0) else {
            return Ok(false);
        };
        let Ok(secondary_max_read) = second_dir_angle.max_one_sided_edge_read_index(block_size, 0)
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
            PlaneId::Y,
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
            true,
            use_tcq,
            Some(luma_context),
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

    /// Attempts to reconstruct a §7.13.2.8 ZONE-2 (middle, `90 < p_angle < 180`)
    /// directional luma leaf over BOTH the above row and the left column. Returns
    /// `Ok(true)` when reconstructed bit-exact, `Ok(false)` when DEFERRED.
    ///
    /// A zone-2 leaf reads the in-block above row `AboveRow[0..w)`, the in-block left
    /// column `LeftCol[0..h)`, and the shared corner — NO above-right / below-left far
    /// samples (the z2 projection's `base_x in [-1, w-1]`, `base_y in [-1, h-1]` stay
    /// in-block, AVM `need_right == need_bottom == 0`). A leaf is ADMITTED only when
    /// the whole above row, left column, and corner unit are reconstructed by this
    /// sink ([`Self::two_sided_middle_neighbours_reconstructed`]) and the §7.13.2.7
    /// two-edge filter resolves (its corner-opposite samples are covered); otherwise
    /// DEFER. `MrlIndex == 0` is enforced by the caller.
    #[allow(clippy::too_many_arguments)]
    fn try_reconstruct_two_sided_middle(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        p_angle: i32,
        block: &LumaCoeffBlock,
        qindex: u32,
        use_tcq: bool,
        mi_w: usize,
        mi_h: usize,
        luma_context: LumaTransformTypeContext,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let Ok(p_angle_u16) = u16::try_from(p_angle) else {
            return Ok(false);
        };
        let w = 1u32 << log2_width;
        let h = 1u32 << log2_height;
        if !self.full_recon
            && !self.two_sided_middle_neighbours_reconstructed(mi_col, mi_row, mi_w, mi_h)
        {
            return Ok(false);
        }
        let filters = match self.resolve_two_sided_middle_edge_filters(
            mi_col,
            mi_row,
            w,
            h,
            p_angle,
            tile_offset,
        )? {
            Some(filters) => filters,
            None if self.full_recon => TwoSidedMiddleEdgeFilters {
                above: OneSidedEdgeFilter::default(),
                left: OneSidedEdgeFilter::default(),
            },
            None => return Ok(false),
        };
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        reconstruct_general_intra_middle_neighbour_rect_block_into(
            &mut self.workspace,
            block,
            p_angle_u16,
            PlaneId::Y,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            use_tcq,
            Some(luma_context),
            self.bit_depth,
            filters,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_luma_two_sided_middle_write",
            )
        })?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn try_reconstruct_two_sided_middle_mrl(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        p_angle: i32,
        block: &LumaCoeffBlock,
        qindex: u32,
        use_tcq: bool,
        mi_w: usize,
        mi_h: usize,
        mrl_index: usize,
        secondary_mrl: bool,
        luma_context: LumaTransformTypeContext,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let Ok(p_angle_u16) = u16::try_from(p_angle) else {
            return Ok(false);
        };
        if !self.full_recon
            && !self.two_sided_middle_neighbours_reconstructed(mi_col, mi_row, mi_w, mi_h)
        {
            return Ok(false);
        }
        let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
        let above_mrl_index = self.above_mrl_index(mi_row, mrl_index);
        let is_sb_boundary = self.sb_mib != 0 && mi_row.is_multiple_of(self.sb_mib);
        reconstruct_general_intra_two_sided_middle_luma_mrl_block_into(
            &mut self.workspace,
            block,
            p_angle_u16,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            mrl_index,
            above_mrl_index,
            is_sb_boundary,
            secondary_mrl,
            use_tcq,
            Some(luma_context),
            self.bit_depth,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_luma_two_sided_middle_mrl_write",
            )
        })?;
        Ok(true)
    }

    /// Whether the §7.13.2.8 zone-2 above row (`mi_row - 1` over the block's `mi_w`
    /// columns), left column (`mi_col - 1` over `mi_h` rows), AND the diagonal corner
    /// unit `(mi_col - 1, mi_row - 1)` are all reconstructed by this sink. Zone-2
    /// reads no above-right / below-left, so only the in-block edges + corner matter.
    fn two_sided_middle_neighbours_reconstructed(
        &self,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let (Some(above), Some(left)) = (mi_row.checked_sub(1), mi_col.checked_sub(1)) else {
            return false;
        };
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if !covered(left, above) {
            return false;
        }
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return false;
        }
        (mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r))
    }

    #[allow(clippy::too_many_arguments)]
    fn cardinal_mrl_edge_reconstructed(
        &self,
        direction: IntraCardinalDirection,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
        mrl_index: usize,
        secondary_mrl: bool,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        match direction {
            IntraCardinalDirection::Vertical => {
                let Some(primary_row) = mi_row
                    .checked_sub(1)
                    .and_then(|row| row.checked_sub(self.above_mrl_index(mi_row, mrl_index)))
                else {
                    return false;
                };
                if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, primary_row)) {
                    return false;
                }
                if secondary_mrl {
                    let Some(immediate_row) = mi_row.checked_sub(1) else {
                        return false;
                    };
                    (mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, immediate_row))
                } else {
                    true
                }
            }
            IntraCardinalDirection::Horizontal => {
                let Some(primary_col) = mi_col
                    .checked_sub(1)
                    .and_then(|col| col.checked_sub(mrl_index))
                else {
                    return false;
                };
                if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(primary_col, r)) {
                    return false;
                }
                if secondary_mrl {
                    let Some(immediate_col) = mi_col.checked_sub(1) else {
                        return false;
                    };
                    (mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(immediate_col, r))
                } else {
                    true
                }
            }
        }
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
        if let Some(num4) = self.full_recon_far_edge(mi_col, mi_row, FarEdgeSide::AboveRight) {
            return Some(num4);
        }
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
        Self::one_sided_num4_or_defer(num4_above_right, mi_w, width, max_read, edge_filter_active)
    }

    /// Shared §7.13.2.1 tail for the symmetric one-sided coverage guards: given the
    /// counted far-edge `num4` (above-right or below-left) plus the leaf's `mi_span`
    /// (`mi_w` / `mi_h`) and sample `dim` (`width` / `height`), DEFERS (`None`) when an
    /// ACTIVE edge filter lacks the full `mi_span` span, or when a no-op-filter leaf's
    /// projection `max_read` reaches past the covered extent; otherwise returns the
    /// `num4`. Both axes share this exactly, so it lives once.
    fn one_sided_num4_or_defer(
        num4: usize,
        mi_span: usize,
        dim: usize,
        max_read: usize,
        edge_filter_active: bool,
    ) -> Option<usize> {
        if edge_filter_active {
            if num4 < mi_span {
                return None;
            }
        } else {
            let covered_extent = dim.checked_add(num4.checked_mul(MI_SIZE)?)?;
            if max_read >= dim && max_read >= covered_extent {
                return None;
            }
        }
        Some(num4)
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
    /// Whether a zone-3 (left-reading) leaf's above row is AVAILABLE (§7.13.2.1
    /// `haveAbove` / AVM `n_top_px > 0`): the above MI unit `(mi_col, mi_row - 1)`
    /// is in-grid. This drives the §7.13.2.1 corner `LeftCol[-1]` selection — the
    /// DIAGONAL above-left `CurrFrame[y - 1][x - 1]` when available, else the top of
    /// the left column `CurrFrame[y][x - 1]`. A frame-top leaf (`mi_row == 0`) has
    /// no above row, so the corner is the left-column top.
    fn left_leaf_has_above_row(&self, mi_col: usize, mi_row: usize) -> bool {
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        mi_row
            .checked_sub(1)
            .is_some_and(|above| !coverage.off_grid(mi_col, above))
    }

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
        if let Some(num4) = self.full_recon_far_edge(mi_col, mi_row, FarEdgeSide::BelowLeft) {
            return Some(num4);
        }
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
        Self::one_sided_num4_or_defer(num4_below_left, mi_h, height, max_read, edge_filter_active)
    }

    /// Records the AV2 §7.13.2.1 PER-TRANSFORM far-edge availability
    /// (`num4AboveRight`, `num4BelowLeft`, in luma 4x4 units) for the luma transform
    /// at `(mi_col, mi_row)` of `tx_size`, as derived by the caller from the live
    /// §5.20.2.3 `BlockDecoded` state and the transform's `(row_off, col_off)` within
    /// its coding block — the AVM `has_top_right` / `has_bottom_left` counts at the
    /// transform's own `tx_size_wide_unit` / `tx_size_high_unit`. This is the durable
    /// AVM-availability infrastructure: it populates [`Self::far_edge_avail`] so any
    /// predictor path can query the real per-transform far-edge availability (via
    /// [`Self::block_decoded_far_edge`]) rather than re-deriving it from the
    /// conservative coverage map. Behavior-neutral: the coverage-gated admission path
    /// does not consume the recorded data yet.
    pub(in crate::runtime_minimal) fn record_block_decoded_far_edge(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        tx_size: usize,
        num4_above_right: usize,
        num4_below_left: usize,
    ) {
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return;
        };
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        let above_right = u32::try_from(num4_above_right).unwrap_or(u32::MAX);
        let below_left = u32::try_from(num4_below_left).unwrap_or(u32::MAX);
        self.far_edge_avail
            .record(mi_col, mi_row, mi_w, mi_h, above_right, below_left);
    }

    /// The AV2 §7.13.2.1 far-edge availability (`num4AboveRight`, `num4BelowLeft`)
    /// recorded for the luma MI unit at `(mi_col, mi_row)` from the live §5.20.2.3
    /// `BlockDecoded` state, or `None` when no block has been decoded into that unit.
    /// Lets a verification test confirm the threaded AVM far-edge counts match the
    /// spec `count_top_right_avail` / `count_bottom_left_avail` reads.
    #[cfg(test)]
    pub(in crate::runtime_minimal) fn block_decoded_far_edge(
        &self,
        mi_col: usize,
        mi_row: usize,
    ) -> Option<(usize, usize)> {
        self.far_edge_avail
            .get(mi_col, mi_row)
            .map(|(ar, bl)| (ar as usize, bl as usize))
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
    /// PAETH (`PAETH_PRED`, `mrl_index == 0`) is dispatched to the §7.13.2.2
    /// predictor + §7.14.3 residual add when its `haveAbove && haveLeft` neighbours
    /// (above row, left column, diagonal corner) are covered; otherwise it DEFERS.
    /// The remaining unsupported modes (the §7.13.2.8 angular modes whose one-sided
    /// / middle path the directional arm cannot yet prove, SMOOTH) are DEFERRED.
    ///
    /// `use_tcq` carries the §7.14.4 luma TCQ `dqDenom` term; `qindex` is the
    /// per-block dequant index (the §5.20.6.5 `DeltaQState.current_q_index`);
    /// `fsc_mode` is the leaf's FSC flag; `mrl_index` is the leaf's §5.20.5.5
    /// `MrlIndex` (the multi-reference-line distance, `0` for the immediate edge).
    /// `mi_col` / `mi_row` are the transform's §3 MI coordinates and `tx_size` its
    /// §5.20.6 `TxSize` index.
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
        mrl_sec_index: Option<u8>,
        angle_delta_y: i8,
        qindex: u32,
        use_tcq: bool,
        fsc_mode: bool,
        is_intrabc: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.quant_reconstructable {
            if self.full_recon {
                return Err(full_recon_deferred_leaf_error(tile_offset));
            }
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            if self.full_recon {
                return Err(full_recon_deferred_leaf_error(tile_offset));
            }
            return Ok(());
        };
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if let Some(mode) = leaf_y_mode {
            self.coverage[Self::coverage_index(PlaneId::Y)]
                .record_y_mode(mi_col, mi_row, mi_w, mi_h, mode);
        }
        let allow_full_recon_cardinal_ist = mrl_index == 0
            && matches!(
                directional,
                Some(
                    SupportedDirectionalLumaMode::Horizontal
                        | SupportedDirectionalLumaMode::Vertical
                )
            );
        let allow_full_recon_luma_ist = self.full_recon
            && !fsc_mode
            && block_has_real_ist(block)
            && leaf_y_mode.is_some_and(|mode| {
                mode == IntraYMode::DC_PRED
                    || mode.supported_nondc().is_some()
                    || allow_full_recon_cardinal_ist
                    || full_recon_mode_uses_supported_directional_edge(
                        mode,
                        angle_delta_y,
                        mrl_index,
                        mrl_sec_index,
                        log2_width,
                        log2_height,
                    )
            });
        if !residual_is_reconstructable(block, fsc_mode) && !allow_full_recon_luma_ist {
            if crate::trace_flags::trace_flag!("SPLOT_TRACE_FULL_RECON_DEFER") {
                eprintln!(
                    "full_recon_residual_defer mi=({}, {}) tx_size={} log2={}x{} mode={} directional={:?} angle_delta_y={} mrl_index={} mrl_sec_index={:?} all_zero={} fsc_mode={} intra_ist={:?} offset={}",
                    mi_col,
                    mi_row,
                    tx_size,
                    log2_width,
                    log2_height,
                    full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
                    directional,
                    angle_delta_y,
                    mrl_index,
                    mrl_sec_index,
                    block.all_zero,
                    fsc_mode,
                    block.intra_ist,
                    tile_offset.get()
                );
            }
            return self.defer_full_recon_leaf(
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
                tile_offset,
            );
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
                if !self.full_recon
                    && !self.dc_edges_reconstructed(PlaneId::Y, mi_col, mi_row, mi_w, mi_h)
                {
                    return self.defer_full_recon_leaf(
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        "DC_PRED",
                        tile_offset,
                    );
                }
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                let ibp_dc = self.ibp_dc_applies(log2_width, log2_height);
                let result = if block_has_real_ist(block) {
                    reconstruct_general_intra_luma_dc_rect_block_with_ist_into(
                        &mut self.workspace,
                        block,
                        x,
                        y,
                        log2_width,
                        log2_height,
                        qindex,
                        use_tcq,
                        ibp_dc,
                        self.bit_depth,
                        LumaTransformTypeContext::with_mrl_indices(
                            IntraYMode::DC_PRED,
                            angle_delta_y,
                            mrl_index,
                            mrl_sec_index,
                        ),
                    )
                } else {
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
                };
                if !self.finish_luma_predict(
                    &result,
                    mi_col,
                    mi_row,
                    log2_width,
                    log2_height,
                    "DC_PRED",
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_luma_write",
                )? {
                    return Ok(());
                }
            }
            (_, Some(direction)) => {
                let cardinal_label = match direction {
                    IntraCardinalDirection::Vertical => "V_PRED",
                    IntraCardinalDirection::Horizontal => "H_PRED",
                };
                if mrl_index != 0 {
                    let routed = match leaf_y_mode {
                        Some(mode) => self.try_reconstruct_one_sided_angular(
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
                            mrl_sec_index,
                            tile_offset,
                        ),
                        None => Ok(false),
                    };
                    if !self.routed_angular_wrote(routed, tile_offset)? {
                        return self.defer_full_recon_leaf(
                            mi_col,
                            mi_row,
                            log2_width,
                            log2_height,
                            cardinal_label,
                            tile_offset,
                        );
                    }
                } else {
                    if !self.full_recon
                        && !self.cardinal_edge_reconstructed(
                            direction,
                            PlaneId::Y,
                            mi_col,
                            mi_row,
                            mi_w,
                            mi_h,
                        )
                    {
                        return self.defer_full_recon_leaf(
                            mi_col,
                            mi_row,
                            log2_width,
                            log2_height,
                            cardinal_label,
                            tile_offset,
                        );
                    }
                    let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                    let result = reconstruct_general_intra_cardinal_neighbour_block_into(
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
                        leaf_y_mode.map(|mode| {
                            LumaTransformTypeContext::with_mrl_indices(
                                mode,
                                angle_delta_y,
                                mrl_index,
                                mrl_sec_index,
                            )
                        }),
                        self.bit_depth,
                    );
                    if !self.finish_luma_predict(
                        &result,
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        cardinal_label,
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_luma_cardinal_write",
                    )? {
                        return Ok(());
                    }
                }
            }
            (Some(mode), None) if mode.is_paeth() && mrl_index == 0 => {
                if !self.full_recon
                    && !self.paeth_neighbours_reconstructed(PlaneId::Y, mi_col, mi_row, mi_w, mi_h)
                {
                    return self.defer_full_recon_leaf(
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        "PAETH_PRED",
                        tile_offset,
                    );
                }
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                let result = reconstruct_general_intra_luma_paeth_neighbour_block_into(
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
                );
                if !self.finish_luma_predict(
                    &result,
                    mi_col,
                    mi_row,
                    log2_width,
                    log2_height,
                    "PAETH_PRED",
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_luma_paeth_write",
                )? {
                    return Ok(());
                }
            }
            (Some(mode), None) if self.full_recon && mode.supported_nondc().is_some() => {
                let Some(smooth_mode) = mode.supported_nondc() else {
                    return self.defer_full_recon_leaf(
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
                        tile_offset,
                    );
                };
                let (x, y) = luma_sample_origin(mi_col, mi_row, tile_offset)?;
                let num4_above_right = self
                    .full_recon_far_edge(mi_col, mi_row, FarEdgeSide::AboveRight)
                    .unwrap_or(0);
                let num4_below_left = self
                    .full_recon_far_edge(mi_col, mi_row, FarEdgeSide::BelowLeft)
                    .unwrap_or(0);
                let result = reconstruct_general_intra_luma_smooth_rect_block_into(
                    &mut self.workspace,
                    block,
                    smooth_mode,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    use_tcq,
                    num4_above_right,
                    num4_below_left,
                    Some(LumaTransformTypeContext::with_mrl_indices(
                        mode,
                        angle_delta_y,
                        mrl_index,
                        mrl_sec_index,
                    )),
                    self.bit_depth,
                );
                if !self.finish_luma_predict(
                    &result,
                    mi_col,
                    mi_row,
                    log2_width,
                    log2_height,
                    full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_luma_smooth_write",
                )? {
                    return Ok(());
                }
            }
            (Some(mode), None) if mode.is_directional() => {
                let result = self.try_reconstruct_one_sided_angular(
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
                    mrl_sec_index,
                    tile_offset,
                );
                if !self.routed_angular_wrote(result, tile_offset)? {
                    let label = full_recon_mode_label(leaf_y_mode, directional, is_intrabc);
                    return self.defer_full_recon_leaf(
                        mi_col,
                        mi_row,
                        log2_width,
                        log2_height,
                        label,
                        tile_offset,
                    );
                }
            }
            _ => {
                return self.defer_full_recon_leaf(
                    mi_col,
                    mi_row,
                    log2_width,
                    log2_height,
                    full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
                    tile_offset,
                );
            }
        }
        let marked =
            self.coverage[Self::coverage_index(PlaneId::Y)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_luma_4x4 = self.reconstructed_luma_4x4.saturating_add(marked);
        self.record_full_recon_leaf(
            mi_col,
            mi_row,
            log2_width,
            log2_height,
            full_recon_mode_label(leaf_y_mode, directional, is_intrabc),
            true,
        );
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
            return self.defer_full_recon_leaf(
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                "INTRABC",
                tile_offset,
            );
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
        self.record_full_recon_leaf(mi_col, mi_row, log2_width, log2_height, "INTRABC", true);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn reconstruct_cfl_chroma_transform(
        &mut self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
        block: &LumaCoeffBlock,
        cfl_params: CflParams,
        num4_above_right: usize,
        num4_below_left: usize,
        qindex: u32,
    ) -> core::result::Result<bool, GeneralIntraResidualError> {
        let width = 1usize << log2_width;
        let height = 1usize << log2_height;
        if cfl_params.index == CflIndex::Multi {
            let Some(prediction) = self.mhccp_prediction(
                plane_id,
                x,
                y,
                width,
                height,
                cfl_params,
                num4_above_right,
                num4_below_left,
            )?
            else {
                return Ok(false);
            };
            let out = if block.all_zero {
                prediction
            } else {
                reconstruct_general_intra_block_rect_with_prediction(
                    &block.quant,
                    &prediction,
                    qindex,
                    plane_id,
                    log2_width,
                    log2_height,
                    block.plane_tx_type,
                    false,
                    self.bit_depth,
                )?
            };
            let block_size = IntraRectBlockSize::new(
                u8::try_from(log2_width).unwrap_or(u8::MAX),
                u8::try_from(log2_height).unwrap_or(u8::MAX),
            )?;
            self.workspace
                .write_rect_block(plane_id, x, y, block_size, &out)?;
            return Ok(true);
        }
        if self.cfl_filter_index().is_none() {
            return Ok(false);
        }
        let block_size = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        )?;
        let edges = self
            .workspace
            .intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
        let dc = if width > 32 || height > 32 {
            predict_intra_dc_subsampled_rect_value(self.bit_depth, block_size, edges.as_dc_edges())?
        } else {
            predict_intra_dc_rect_value(self.bit_depth, block_size, edges.as_dc_edges())?
        };
        let mut prediction = vec![dc; width.saturating_mul(height)];
        self.apply_cfl_prediction(plane_id, x, y, width, height, cfl_params, &mut prediction)?;
        let out = if block.all_zero {
            prediction
        } else {
            reconstruct_general_intra_block_rect_with_prediction(
                &block.quant,
                &prediction,
                qindex,
                plane_id,
                log2_width,
                log2_height,
                block.plane_tx_type,
                false,
                self.bit_depth,
            )?
        };
        self.workspace
            .write_rect_block(plane_id, x, y, block_size, &out)?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn mhccp_prediction(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        cfl_params: CflParams,
        num4_above_right: usize,
        num4_below_left: usize,
    ) -> core::result::Result<Option<Vec<T>>, GeneralIntraResidualError> {
        let Some(mh_dir) = cfl_params.mh_dir else {
            return Ok(None);
        };
        if mh_dir > 2 || self.cfl_filter_index().is_none() {
            return Ok(None);
        }
        let Some(refs) = self.mhccp_references(
            plane_id,
            x,
            y,
            width,
            height,
            num4_above_right,
            num4_below_left,
        )?
        else {
            return Ok(None);
        };
        let params = derive_mhccp_params(&refs, mh_dir, self.bit_depth);
        let max = i64::from(self.bit_depth.max_sample());
        let mid = 1i64 << (u32::from(self.bit_depth.bits()) - 1);
        let mut prediction = Vec::with_capacity(width.saturating_mul(height));
        for row in 0..height {
            for col in 0..width {
                let center_index = (refs.above + row) * refs.width + refs.left + col;
                let center = refs.luma[center_index];
                let linear = match mh_dir {
                    0 => center,
                    1 => {
                        let top_row = refs.above.saturating_add(row).saturating_sub(1);
                        refs.luma[top_row * refs.width + refs.left + col]
                    }
                    _ => {
                        let left_col = refs.left.saturating_add(col).saturating_sub(1);
                        refs.luma[(refs.above + row) * refs.width + left_col]
                    }
                };
                let vector = [
                    linear,
                    round2(
                        center.saturating_mul(center),
                        u32::from(self.bit_depth.bits()),
                    ),
                    mid,
                ];
                let mut predicted = 0i64;
                for k in 0..MHCCP_PARAM_COUNT {
                    predicted = predicted
                        .saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS));
                }
                let sample = clip3(0, max, predicted) as u16;
                prediction.push(T::try_from_u16(sample)?);
            }
        }
        Ok(Some(prediction))
    }

    #[allow(clippy::too_many_arguments)]
    fn mhccp_references(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        num4_above_right: usize,
        num4_below_left: usize,
    ) -> core::result::Result<Option<MhccpRefs>, GeneralIntraResidualError> {
        let have_above = y > 0;
        let have_left = x > 0;
        let above = if have_above { y.min(2) } else { 0 };
        let left = if have_left { x.min(2) } else { 0 };
        let luma_mi_row = y / 2;
        let sb_height_luma = self.sb_mib.saturating_mul(MI_SIZE);
        let sb_start_luma_y = luma_mi_row
            .checked_div(self.sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma));
        let sb_chroma_y = sb_start_luma_y / 2;
        let min_chroma_ref_y = sb_chroma_y.saturating_sub(1);
        let min_luma_ref_y = isize::try_from(sb_start_luma_y)
            .ok()
            .and_then(|sb_y| sb_y.checked_sub(1));
        let extra_right = if have_above && width > 4 {
            num4_above_right.saturating_mul(MI_SIZE).min(width)
        } else {
            0
        };
        let extra_bottom = if have_left && height > 4 {
            num4_below_left.saturating_mul(MI_SIZE).min(height)
        } else {
            0
        };
        let frame_right = self.chroma_width_for_sample_reads().saturating_sub(x);
        let frame_bottom = self.chroma_height_for_sample_reads().saturating_sub(y);
        let ref_width = left
            .saturating_add(width)
            .saturating_add(extra_right)
            .min(64)
            .min(left.saturating_add(frame_right));
        let ref_height = above
            .saturating_add(height)
            .saturating_add(extra_bottom)
            .min(64)
            .min(above.saturating_add(frame_bottom));
        if ref_width < left.saturating_add(width) || ref_height < above.saturating_add(height) {
            return Ok(None);
        }

        let mut luma = vec![0i64; ref_width.saturating_mul(ref_height)];
        let mut chroma = vec![0i64; ref_width.saturating_mul(ref_height)];
        for row in 0..ref_height {
            for col in 0..ref_width {
                let chroma_x = x + col - left;
                let chroma_y = y + row - above;
                let is_ref = row < above || col < left;
                if is_ref {
                    let ref_chroma_y = chroma_y.max(min_chroma_ref_y);
                    if !self.chroma_sample_reconstructed(plane_id, chroma_x, ref_chroma_y) {
                        return Ok(None);
                    }
                    chroma[row * ref_width + col] = i64::from(
                        self.clamped_chroma_sample(plane_id, chroma_x, ref_chroma_y)?
                            .to_u16(),
                    );
                }
                if mhccp_luma_ref_available(row, col, above, left, width, height) {
                    let clamp_x = col == 0;
                    let clamp_y = row == 0;
                    if !self.mhccp_luma_q3_sample_reconstructed(
                        chroma_x,
                        chroma_y,
                        clamp_x,
                        clamp_y,
                        min_luma_ref_y,
                    ) {
                        return Ok(None);
                    }
                    luma[row * ref_width + col] = self.cfl_luma_q3_with_min_y(
                        chroma_x,
                        chroma_y,
                        clamp_x,
                        clamp_y,
                        min_luma_ref_y,
                    )? >> 3;
                }
            }
        }
        Ok(Some(MhccpRefs {
            width: ref_width,
            height: ref_height,
            above,
            left,
            luma,
            chroma,
        }))
    }

    fn chroma_sample_reconstructed(&self, plane_id: PlaneId, x: usize, y: usize) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        coverage.is_covered(x / MI_SIZE, y / MI_SIZE)
    }

    fn mhccp_luma_q3_sample_reconstructed(
        &self,
        chroma_x: usize,
        chroma_y: usize,
        clamp_x: bool,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> bool {
        let Some(filter_index) = self.cfl_filter_index() else {
            return false;
        };
        let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
        let max_x = self.luma_width.saturating_sub(1) as isize;
        let max_y = self.luma_height.saturating_sub(1) as isize;
        let luma_x = chroma_x.saturating_mul(2) as isize;
        let luma_y = chroma_y.saturating_mul(2) as isize;
        for (dy_index, dy) in [-1isize, 0, 1].into_iter().enumerate() {
            for (dx_index, dx) in [-1isize, 0, 1].into_iter().enumerate() {
                if CFL_FILTERS_420[filter_index][dy_index][dx_index] == 0 {
                    continue;
                }
                let sx = luma_x + if clamp_x { dx.max(0) } else { dx };
                let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
                if let Some(min_y) = min_luma_ref_y {
                    sy = sy.max(min_y);
                }
                let sample_x = sx.clamp(0, max_x) as usize;
                let sample_y = sy.clamp(0, max_y) as usize;
                if !coverage.is_covered(sample_x / MI_SIZE, sample_y / MI_SIZE) {
                    return false;
                }
            }
        }
        true
    }

    fn cfl_luma_q3_with_min_y(
        &self,
        chroma_x: usize,
        chroma_y: usize,
        clamp_x: bool,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> core::result::Result<i64, GeneralIntraResidualError> {
        let Some(filter_index) = self.cfl_filter_index() else {
            return Ok(0);
        };
        let luma_x = (chroma_x.saturating_mul(2)) as isize;
        let luma_y = (chroma_y.saturating_mul(2)) as isize;
        let mut total = 0i64;
        for (dy_index, dy) in [-1isize, 0, 1].into_iter().enumerate() {
            for (dx_index, dx) in [-1isize, 0, 1].into_iter().enumerate() {
                let weight = CFL_FILTERS_420[filter_index][dy_index][dx_index];
                if weight == 0 {
                    continue;
                }
                let sx = luma_x + if clamp_x { dx.max(0) } else { dx };
                let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
                if let Some(min_y) = min_luma_ref_y {
                    sy = sy.max(min_y);
                }
                total += weight * i64::from(self.clamped_luma_sample(sx, sy)?.to_u16());
            }
        }
        Ok(total)
    }

    fn cfl_above_min_luma_ref_y(&self, chroma_y: usize) -> Option<isize> {
        if self.sb_mib == 0 {
            return None;
        }
        let luma_mi_row = chroma_y / 2;
        let sb_height_luma = self.sb_mib.saturating_mul(MI_SIZE);
        let sb_start_luma_y = (luma_mi_row / self.sb_mib).saturating_mul(sb_height_luma);
        isize::try_from(sb_start_luma_y)
            .ok()
            .and_then(|sb_y| sb_y.checked_sub(1))
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_cfl_prediction(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        cfl_params: CflParams,
        prediction: &mut [T],
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let alpha_q3 = self.cfl_alpha_q3(plane_id, x, y, width, height, cfl_params)?;
        let luma_avg = self.cfl_luma_average_q3(x, y, width, height)?;
        let max = i64::from(self.bit_depth.max_sample());
        for row in 0..height {
            let chroma_y = y.saturating_add(row);
            let luma_y = chroma_y.saturating_mul(2);
            let clamp_y = row == 0 || luma_y % 64 == 0;
            for col in 0..width {
                let chroma_x = x.saturating_add(col);
                let luma_x = chroma_x.saturating_mul(2);
                let clamp_x = col == 0 || luma_x % 64 == 0;
                let luma = self.cfl_luma_q3(chroma_x, chroma_y, clamp_x, clamp_y)?;
                let scaled_luma = round2_signed(alpha_q3 * (luma - luma_avg), CFL_ALPHA_SHIFT);
                let index = row * width + col;
                let dc = i64::from(prediction[index].to_u16());
                let clipped = clip3(0, max, dc + scaled_luma) as u16;
                prediction[index] = T::try_from_u16(clipped)?;
            }
        }
        Ok(())
    }

    fn cfl_alpha_q3(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        cfl_params: CflParams,
    ) -> core::result::Result<i64, GeneralIntraResidualError> {
        let alpha_q3 = match cfl_params.index {
            CflIndex::Explicit => {
                let alpha = match plane_id {
                    PlaneId::U => cfl_params.alpha_u,
                    PlaneId::V => cfl_params.alpha_v,
                    PlaneId::Y => 0,
                };
                i64::from(alpha) * CFL_ALPHA_SCALE
            }
            CflIndex::DerivedAlpha => self.derive_cfl_alpha_q3(plane_id, x, y, width, height)?,
            CflIndex::Multi => 0,
        };
        Ok(alpha_q3)
    }

    fn derive_cfl_alpha_q3(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> core::result::Result<i64, GeneralIntraResidualError> {
        let have_above = y > 0;
        let have_left = x > 0;
        let (mut num_above, mut num_left) = if have_above && have_left {
            if width > height.saturating_mul(2) {
                (NUM_REF_SAM_CFL, 0)
            } else if height > width.saturating_mul(2) {
                (0, NUM_REF_SAM_CFL)
            } else {
                (NUM_REF_SAM_CFL >> 1, NUM_REF_SAM_CFL >> 1)
            }
        } else {
            (
                if have_above { NUM_REF_SAM_CFL } else { 0 },
                if have_left { NUM_REF_SAM_CFL } else { 0 },
            )
        };
        num_above = num_above.min(width);
        num_left = num_left.min(height);

        let mut count = 0i64;
        let mut sum_x = 0i64;
        let mut sum_y = 0i64;
        let mut sum_xy = 0i64;
        let mut sum_xx = 0i64;
        if num_above > 0 {
            let min_luma_ref_y = self.cfl_above_min_luma_ref_y(y);
            let step = width.checked_div(num_above).unwrap_or(0).max(1);
            let start = if step == 1 { 0 } else { step >> 1 };
            for col in (start..width).step_by(step) {
                let chroma_x = x.saturating_add(col);
                let luma_x = chroma_x.saturating_mul(2);
                let clamp_x = col == 0 || luma_x % 64 == 0;
                let luma =
                    self.cfl_luma_q3_with_min_y(chroma_x, y - 1, clamp_x, false, min_luma_ref_y)?
                        >> 3;
                let chroma = i64::from(
                    self.clamped_chroma_sample(plane_id, chroma_x, y - 1)?
                        .to_u16(),
                );
                sum_x += luma;
                sum_y += chroma;
                sum_xy += luma * chroma;
                sum_xx += luma * luma;
                count += 1;
            }
        }
        if num_left > 0 {
            let step = height.checked_div(num_left).unwrap_or(0).max(1);
            let start = if step == 1 { 0 } else { step >> 1 };
            for row in (start..height).step_by(step) {
                let chroma_y = y.saturating_add(row);
                let luma_y = chroma_y.saturating_mul(2);
                let clamp_y = row == 0 || luma_y % 64 == 0;
                let luma = self.cfl_luma_q3(x - 1, chroma_y, false, clamp_y)? >> 3;
                let chroma = i64::from(
                    self.clamped_chroma_sample(plane_id, x - 1, chroma_y)?
                        .to_u16(),
                );
                sum_x += luma;
                sum_y += chroma;
                sum_xy += luma * chroma;
                sum_xx += luma * luma;
                count += 1;
            }
        }
        if count == 0 {
            return Ok(0);
        }
        let der = sum_xx - (sum_x * sum_x) / count;
        let nor = sum_xy - (sum_x * sum_y) / count;
        if der == 0 || nor == 0 {
            return Ok(0);
        }
        Ok(i64::from(resolve_division(
            nor,
            der,
            CFL_DERIVED_ALPHA_SHIFT,
        )))
    }

    fn cfl_luma_region_reconstructed(
        &self,
        chroma_x: usize,
        chroma_y: usize,
        log2_width: u32,
        log2_height: u32,
    ) -> bool {
        let luma_mi_col = chroma_x / 2;
        let luma_mi_row = chroma_y / 2;
        let luma_mi_w = (1usize << log2_width).div_ceil(2);
        let luma_mi_h = (1usize << log2_height).div_ceil(2);
        self.coverage[Self::coverage_index(PlaneId::Y)].region_fully_covered(
            luma_mi_col,
            luma_mi_row,
            luma_mi_w,
            luma_mi_h,
        )
    }

    fn cfl_luma_average_q3(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> core::result::Result<i64, GeneralIntraResidualError> {
        let step_w = if width > 32 { 2 } else { 1 };
        let step_h = if height > 32 { 2 } else { 1 };
        let mut sum = 0u64;
        let mut count = 0u64;
        if let Some(above_y) = y.checked_sub(1) {
            let min_luma_ref_y = self.cfl_above_min_luma_ref_y(y);
            for col in (0..width).step_by(step_w) {
                let chroma_x = x.saturating_add(col);
                let luma_x = chroma_x.saturating_mul(2);
                let clamp_x = col == 0 || luma_x % 64 == 0;
                sum = sum.saturating_add(self.cfl_luma_q3_with_min_y(
                    chroma_x,
                    above_y,
                    clamp_x,
                    false,
                    min_luma_ref_y,
                )? as u64);
                count = count.saturating_add(1);
            }
        }
        if let Some(left_x) = x.checked_sub(1) {
            for row in (0..height).step_by(step_h) {
                let chroma_y = y.saturating_add(row);
                let luma_y = chroma_y.saturating_mul(2);
                let clamp_y = row == 0 || luma_y % 64 == 0;
                sum =
                    sum.saturating_add(self.cfl_luma_q3(left_x, chroma_y, false, clamp_y)? as u64);
                count = count.saturating_add(1);
            }
        }
        if count == 0 {
            return Ok(i64::from(8u16 << (self.bit_depth.bits() - 1)));
        }
        let max = (8u16 << self.bit_depth.bits()).saturating_sub(1);
        Ok(i64::from(approx_divide(sum, count)?.min(max)))
    }

    fn cfl_luma_q3(
        &self,
        chroma_x: usize,
        chroma_y: usize,
        clamp_x: bool,
        clamp_y: bool,
    ) -> core::result::Result<i64, GeneralIntraResidualError> {
        let Some(filter_index) = self.cfl_filter_index() else {
            return Ok(0);
        };
        let luma_x = (chroma_x.saturating_mul(2)) as isize;
        let luma_y = (chroma_y.saturating_mul(2)) as isize;
        let mut total = 0i64;
        for (dy_index, dy) in [-1isize, 0, 1].into_iter().enumerate() {
            for (dx_index, dx) in [-1isize, 0, 1].into_iter().enumerate() {
                let weight = CFL_FILTERS_420[filter_index][dy_index][dx_index];
                if weight == 0 {
                    continue;
                }
                let sx = luma_x + if clamp_x { dx.max(0) } else { dx };
                let sy = luma_y + if clamp_y { dy.max(0) } else { dy };
                total += weight * i64::from(self.clamped_luma_sample(sx, sy)?.to_u16());
            }
        }
        Ok(total)
    }

    fn clamped_luma_sample(&self, x: isize, y: isize) -> splot_recon::Result<T> {
        let max_x = self.luma_width.saturating_sub(1) as isize;
        let max_y = self.luma_height.saturating_sub(1) as isize;
        let sx = x.clamp(0, max_x) as usize;
        let sy = y.clamp(0, max_y) as usize;
        self.direct_plane_sample(PlaneId::Y, sx, sy)
    }

    fn clamped_chroma_sample(
        &self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
    ) -> splot_recon::Result<T> {
        let sx = x.min(self.chroma_width_for_sample_reads().saturating_sub(1));
        let sy = y.min(self.chroma_height_for_sample_reads().saturating_sub(1));
        self.direct_plane_sample(plane_id, sx, sy)
    }

    /// Clamped-coordinate plane read on the CfL/MHCCP reference paths. The
    /// callers clamp `(sx, sy)` to plane dimensions, which never exceed the
    /// workspace storage, so the flat read always lands in-row; the checked
    /// accessor remains as the fallback that reports the identical error for
    /// an out-of-storage coordinate.
    fn direct_plane_sample(
        &self,
        plane_id: PlaneId,
        sx: usize,
        sy: usize,
    ) -> splot_recon::Result<T> {
        let plane = self.workspace.plane(plane_id)?;
        match plane.samples().get(sy * plane.stride_samples() + sx) {
            Some(&sample) => Ok(sample),
            None => self.workspace.reconstructed_sample(plane_id, sx, sy),
        }
    }

    const fn chroma_width_for_sample_reads(&self) -> usize {
        self.luma_width.div_ceil(2)
    }

    const fn chroma_height_for_sample_reads(&self) -> usize {
        self.luma_height.div_ceil(2)
    }

    const fn cfl_filter_index(&self) -> Option<usize> {
        match self.cfl_ds_filter_index {
            0 | 3 => Some(0),
            1 => Some(1),
            2 => Some(2),
            _ => None,
        }
    }

    /// Reconstructs one chroma (U or V) transform block at the given chroma-plane
    /// sample position into the workspace. The block is DEFERRED unless ALL of the
    /// proven subset holds: the frame dequant matches the zero-`QuantizerDeltas`
    /// assumption (`quant_reconstructable`); the residual is reconstructable; and
    /// the §7.13.2 DC-prediction/CfL reference inputs are off-frame or already
    /// reconstructed by this sink. The `(x, y)` sample position must be MI-aligned
    /// (chroma transforms are).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn reconstruct_chroma_transform(
        &mut self,
        plane_id: PlaneId,
        chroma_tx: usize,
        x: usize,
        y: usize,
        block: &LumaCoeffBlock,
        chroma_mode: Option<SupportedChromaMode>,
        angle_delta_y: i8,
        cfl_params: Option<CflParams>,
        num4_above_right: usize,
        num4_below_left: usize,
        qindex: u32,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.quant_reconstructable {
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
        if let Some(cfl_params) = cfl_params {
            let edges_ok = self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h);
            let width = 1usize << log2_width;
            let height = 1usize << log2_height;
            let alpha_q3 = self
                .cfl_alpha_q3(plane_id, x, y, width, height, cfl_params)
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_write",
                    )
                })?;
            let luma_ok =
                alpha_q3 == 0 || self.cfl_luma_region_reconstructed(x, y, log2_width, log2_height);
            if !edges_ok || !luma_ok {
                self.defer_chroma_transform(
                    plane_id,
                    chroma_tx,
                    x,
                    y,
                    block,
                    chroma_mode,
                    angle_delta_y,
                    Some(cfl_params),
                    num4_above_right,
                    num4_below_left,
                    qindex,
                    tile_offset,
                );
                return Ok(());
            }
            let wrote = self
                .reconstruct_cfl_chroma_transform(
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    block,
                    cfl_params,
                    num4_above_right,
                    num4_below_left,
                    qindex,
                )
                .map_err(|_| {
                    wienerns_lr_selectable_transform_record_error_reason(
                        tile_offset,
                        "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_write",
                    )
                })?;
            if !wrote {
                self.defer_chroma_transform(
                    plane_id,
                    chroma_tx,
                    x,
                    y,
                    block,
                    chroma_mode,
                    angle_delta_y,
                    Some(cfl_params),
                    num4_above_right,
                    num4_below_left,
                    qindex,
                    tile_offset,
                );
                return Ok(());
            }
            let marked =
                self.coverage[Self::coverage_index(plane_id)].mark(mi_col, mi_row, mi_w, mi_h);
            self.reconstructed_chroma_4x4 = self.reconstructed_chroma_4x4.saturating_add(marked);
            self.record_chroma_deblock_block(plane_id, x, y, chroma_tx, qindex);
            return Ok(());
        }
        let Some(chroma_mode) = chroma_mode else {
            return Ok(());
        };
        self.coverage[Self::coverage_index(plane_id)].record_chroma_mode(
            mi_col,
            mi_row,
            mi_w,
            mi_h,
            chroma_mode,
        );
        if self.full_recon && Self::chroma_direction_base_angle(chroma_mode).is_some() {
            if self.try_reconstruct_chroma_follow_angle(
                plane_id,
                chroma_mode,
                angle_delta_y,
                chroma_tx,
                x,
                y,
                log2_width,
                log2_height,
                block,
                num4_above_right,
                num4_below_left,
                qindex,
                tile_offset,
            )? {
                return Ok(());
            }
            self.defer_chroma_transform(
                plane_id,
                chroma_tx,
                x,
                y,
                block,
                Some(chroma_mode),
                angle_delta_y,
                cfl_params,
                num4_above_right,
                num4_below_left,
                qindex,
                tile_offset,
            );
            return Ok(());
        }
        match chroma_mode {
            SupportedChromaMode::Dc => {
                let edges_ok = self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h);
                if !edges_ok {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
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
            }
            SupportedChromaMode::Paeth if self.full_recon => {
                if !self.paeth_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h) {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                }
                reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    false,
                    self.bit_depth,
                )
            }
            SupportedChromaMode::VerticalFollow if self.full_recon => {
                let edge_ok = self.cardinal_edge_reconstructed(
                    IntraCardinalDirection::Vertical,
                    plane_id,
                    mi_col,
                    mi_row,
                    mi_w,
                    mi_h,
                );
                if !edge_ok {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                }
                reconstruct_general_intra_chroma_block_into(
                    &mut self.workspace,
                    block,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    chroma_mode,
                    num4_above_right,
                    num4_below_left,
                    self.bit_depth,
                )
            }
            SupportedChromaMode::HorizontalFollow | SupportedChromaMode::Horizontal
                if self.full_recon =>
            {
                let edge_ok = self.cardinal_edge_reconstructed(
                    IntraCardinalDirection::Horizontal,
                    plane_id,
                    mi_col,
                    mi_row,
                    mi_w,
                    mi_h,
                );
                if !edge_ok {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                }
                reconstruct_general_intra_chroma_block_into(
                    &mut self.workspace,
                    block,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    chroma_mode,
                    num4_above_right,
                    num4_below_left,
                    self.bit_depth,
                )
            }
            SupportedChromaMode::Smooth
            | SupportedChromaMode::SmoothVertical
            | SupportedChromaMode::SmoothHorizontal
                if self.full_recon =>
            {
                let smooth_mode = match chroma_mode {
                    SupportedChromaMode::Smooth => IntraSmoothMode::Smooth,
                    SupportedChromaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
                    SupportedChromaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
                    _ => return Ok(()),
                };
                let edge_samples =
                    self.smooth_edge_availability_samples(plane_id, mi_col, mi_row, mi_w, mi_h);
                let Some((left_samples, above_samples)) = edge_samples else {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                };
                if !self.smooth_edges_reconstructable(
                    plane_id,
                    mi_col,
                    mi_row,
                    mi_w,
                    mi_h,
                    chroma_mode,
                    num4_above_right,
                    num4_below_left,
                    left_samples,
                    above_samples,
                ) {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                }
                reconstruct_general_intra_chroma_smooth_available_edges_into(
                    &mut self.workspace,
                    block,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    smooth_mode,
                    left_samples,
                    above_samples,
                    num4_above_right,
                    num4_below_left,
                    self.bit_depth,
                )
            }
            mode if self.full_recon && log2_width == log2_height => {
                let edges_ok = self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h);
                if !edges_ok {
                    self.defer_chroma_transform(
                        plane_id,
                        chroma_tx,
                        x,
                        y,
                        block,
                        Some(chroma_mode),
                        angle_delta_y,
                        cfl_params,
                        num4_above_right,
                        num4_below_left,
                        qindex,
                        tile_offset,
                    );
                    return Ok(());
                }
                reconstruct_general_intra_chroma_block_into(
                    &mut self.workspace,
                    block,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    mode,
                    num4_above_right,
                    num4_below_left,
                    self.bit_depth,
                )
            }
            _ => return Ok(()),
        }
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_write",
            )
        })?;
        self.finish_chroma_reconstruction(
            plane_id, x, y, chroma_tx, qindex, mi_col, mi_row, mi_w, mi_h,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn smooth_edges_reconstructable(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
        mode: SupportedChromaMode,
        num4_above_right: usize,
        num4_below_left: usize,
        left_samples: usize,
        above_samples: usize,
    ) -> bool {
        let width = mi_w * MI_SIZE;
        let height = mi_h * MI_SIZE;
        if mi_col > 0 && left_samples < height {
            return false;
        }
        if mi_row > 0 && above_samples < width {
            return false;
        }
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        if matches!(
            mode,
            SupportedChromaMode::Smooth | SupportedChromaMode::SmoothVertical
        ) && mi_col > 0
            && num4_below_left > 0
        {
            let Some(left_col) = mi_col.checked_sub(1) else {
                return false;
            };
            if !coverage.is_covered(left_col, mi_row.saturating_add(mi_h)) {
                return false;
            }
        }
        if matches!(
            mode,
            SupportedChromaMode::Smooth | SupportedChromaMode::SmoothHorizontal
        ) && mi_row > 0
            && num4_above_right > 0
        {
            let Some(above_row) = mi_row.checked_sub(1) else {
                return false;
            };
            if !coverage.is_covered(mi_col.saturating_add(mi_w), above_row) {
                return false;
            }
        }
        true
    }

    fn paeth_edges_reconstructed(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) -> bool {
        if !self.dc_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h) {
            return false;
        }
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        match (mi_col.checked_sub(1), mi_row.checked_sub(1)) {
            (Some(left), Some(above)) => coverage.is_covered(left, above),
            _ => true,
        }
    }

    fn chroma_two_sided_middle_edges_reconstructed(
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
        if !covered(left, above) {
            return false;
        }
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return false;
        }
        (mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r))
    }

    fn mark_chroma_coverage(
        &mut self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) {
        let marked = self.coverage[Self::coverage_index(plane_id)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_chroma_4x4 = self.reconstructed_chroma_4x4.saturating_add(marked);
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_chroma_reconstruction(
        &mut self,
        plane_id: PlaneId,
        x: usize,
        y: usize,
        chroma_tx: usize,
        qindex: u32,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        mi_h: usize,
    ) {
        self.record_chroma_deblock_block(plane_id, x, y, chroma_tx, qindex);
        self.mark_chroma_coverage(plane_id, mi_col, mi_row, mi_w, mi_h);
    }

    fn chroma_direction_base_angle(mode: SupportedChromaMode) -> Option<(i32, bool)> {
        match mode {
            SupportedChromaMode::VerticalFollow => Some((90, true)),
            SupportedChromaMode::Vertical => Some((90, false)),
            SupportedChromaMode::HorizontalFollow => Some((180, true)),
            SupportedChromaMode::D45Follow => Some((45, true)),
            SupportedChromaMode::D135Follow => Some((135, true)),
            SupportedChromaMode::D113Follow => Some((113, true)),
            SupportedChromaMode::D157Follow => Some((157, true)),
            SupportedChromaMode::D203Follow => Some((203, true)),
            SupportedChromaMode::D67Follow => Some((67, true)),
            SupportedChromaMode::D45 => Some((45, false)),
            SupportedChromaMode::D67 => Some((67, false)),
            SupportedChromaMode::D135 => Some((135, false)),
            SupportedChromaMode::D113 => Some((113, false)),
            SupportedChromaMode::D203 => Some((203, false)),
            SupportedChromaMode::D157 => Some((157, false)),
            _ => None,
        }
    }

    fn chroma_one_sided_above_edges_reconstructed(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_w: usize,
        num4_above_right: usize,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        let Some(above) = mi_row.checked_sub(1) else {
            return false;
        };
        let Some(corner) = mi_col.checked_sub(1) else {
            return false;
        };
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if !covered(corner, above) {
            return false;
        }
        if !(mi_col..mi_col.saturating_add(mi_w)).all(|c| covered(c, above)) {
            return false;
        }
        let right_edge = mi_col.saturating_add(mi_w);
        (0..num4_above_right).all(|offset| covered(right_edge.saturating_add(offset), above))
    }

    fn chroma_leaf_has_above_row(&self, plane_id: PlaneId, mi_col: usize, mi_row: usize) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        mi_row
            .checked_sub(1)
            .is_some_and(|above| !coverage.off_grid(mi_col, above))
    }

    fn chroma_one_sided_left_edges_reconstructed(
        &self,
        plane_id: PlaneId,
        mi_col: usize,
        mi_row: usize,
        mi_h: usize,
        num4_below_left: usize,
        have_above: bool,
    ) -> bool {
        let coverage = &self.coverage[Self::coverage_index(plane_id)];
        let Some(left) = mi_col.checked_sub(1) else {
            return false;
        };
        let covered = |c: usize, r: usize| !coverage.off_grid(c, r) && coverage.is_covered(c, r);
        if have_above {
            let Some(above) = mi_row.checked_sub(1) else {
                return false;
            };
            if !covered(left, above) {
                return false;
            }
        }
        if !(mi_row..mi_row.saturating_add(mi_h)).all(|r| covered(left, r)) {
            return false;
        }
        let bottom_edge = mi_row.saturating_add(mi_h);
        (0..num4_below_left).all(|offset| covered(left, bottom_edge.saturating_add(offset)))
    }

    #[allow(clippy::too_many_arguments)]
    fn try_reconstruct_chroma_follow_angle(
        &mut self,
        plane_id: PlaneId,
        mode: SupportedChromaMode,
        angle_delta_y: i8,
        chroma_tx: usize,
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
        block: &LumaCoeffBlock,
        num4_above_right: usize,
        num4_below_left: usize,
        qindex: u32,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let Some((base_angle, inherit_luma_delta)) = Self::chroma_direction_base_angle(mode) else {
            return Ok(false);
        };
        let width = 1u32 << log2_width;
        let height = 1u32 << log2_height;
        let angle_delta = if inherit_luma_delta { angle_delta_y } else { 0 };
        if !matches!(
            mode,
            SupportedChromaMode::VerticalFollow
                | SupportedChromaMode::HorizontalFollow
                | SupportedChromaMode::D45Follow
                | SupportedChromaMode::D45
                | SupportedChromaMode::D67Follow
                | SupportedChromaMode::D67
                | SupportedChromaMode::D113Follow
                | SupportedChromaMode::D135Follow
                | SupportedChromaMode::D157Follow
                | SupportedChromaMode::D203Follow
        ) && angle_delta != 0
            && width != height
        {
            return Ok(false);
        }
        let p_angle = wide_angle_mapping(
            width,
            height,
            base_angle + i32::from(angle_delta) * ANGLE_STEP,
        );
        let (mi_col, mi_row) = (x / MI_SIZE, y / MI_SIZE);
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if p_angle == 90 || p_angle == 180 {
            let direction = if p_angle == 90 {
                IntraCardinalDirection::Vertical
            } else {
                IntraCardinalDirection::Horizontal
            };
            if !self.cardinal_edge_reconstructed(direction, plane_id, mi_col, mi_row, mi_w, mi_h) {
                return Ok(false);
            }
            reconstruct_general_intra_cardinal_neighbour_block_into(
                &mut self.workspace,
                block,
                direction,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                false,
                None,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_angle_write",
                )
            })?;
            self.finish_chroma_reconstruction(
                plane_id, x, y, chroma_tx, qindex, mi_col, mi_row, mi_w, mi_h,
            );
            return Ok(true);
        }
        let Ok(p_angle_u16) = u16::try_from(p_angle) else {
            return Ok(false);
        };
        let Ok(angle) = IntraDirectionalAngle::try_from_p_angle(p_angle_u16) else {
            if p_angle <= 90 || p_angle >= 180 {
                return Ok(false);
            }
            if !self
                .chroma_two_sided_middle_edges_reconstructed(plane_id, mi_col, mi_row, mi_w, mi_h)
            {
                return Ok(false);
            }
            let Some(filters) = self.resolve_chroma_two_sided_middle_edge_filters(
                plane_id,
                mi_col,
                mi_row,
                width,
                height,
                p_angle,
                tile_offset,
            )?
            else {
                return Ok(false);
            };
            reconstruct_general_intra_middle_neighbour_rect_block_into(
                &mut self.workspace,
                block,
                p_angle_u16,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                false,
                None,
                self.bit_depth,
                filters,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_angle_write",
                )
            })?;
            self.finish_chroma_reconstruction(
                plane_id, x, y, chroma_tx, qindex, mi_col, mi_row, mi_w, mi_h,
            );
            return Ok(true);
        };
        let not4x4 = !(width == 4 && height == 4);
        let apply_ibp_filter = self.enable_ibp && not4x4;
        let apply_ibp = false;
        let Some(edge_filter) = self.resolve_chroma_one_sided_edge_filter(
            plane_id,
            mi_col,
            mi_row,
            width,
            height,
            p_angle,
            apply_ibp_filter,
            tile_offset,
        )?
        else {
            return Ok(false);
        };
        if apply_ibp {
            let zone1 = p_angle < 90;
            let second_angle_i = if zone1 { p_angle + 180 } else { p_angle - 180 };
            let Ok(second_angle) = u16::try_from(second_angle_i) else {
                return Ok(false);
            };
            if IntraDirectionalAngle::try_from_p_angle(second_angle).is_err() {
                return Ok(false);
            }
            let Some(secondary_edge_filter) = self.resolve_chroma_ibp_secondary_edge_filter(
                plane_id,
                mi_col,
                mi_row,
                width,
                height,
                p_angle,
                tile_offset,
            )?
            else {
                return Ok(false);
            };
            let have_above = self.chroma_leaf_has_above_row(plane_id, mi_col, mi_row);
            let (primary_ok, secondary_ok, primary_num4, secondary_num4) = if zone1 {
                (
                    self.chroma_one_sided_above_edges_reconstructed(
                        plane_id,
                        mi_col,
                        mi_row,
                        mi_w,
                        num4_above_right,
                    ),
                    self.chroma_one_sided_left_edges_reconstructed(
                        plane_id,
                        mi_col,
                        mi_row,
                        mi_h,
                        num4_below_left,
                        have_above,
                    ),
                    num4_above_right,
                    num4_below_left,
                )
            } else {
                (
                    self.chroma_one_sided_left_edges_reconstructed(
                        plane_id,
                        mi_col,
                        mi_row,
                        mi_h,
                        num4_below_left,
                        have_above,
                    ),
                    self.chroma_one_sided_above_edges_reconstructed(
                        plane_id,
                        mi_col,
                        mi_row,
                        mi_w,
                        num4_above_right,
                    ),
                    num4_below_left,
                    num4_above_right,
                )
            };
            if !primary_ok || !secondary_ok {
                return Ok(false);
            }
            reconstruct_general_intra_one_sided_ibp_luma_block_into(
                &mut self.workspace,
                block,
                p_angle_u16,
                plane_id,
                x,
                y,
                log2_width,
                log2_height,
                qindex,
                primary_num4,
                edge_filter,
                IbpSecondary {
                    second_angle,
                    edge_filter: secondary_edge_filter,
                    num4_far: secondary_num4,
                },
                true,
                false,
                None,
                self.bit_depth,
            )
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_angle_write",
                )
            })?;
            self.finish_chroma_reconstruction(
                plane_id, x, y, chroma_tx, qindex, mi_col, mi_row, mi_w, mi_h,
            );
            return Ok(true);
        }
        let write_result = match angle.required_edge() {
            IntraDirectionalAngleEdge::Above => {
                if !self.chroma_one_sided_above_edges_reconstructed(
                    plane_id,
                    mi_col,
                    mi_row,
                    mi_w,
                    num4_above_right,
                ) {
                    return Ok(false);
                }
                reconstruct_general_intra_one_sided_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    p_angle_u16,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    num4_above_right,
                    OneSidedAboveMrl::default(),
                    false,
                    None,
                    self.bit_depth,
                    edge_filter,
                )
            }
            IntraDirectionalAngleEdge::Left => {
                let have_above = self.chroma_leaf_has_above_row(plane_id, mi_col, mi_row);
                if !self.chroma_one_sided_left_edges_reconstructed(
                    plane_id,
                    mi_col,
                    mi_row,
                    mi_h,
                    num4_below_left,
                    have_above,
                ) {
                    return Ok(false);
                }
                reconstruct_general_intra_one_sided_left_neighbour_block_into(
                    &mut self.workspace,
                    block,
                    p_angle_u16,
                    plane_id,
                    x,
                    y,
                    log2_width,
                    log2_height,
                    qindex,
                    num4_below_left,
                    have_above,
                    0,
                    false,
                    None,
                    self.bit_depth,
                    edge_filter,
                )
            }
        };
        write_result.map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_chroma_angle_write",
            )
        })?;
        self.finish_chroma_reconstruction(
            plane_id, x, y, chroma_tx, qindex, mi_col, mi_row, mi_w, mi_h,
        );
        Ok(true)
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
    /// A FRACTIONAL DV (`source` shape one row/col larger than `target`) takes the
    /// §7.13.3.18 bilinear convolution ([`Self::reconstruct_intrabc_fractional_into`])
    /// instead of the copy, but ONLY in full-recon; the gated sink DEFERS it. An
    /// INTEGER DV (same shape) copies the source rect.
    ///
    /// The block is DEFERRED (returns `Ok(())` without writing — never wrong samples
    /// claimed correct) unless ALL of the proven subset holds:
    /// * the frame dequant matches the zero-`QuantizerDeltas` assumption
    ///   (`quant_reconstructable`);
    /// * the block vector is INTEGER (or full-recon, which runs the bilinear path);
    /// * EVERY source MI unit is already reconstructed by this sink — copying an
    ///   unreconstructed (fill) source sample is the cardinal sin. (Full-recon drops
    ///   this conservative coverage gate: it reconstructs every block in decode order,
    ///   so the integer-DV source — always above-left of the target — is written.)
    ///
    /// `source` / `target` are the §7.13.3.18 integer-copy luma rectangles (sample
    /// units); for `fractional`, `source` is ignored and `scaling` drives the
    /// §7.13.3.18 bilinear predictor with reference clipping.
    pub(in crate::runtime_minimal) fn reconstruct_intrabc_block(
        &mut self,
        source: PlaneRect,
        target: PlaneRect,
        scaling: PlaneScaling,
        fractional: bool,
        skip_flag: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if !self.quant_reconstructable {
            return Ok(());
        }
        if fractional {
            if !self.is_full_recon()
                || !self.reconstruct_intrabc_fractional_into(target, scaling, tile_offset)?
            {
                return Ok(());
            }
        } else {
            let coverage = &self.coverage[Self::coverage_index(PlaneId::Y)];
            let src_mi_col = source.x() / MI_SIZE;
            let src_mi_row = source.y() / MI_SIZE;
            let src_mi_w = (source.x() + source.width()).div_ceil(MI_SIZE) - src_mi_col;
            let src_mi_h = (source.y() + source.height()).div_ceil(MI_SIZE) - src_mi_row;
            if !self.full_recon
                && !coverage.region_fully_covered(src_mi_col, src_mi_row, src_mi_w, src_mi_h)
            {
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
        }
        if !skip_flag {
            self.pending_intrabc_predictions.push(target);
            return Ok(());
        }
        let (tgt_mi_col, tgt_mi_row) = (target.x() / MI_SIZE, target.y() / MI_SIZE);
        let (tgt_mi_w, tgt_mi_h) = (target.width() / MI_SIZE, target.height() / MI_SIZE);
        let marked = self.coverage[Self::coverage_index(PlaneId::Y)]
            .mark(tgt_mi_col, tgt_mi_row, tgt_mi_w, tgt_mi_h);
        self.reconstructed_luma_4x4 = self.reconstructed_luma_4x4.saturating_add(marked);
        if self.full_recon {
            self.full_recon_luma_log.push(FullReconLumaLeaf {
                mi_col: tgt_mi_col,
                mi_row: tgt_mi_row,
                x: target.x(),
                y: target.y(),
                width: target.width(),
                height: target.height(),
                mode: "INTRABC",
                written: true,
            });
        }
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
/// region into the sink's workspace in decode order. This is a 10-bit (`u16`)
/// driver for the active Wiener-NS LR selectable-transform path.
#[allow(clippy::too_many_arguments)]
pub(in crate::runtime_minimal) fn reconstruct_ac0ej3_selectable_intra_region(
    bytes: &[u8],
    options: &crate::DecodeOptions,
    plan: &crate::DecodeStreamPlan,
    key_candidate: &crate::DecodePlannedObu,
    key_envelope: splot_core::annexb::ObuEnvelope<'_>,
    sequence: &splot_core::headers::sequence::SequenceHeader,
    core: &splot_core::headers::frame::FrameHeaderCore,
    full_recon: bool,
) -> Result<Ac0ej3SelectableIntraRegion> {
    let frame_size = core.frame_size.ok_or_else(|| {
        super::super::unsupported_at(
            "missing_frame_size_for_recon",
            key_envelope.offset,
            "ac0ej3 reconstruction bridge requires the parsed frame size",
        )
    })?;
    let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())?;
    super::super::ensure_runtime_limits(
        options.limits(),
        frame_size.width,
        frame_size.height,
        0,
        bit_depth,
    )?;
    let enable_ibp = sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_ibp);
    let enable_intra_edge_filter = sequence
        .intra
        .as_ref()
        .is_some_and(|intra| intra.enable_intra_edge_filter);
    let cfl_ds_filter_index = sequence
        .intra
        .as_ref()
        .map_or(0, |intra| intra.cfl_ds_filter_index);
    let sb_size = sequence.partition.as_ref().map_or(
        splot_core::headers::sequence::SuperblockSize::Block64x64,
        splot_core::headers::sequence::SequencePartitionConfig::seq_sb_size,
    );
    let sb_mib = splot_core::tile::num_4x4_blocks_wide(sb_size) as usize;
    let base_sink = WienerNsLrReconSink::<u16>::new(
        frame_size.width as usize,
        frame_size.height as usize,
        bit_depth,
        frame_quant_reconstructable(core),
        enable_ibp,
        enable_intra_edge_filter,
        cfl_ds_filter_index,
        sb_mib,
    )?;
    let mut sink = if full_recon {
        base_sink.into_full_recon()
    } else {
        base_sink
    };
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
        Ok(handoff) => {
            let tx_skip_grid = super::derive_wienerns_lr_tx_skip_grid_retention(
                handoff.tx_skip_rows,
                handoff.tx_skip_cols,
                &handoff.records,
            )?;
            let frame_cdfs = handoff.frame_cdfs;
            sink.set_cdef_grid(handoff.cdef_grid);
            sink.set_ccso_grid(handoff.ccso_grid);
            sink.set_tx_skip_grid(Some(tx_skip_grid));
            sink.set_lr_source_blocks(handoff.active_source_blocks);
            sink.set_lr_unit_filters(handoff.unit_filters);
            Ok(Ac0ej3SelectableIntraRegion { sink, frame_cdfs })
        }
        Err(crate::error::DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == EXPECTED_RECON_FRONTIER_REASON =>
        {
            Ok(Ac0ej3SelectableIntraRegion {
                sink,
                frame_cdfs: FrameCdfSubset::from_defaults(),
            })
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
const EXPECTED_RECON_FRONTIER_REASON: &str =
    "unsupported_wienerns_lr_selectable_live_frame_samples_unpopulated";

fn mhccp_luma_ref_available(
    row: usize,
    col: usize,
    above: usize,
    left: usize,
    width: usize,
    height: usize,
) -> bool {
    (row < above || col < left.saturating_add(width))
        && (row < above.saturating_add(height) || col < left)
}

/// Whether the frame's §5.18.6 quantization matches the reconstruction primitive's
/// zero-`QuantizerDeltas` assumption: no per-plane DC/AC quantizer delta and no
/// quantizer matrix. When `false` the sink must reconstruct nothing (the primitive
/// would dequantize with the wrong DC/AC quantizers), so the gate defers — the safe
/// choice. ac0ej3's verified frame has no such delta.
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

/// § 7.17 chroma deblock geometry for one 4:2:0 chroma transform at
/// plane-sample (`x`, `y`): chroma MI cells map ×2 onto the luma MI grid.
/// Returns the chroma list index (U = 0, V = 1) with the record.
pub(in crate::runtime_minimal) fn chroma_transform_deblock_block(
    plane_id: PlaneId,
    x: usize,
    y: usize,
    chroma_tx: usize,
    qindex: u32,
) -> Option<(usize, super::super::deblock::DeblockBlock)> {
    let (log2_width, log2_height) = tx_size_log2(chroma_tx)?;
    let plane_index = match plane_id {
        PlaneId::U => 0,
        PlaneId::V => 1,
        PlaneId::Y => return None,
    };
    let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
    Some((
        plane_index,
        super::super::deblock::DeblockBlock {
            r: (y / MI_SIZE).saturating_mul(2),
            c: (x / MI_SIZE).saturating_mul(2),
            block_r: (y / MI_SIZE).saturating_mul(2),
            block_c: (x / MI_SIZE).saturating_mul(2),
            chroma_base_r: (y / MI_SIZE).saturating_mul(2),
            chroma_base_c: (x / MI_SIZE).saturating_mul(2),
            n4w: mi_w.saturating_mul(2),
            n4h: mi_h.saturating_mul(2),
            luma_tx: chroma_tx,
            chroma_tx: Some(chroma_tx),
            qindex,
            skip: false,
        },
    ))
}

/// Maps a §5.20.6 `TxSize` index to its `(log2_width, log2_height)` sample
/// dimensions via the §9 `Tx_Width` / `Tx_Height` log2 tables, or `None` when the
/// index is outside the 19-entry table range.
fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    let w = u32::try_from(*TX_WIDTH_LOG2.get(tx_size)?).ok()?;
    let h = u32::try_from(*TX_HEIGHT_LOG2.get(tx_size)?).ok()?;
    Some((w, h))
}

/// `SPLOT_DUMP_PREFILTER` env var naming an append-target path: every
/// reconstructed frame's workspace appends as raw u16-LE I420 before the
/// § 7.2 filter chain runs. Diagnostics only; inert without the env var.
fn dump_prefilter_frame_for_diagnostics<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    luma_width: usize,
    luma_height: usize,
) {
    let Some(path) = std::env::var_os("SPLOT_DUMP_PREFILTER") else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let mut bytes: Vec<u8> = Vec::new();
    for (plane, w, h) in [
        (PlaneId::Y, luma_width, luma_height),
        (PlaneId::U, luma_width.div_ceil(2), luma_height.div_ceil(2)),
        (PlaneId::V, luma_width.div_ceil(2), luma_height.div_ceil(2)),
    ] {
        let Ok(rect) = PlaneRect::new(0, 0, w, h) else {
            continue;
        };
        let Ok(rows) = workspace.rect_rows(plane, rect) else {
            continue;
        };
        for row in rows {
            bytes.extend(row.iter().flat_map(|&s| s.to_u16().to_le_bytes()));
        }
    }
    let _ = std::io::Write::write_all(&mut file, &bytes);
}

/// The §3 sample-space `(x, y)` origin of a luma MI position, overflow-checked.
pub(super) fn luma_sample_origin(
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
    !(block_has_real_ist(block) || fsc_mode)
}

fn block_has_real_ist(block: &LumaCoeffBlock) -> bool {
    block.intra_ist.is_some_and(|ist| ist.sec_tx_type != 0)
}

fn full_recon_mode_uses_supported_directional_edge(
    mode: IntraYMode,
    angle_delta_y: i8,
    mrl_index: u8,
    mrl_sec_index: Option<u8>,
    log2_width: u32,
    log2_height: u32,
) -> bool {
    if mrl_sec_index == Some(1) || !mode.is_directional() {
        return false;
    }
    let Some(nominal) = mode.mode_to_angle() else {
        return false;
    };
    let Some(w) = 1u32.checked_shl(log2_width) else {
        return false;
    };
    let Some(h) = 1u32.checked_shl(log2_height) else {
        return false;
    };
    let Some(&mrl_delta) = MRL_INDEX_TO_DELTA.get(usize::from(mrl_index)) else {
        return false;
    };
    let Some(nominal_angle) = i32::from(nominal)
        .checked_add(i32::from(angle_delta_y) * ANGLE_STEP)
        .and_then(|angle| angle.checked_add(mrl_delta))
    else {
        return false;
    };
    let p_angle = wide_angle_mapping(w, h, nominal_angle);
    (0 < p_angle && p_angle < 90)
        || (90 < p_angle && p_angle < 180)
        || (180 < p_angle && p_angle < 270)
}

mod edge_filter;
mod final_filters;
pub(in crate::runtime_minimal) mod full_recon;
use full_recon::{
    ANGLE_STEP, FarEdgeSide, MRL_INDEX_TO_DELTA, full_recon_deferred_leaf_error,
    full_recon_mode_label, wide_angle_mapping,
};

#[cfg(test)]
#[path = "recon_tests.rs"]
mod tests;
