// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Full-reconstruction support for the ac0ej3 sink.
//!
//! Full-reconstruction support for the ac0ej3 sink: a per-leaf decode-order log and
//! the per-transform far-edge `num4` sources that replace the conservative coverage
//! gates in full-recon mode. The gated sink stays unchanged; full recon fails loud
//! when a leaf cannot be reconstructed.

use splot_core::span::ByteOffset;

use super::{FullReconLumaLeaf, MI_SIZE, WienerNsLrReconSink};
use crate::Result;
use crate::runtime_minimal::inter::mv_scaling::PlaneScaling;
use crate::runtime_minimal::wienerns_lr::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use crate::tile_payload::{GeneralIntraResidualError, IntraYMode, SupportedDirectionalLumaMode};
use splot_recon::{
    BitDepth, InterpolationFilter, PlaneId, PlaneRect, ReconSample, ReferencePlaneView,
    SubpelPredictParams, subpel_predict_block,
};

pub(super) fn full_recon_deferred_leaf_error(offset: ByteOffset) -> crate::error::DecodeError {
    wienerns_lr_selectable_transform_record_error_reason(
        offset,
        "unsupported_wienerns_lr_selectable_transform_records_full_recon_deferred_leaf",
    )
}

/// Which §7.13.2.1 far edge a directional one-sided leaf reads: zone-1 above-right or
/// zone-3 below-left. Selects the [`WienerNsLrReconSink::full_recon_far_edge`] override
/// component for the symmetric coverage helpers.
#[derive(Clone, Copy)]
pub(super) enum FarEdgeSide {
    /// `num4AboveRight` (AVM `has_top_right`) for a zone-1 above-reading leaf.
    AboveRight,
    /// `num4BelowLeft` (AVM `has_bottom_left`) for a zone-3 left-reading leaf.
    BelowLeft,
}

