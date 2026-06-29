// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! DIAGNOSTIC-ONLY full-reconstruction support for the ac0ej3 sink.
//!
//! Test-only (`#[cfg(test)]`) infrastructure powering the `SPLOT_AC0EJ3_FULL_RECON`
//! whole-frame differential: a per-leaf decode-order log, the per-transform far-edge
//! `num4` sources that replace the conservative coverage gates in full-recon mode,
//! and the primitive-error swallowing that lets one unwired frame-edge / mode case
//! defer instead of aborting the walk. None of this affects the shipped gated sink
//! (every method is a no-op or unused unless [`WienerNsLrReconSink::into_full_recon`]
//! flips `full_recon` to `true`).

use super::{FullReconLumaLeaf, MI_SIZE, WienerNsLrReconSink};
use crate::Result;
use crate::runtime_minimal::wienerns_lr::diagnostics::wienerns_lr_selectable_transform_record_error_reason;
use crate::tile_payload::{GeneralIntraResidualError, IntraYMode, SupportedDirectionalLumaMode};
use splot_core::span::ByteOffset;
use splot_recon::ReconSample;

impl<T: ReconSample> WienerNsLrReconSink<T> {
    /// Switches this sink into the DIAGNOSTIC-ONLY full-reconstruction mode (see the
    /// `full_recon` field). Used ONLY by the `SPLOT_AC0EJ3_FULL_RECON` ignored
    /// harness, which reconstructs every luma leaf in decode order and diffs the whole
    /// frame against the AVM pre-filter oracle. The shipped oracle-pin tests never call
    /// this, so the gated path is untouched.
    pub(in crate::runtime_minimal) fn into_full_recon(mut self) -> Self {
        self.full_recon = true;
        self
    }

    /// The recorded per-transform §7.13.2.1 `num4AboveRight` for the luma MI unit at
    /// `(mi_col, mi_row)` (the AVM `has_top_right` count, in 4x4 units), or `0` when no
    /// transform has been recorded there. Full-recon-only: the directional read-or-pad
    /// bound replacing the conservative coverage count.
    pub(super) fn far_edge_above_right(&self, mi_col: usize, mi_row: usize) -> usize {
        self.far_edge_avail
            .get(mi_col, mi_row)
            .map_or(0, |(above_right, _)| above_right as usize)
    }

    /// The recorded per-transform §7.13.2.1 `num4BelowLeft` for the luma MI unit at
    /// `(mi_col, mi_row)` (the AVM `has_bottom_left` count, in 4x4 units), or `0` when
    /// no transform has been recorded there. Full-recon-only.
    pub(super) fn far_edge_below_left(&self, mi_col: usize, mi_row: usize) -> usize {
        self.far_edge_avail
            .get(mi_col, mi_row)
            .map_or(0, |(_, below_left)| below_left as usize)
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
    pub(in crate::runtime_minimal) fn full_recon_luma_log(&self) -> &[FullReconLumaLeaf] {
        &self.full_recon_luma_log
    }

    /// Resolves a §7.13.2 luma predictor primitive result. Returns `Ok(true)` when the
    /// primitive wrote the leaf (the caller then marks it). On a primitive error: the
    /// GATED path maps it to the named transform-record diagnostic and propagates (the
    /// shipped fail-closed behaviour); the FULL-RECON path treats it as a DEFERRED leaf
    /// (records it unwritten and returns `Ok(false)`) so one unwired frame-edge / mode
    /// case does not abort the whole-frame differential. `reason` is the diagnostic
    /// reason string for the gated propagation.
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
            return Ok(false);
        }
        Err(wienerns_lr_selectable_transform_record_error_reason(
            tile_offset,
            reason,
        ))
    }
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