/// Builds the §7.13.3.18 `BILINEAR` IntrABC sub-pel parameters for a `w` x `h`
/// target from its §7.13.3.17 `scaling` (`startX` / `startY` / `stepX` / `stepY`
/// and the `firstX..lastX` clip bounds). Shared by the full-recon fractional
/// predictor and its unit test so the reference is the same parameter mapping.
pub(in crate::runtime_minimal) fn intrabc_bilinear_params(
    scaling: PlaneScaling,
    w: usize,
    h: usize,
    bit_depth: BitDepth,
) -> SubpelPredictParams {
    SubpelPredictParams {
        interp: InterpolationFilter::Bilinear,
        w,
        h,
        start_x: scaling.start_x,
        start_y: scaling.start_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: scaling.first_x,
        first_y: scaling.first_y,
        last_x: scaling.last_x,
        last_y: scaling.last_y,
        bit_depth,
    }
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    /// Switches this sink into the full-reconstruction mode (see the `full_recon`
    /// field), reconstructing every luma leaf in decode order with recorded
    /// per-transform far-edge availability.
    pub(in crate::runtime_minimal) fn into_full_recon(mut self) -> Self {
        self.full_recon = true;
        self
    }

    /// Whether this sink is in the DIAGNOSTIC-ONLY full-reconstruction mode. The
    /// fractional-DV IntrABC bilinear predictor runs ONLY here (the shipped gated
    /// sink still DEFERS a fractional-DV block); the gated copy path is untouched.
    pub(in crate::runtime_minimal) const fn is_full_recon(&self) -> bool {
        self.full_recon
    }

    /// Reconstructs a fractional-DV §7.13.3.18 IntrABC luma block into the `target`
    /// rect by the bilinear sub-pel convolution, returning whether the predictor was
    /// written (`false` keeps the workspace fill value, e.g. for a non-`u16` storage
    /// type the full-recon harness never uses). The displaced predictor is the
    /// §7.13.3.13 / §7.13.3.18 `block_inter_prediction(refIdx = -1, …, BILINEAR)`:
    /// the §7.13.3.17 `scaling` (`startX` / `startY` / `stepX` / `stepY`, already
    /// derived from the block position + eighth-pel DV) drives the two-pass
    /// separable convolution over `CurrFrame` (the workspace luma plane is the
    /// reference, `ref = CurrFrame` for `refIdx == -1`). `Subpel_Filters[BILINEAR]`
    /// (the spec's 2-tap row at taps 3/4) carries the fractional weights; the
    /// `InterRound0 = 3` / `InterRound1 = 11` rounding and the final §4.8 `Clip1`
    /// are [`subpel_predict_block`]'s.
    ///
    /// This is the predictor an INTEGER-DV block writes via the plain rect copy; for
    /// a fractional DV (`block_mv & 7 != 0`) the copy is replaced by this
    /// convolution. The caller has cleared the §6.19 DV-validity gate, so the whole
    /// `-3..=+4`-tap reference window lies in the already-reconstructed region
    /// (the source is above-left of the target in decode order).
    pub(super) fn reconstruct_intrabc_fractional_into(
        &mut self,
        target: PlaneRect,
        scaling: PlaneScaling,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        let storage = self.workspace.plane(PlaneId::Y)?.storage_size();
        let (width, height) = (storage.width(), storage.height());
        let reference: Vec<u16> = self
            .workspace
            .samples(PlaneId::Y)?
            .iter()
            .map(|sample| sample.to_u16())
            .collect();
        let view = ReferencePlaneView::new(&reference, width, height).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_copy",
            )
        })?;
        let params =
            intrabc_bilinear_params(scaling, target.width(), target.height(), self.bit_depth);
        let predicted = subpel_predict_block(&view, &params).map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_intrabc_copy",
            )
        })?;
        let mut typed: Vec<T> = Vec::with_capacity(predicted.len());
        for sample in predicted {
            let Ok(value) = T::try_from_u16(sample) else {
                return Ok(false);
            };
            typed.push(value);
        }
        self.workspace
            .write_rect(PlaneId::Y, target, &typed, target.width())
            .map_err(|_| {
                wienerns_lr_selectable_transform_record_error_reason(
                    tile_offset,
                    "unsupported_wienerns_lr_selectable_transform_records_intrabc_copy",
                )
            })?;
        Ok(true)
    }

    /// The full-recon directional read-or-pad `num4` for one far edge of the luma MI
    /// unit at `(mi_col, mi_row)`, or `None` for the GATED sink (the coverage-counting
    /// path then runs). In full-recon this is the recorded per-transform §7.13.2.1
    /// availability (AVM `has_top_right` / `has_bottom_left`, in 4x4 units), `0` when no
    /// transform was recorded — replacing the conservative coverage count. Shared by
    /// the symmetric above-right / below-left coverage helpers so the override lives
    /// once.
    pub(super) fn full_recon_far_edge(
        &self,
        mi_col: usize,
        mi_row: usize,
        side: FarEdgeSide,
    ) -> Option<usize> {
        if !self.full_recon {
            return None;
        }
        let (above_right, below_left) = self
            .far_edge_avail
            .get(mi_col, mi_row)
            .map_or((0, 0), |(ar, bl)| (ar as usize, bl as usize));
        Some(match side {
            FarEdgeSide::AboveRight => above_right,
            FarEdgeSide::BelowLeft => below_left,
        })
    }

    /// Appends one decode-order luma leaf to the full-reconstruction diagnostic log.
    /// A NO-OP unless `full_recon` is set, so the shipped gated sink never allocates or
    /// records — its behaviour is byte-identical with or without this call. `written`
    /// is `true` when the dispatch wrote a real predictor, `false` when it left the
    /// workspace fill value (a deferred / unwired leaf).
    pub(super) fn record_full_recon_leaf(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        mode: &'static str,
        written: bool,
    ) {
        if !self.full_recon {
            return;
        }
        let width = 1usize << log2_width;
        let height = 1usize << log2_height;
        let (x, y) = (mi_col * MI_SIZE, mi_row * MI_SIZE);
        self.full_recon_luma_log.push(FullReconLumaLeaf {
            mi_col,
            mi_row,
            x,
            y,
            width,
            height,
            mode,
            written,
        });
    }

    /// The decode-order luma leaf log captured in full-reconstruction mode (empty for
    /// a gated sink). The `SPLOT_AC0EJ3_FULL_RECON` harness replays this to locate the
    /// FIRST decode-order block whose samples diverge from the AVM oracle.
    #[cfg(test)]
    pub(in crate::runtime_minimal) fn full_recon_luma_log(&self) -> &[FullReconLumaLeaf] {
        &self.full_recon_luma_log
    }

    /// Records a DEFERRED (unwritten) luma leaf. The gated sink keeps the existing
    /// early-return behavior; full recon converts the defer into an Unsupported
    /// diagnostic so runtime output cannot freeze fill samples as decoded pixels.
    pub(super) fn defer_full_recon_leaf(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        mode: &'static str,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_FULL_RECON_DEFER") {
            eprintln!(
                "full_recon_defer mi=({}, {}) log2={}x{} mode={} offset={}",
                mi_col,
                mi_row,
                log2_width,
                log2_height,
                mode,
                tile_offset.get()
            );
        }
        self.record_full_recon_leaf(mi_col, mi_row, log2_width, log2_height, mode, false);
        if self.full_recon {
            return Err(full_recon_deferred_leaf_error(tile_offset));
        }
        Ok(())
    }

    /// Resolves a §7.13.2 luma predictor primitive result. Returns `Ok(true)` when the
    /// primitive wrote the leaf (the caller then marks it). On a primitive error: the
    /// GATED path maps it to the named transform-record diagnostic and propagates; the
    /// full-recon path records the unwritten leaf and returns Unsupported. `reason` is
    /// the diagnostic reason string for the gated propagation.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish_luma_predict(
        &mut self,
        result: &core::result::Result<(), GeneralIntraResidualError>,
        mi_col: usize,
        mi_row: usize,
        log2_width: u32,
        log2_height: u32,
        mode: &'static str,
        tile_offset: ByteOffset,
        reason: &'static str,
    ) -> Result<bool> {
        if result.is_ok() {
            return Ok(true);
        }
        if self.full_recon {
            self.record_full_recon_leaf(mi_col, mi_row, log2_width, log2_height, mode, false);
            return Err(full_recon_deferred_leaf_error(tile_offset));
        }
        Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            reason,
        ))
    }

    /// Collapses a [`Self::try_reconstruct_one_sided_angular`] routing outcome into
    /// "did the angular path write the leaf". `Ok(true)` reconstructed it; `Ok(false)`
    /// DEFERRED it. A primitive error propagates in the gated sink and becomes an
    /// Unsupported diagnostic in full-recon output.
    pub(super) fn routed_angular_wrote(
        &self,
        routed: Result<bool>,
        tile_offset: ByteOffset,
    ) -> Result<bool> {
        match routed {
            Ok(wrote) => Ok(wrote),
            Err(_) if self.full_recon => Err(full_recon_deferred_leaf_error(tile_offset)),
            Err(error) => Err(error),
        }
    }
}

/// Which §7.13.2.1 reference edge a one-sided filter is being assembled for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EdgeOrientation {
    /// The above row `AboveRow[..]` (zone-1 read edge / zone-3 IBP secondary edge).
    Above,
    /// The left column `LeftCol[..]` (zone-3 read edge / zone-1 IBP secondary edge).
    Left,
}

/// The resolved §7.13.2.7 inputs for ONE one-sided reference edge: the edge
/// orientation, the §7.13.2.15/16 `filterType` (smooth-neighbour flag), the
/// §7.13.2.7 `angleAbove`/`angleLeft` delta, and whether the far extension
/// (above-right / below-left) is needed for the §7.13.2.18 sweep span.
#[derive(Clone, Copy, Debug)]
pub(super) struct OneSidedEdgeSpec {
    pub orientation: EdgeOrientation,
    pub filter_type: bool,
    pub angle_delta: i32,
    pub need_far: bool,
}

/// AV2 §5 `ANGLE_STEP`: degrees of angle change per unit `AngleDeltaY`.
pub(super) const ANGLE_STEP: i32 = 3;
/// AV2 §5.20.7.27 `Mrl_Index_To_Delta[4]` (the multi-reference-line angle nudge).
pub(super) const MRL_INDEX_TO_DELTA: [i32; 4] = [0, 1, -1, 0];
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
pub(super) fn wide_angle_mapping(w: u32, h: u32, p_angle: i32) -> i32 {
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

/// Human label for a luma leaf's §7.13.2 prediction mode, for the full-reconstruction
/// decode-order diagnostic. Maps the §9.2 canonical `IntraYMode` value to its mode
/// name; an IntrABC leaf is labelled `"INTRABC"` regardless of its (unused) intra
/// mode. Used ONLY by the `SPLOT_AC0EJ3_FULL_RECON` harness reporting.
pub(super) fn full_recon_mode_label(
    leaf_y_mode: Option<IntraYMode>,
    directional: Option<SupportedDirectionalLumaMode>,
    is_intrabc: bool,
) -> &'static str {
    if is_intrabc {
        return "INTRABC";
    }
    match directional {
        Some(SupportedDirectionalLumaMode::Vertical) => return "V_PRED",
        Some(SupportedDirectionalLumaMode::Horizontal) => return "H_PRED",
        _ => {}
    }
    let Some(mode) = leaf_y_mode else {
        return "UNKNOWN";
    };
    match mode.value() {
        0 => "DC_PRED",
        1 => "V_PRED",
        2 => "H_PRED",
        3 => "D45_PRED",
        4 => "D135_PRED",
        5 => "D113_PRED",
        6 => "D157_PRED",
        7 => "D203_PRED",
        8 => "D67_PRED",
        9 => "SMOOTH_PRED",
        10 => "SMOOTH_V_PRED",
        11 => "SMOOTH_H_PRED",
        12 => "PAETH_PRED",
        _ => "UNKNOWN",
    }
}
