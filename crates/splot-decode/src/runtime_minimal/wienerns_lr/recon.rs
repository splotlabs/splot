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
//! The bridge is a TEST instrument: the public `splot decode` path runs the walk
//! WITHOUT a sink, so it still fails closed at the first active IntrABC block and
//! emits no frame. A region-verification test attaches a sink, lets the walk run
//! until it rejects at IntrABC, and asserts the populated workspace region is
//! bit-exact against the AVM pre-filter reconstruction oracle.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, PlaneRect, ReconSample,
};

use crate::Result;
#[cfg(test)]
use crate::runtime_minimal_recon::new_general_intra_workspace;
use crate::runtime_minimal_recon::{
    reconstruct_general_intra_block_rect_into,
    reconstruct_general_intra_cardinal_neighbour_block_into,
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
    pub(in crate::runtime_minimal) chroma_mode: Option<SupportedChromaMode>,
    pub(in crate::runtime_minimal) qindex: u32,
    pub(in crate::runtime_minimal) luma_use_tcq: bool,
    pub(in crate::runtime_minimal) fsc_mode: bool,
    /// Whether this leaf is a §5.20.5.3 `use_intrabc` block. An IntrABC leaf's
    /// luma samples are reconstructed by
    /// [`WienerNsLrReconSink::reconstruct_intrabc_block`] (a §7.13.3.18 displaced
    /// `CurrFrame` copy) inside `read_intrabc_info`, BEFORE the skip-residual path
    /// runs. The residual path's flat §7.13.2 DC/cardinal prediction must NOT then
    /// overwrite that copy with a spurious `DC_PRED` block (an IntrABC leaf's
    /// `leaf_y_mode` is a placeholder `DC_PRED`, §5.20.5.3 reads no intra Y mode), so
    /// the skip-residual reconstruction is skipped for an IntrABC leaf — the IntrABC
    /// sink already owns its samples.
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
    /// Per-plane MI-unit coverage (`coverage[plane]`, row-major over the plane's MI
    /// grid): luma plane 0, chroma U plane 1, chroma V plane 2. U and V are tracked
    /// SEPARATELY — a reconstructed U must not let a deferred V block pass the
    /// DC-edge guard (4:2:0 U and V share MI dimensions but not reconstruction
    /// state). `true` where the sink has written spec-correct samples.
    coverage: [PlaneCoverage; 3],
    reconstructed_luma_4x4: usize,
    reconstructed_chroma_4x4: usize,
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

    fn mark(&mut self, mi_col: usize, mi_row: usize, mi_w: usize, mi_h: usize) {
        for r in mi_row..mi_row.saturating_add(mi_h) {
            for c in mi_col..mi_col.saturating_add(mi_w) {
                if let Some(slot) = self.covered.get_mut(r * self.cols + c) {
                    *slot = true;
                }
            }
        }
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
    ) -> Result<Self> {
        // 4:2:0 chroma planes are half the luma dimensions in each axis.
        let chroma_width = luma_width.div_ceil(2);
        let chroma_height = luma_height.div_ceil(2);
        Ok(Self {
            workspace: new_general_intra_workspace::<T>(luma_width, luma_height, bit_depth)?,
            bit_depth,
            quant_reconstructable,
            enable_ibp,
            coverage: [
                PlaneCoverage::new(luma_width, luma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
            ],
            reconstructed_luma_4x4: 0,
            reconstructed_chroma_4x4: 0,
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

    /// Whether the §7.13.2.8 cardinal edge a block at `(mi_col, mi_row)` of `mi_w` x
    /// `mi_h` MI units reads is a REAL reconstructed neighbour. A cardinal mode reads
    /// exactly ONE edge — `H_PRED` (pAngle 180) the left column (`pred[i][j] =
    /// LeftCol[i]`), `V_PRED` (pAngle 90) the above row (`pred[i][j] = AboveRow[j]`)
    /// — with no corner, no IDIF, and no `useIBP`. Every MI unit of that edge (the
    /// left column `mi_col - 1` for H, the above row `mi_row - 1` for V) must exist
    /// on-grid AND be reconstructed by this sink. A frame-edge block with no on-grid
    /// edge DEFERS: the cardinal primitive has no real neighbour to copy (the
    /// §7.13.2.1 no-neighbour fallback is a separate, here-unmodelled path).
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
        match direction {
            // H_PRED: the left column `mi_col - 1` over rows `mi_row..mi_row + mi_h`.
            IntraCardinalDirection::Horizontal => {
                let Some(left) = mi_col.checked_sub(1) else {
                    return false;
                };
                (mi_row..mi_row.saturating_add(mi_h))
                    .all(|r| !coverage.off_grid(left, r) && coverage.is_covered(left, r))
            }
            // V_PRED: the above row `mi_row - 1` over cols `mi_col..mi_col + mi_w`.
            IntraCardinalDirection::Vertical => {
                let Some(above) = mi_row.checked_sub(1) else {
                    return false;
                };
                (mi_col..mi_col.saturating_add(mi_w))
                    .all(|c| !coverage.off_grid(c, above) && coverage.is_covered(c, above))
            }
        }
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
    ///   an `all_zero` flat block, or a square non-`all_zero` block with no IST
    ///   and no FSC);
    /// * the leaf mode is one the sink can predict bit-exact AND its required
    ///   §7.13.2 prediction neighbours are off-frame or already reconstructed:
    ///   - `DC_PRED` (the §7.13.2.10 flat DC, with the §7.13.2.12 IBP DC modifier
    ///     when [`Self::ibp_dc_applies`]): both the above row and left column must
    ///     be off-frame or covered ([`Self::dc_edges_reconstructed`]);
    ///   - cardinal `H_PRED` (pAngle 180, `directional ==
    ///     Some(Horizontal)`): the §7.13.2.8 step-5 left-column copy. The left
    ///     column must be present and covered ([`Self::cardinal_left_reconstructed`]);
    ///     it reads ONLY the left column (no above, no corner, no IDIF, no `useIBP`),
    ///     the transform must be SQUARE (the cardinal primitive is square-only), the
    ///     leaf must use the immediate edge (`mrl_index == 0` — the primitive reads
    ///     the adjacent left/above samples, not a §5.20.5.5 multi-reference line),
    ///     and a non-`all_zero` residual must be DC-only (`eob == 1`): a non-DC-only
    ///     residual may carry a non-`DCT_DCT` luma tx type that `LumaCoeffBlock` does
    ///     not retain, and the primitive always inverse-transforms `DCT_DCT`;
    ///   - cardinal `V_PRED` (pAngle 90, `directional == Some(Vertical)`): the
    ///     §7.13.2.8 step-4 above-row copy. The above row must be present and covered
    ///     ([`Self::cardinal_above_reconstructed`]); same square / `mrl_index == 0` /
    ///     DC-only-residual gates.
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
        if is_intrabc {
            // A §7.13.3.18 IntrABC leaf's luma prediction is the displaced `CurrFrame`
            // copy `reconstruct_intrabc_block` performs from the block vector, NOT a
            // §7.13.2 intra prediction — the `leaf_y_mode` is a §5.20.5.3 placeholder
            // `DC_PRED` (no intra Y mode is read for IntrABC). Reconstructing here via
            // that placeholder would overwrite the correct IntrABC copy with a spurious
            // DC block. The non-skip IntrABC residual add (copy + inverse-transform of
            // this transform's coefficients) is a separate proven brick, so the sink
            // defers the residual write — the entropy coefficients were already
            // consumed AVM-faithfully by the caller; only the sample write is deferred.
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return Ok(());
        };
        if !residual_is_reconstructable(block, fsc_mode, log2_width == log2_height) {
            return Ok(());
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
                // recon primitive is SQUARE-only and reads the IMMEDIATE edge
                // (`MrlIndex == 0`), so defer otherwise:
                // * a non-square cardinal transform (the primitive is square-only);
                // * a §5.20.5.5 multi-reference line (`mrl_index > 0`): the primitive
                //   copies the ADJACENT left/above samples, not the selected MRL
                //   reference line, so it would write the wrong prediction;
                // * a non-`all_zero` residual that is not DC-only (`eob != 1`): it may
                //   carry a non-`DCT_DCT` luma tx type `LumaCoeffBlock` does not retain
                //   (the primitive always inverse-transforms `DCT_DCT`). A DC-only
                //   (`eob == 1`) residual is tx-type-agnostic (one shared DC coeff),
                //   exactly ac0ej3's +4-DC top-right `TX_32X32`.
                if log2_width != log2_height {
                    return Ok(());
                }
                if mrl_index != 0 {
                    return Ok(());
                }
                if !block.all_zero && block.eob != 1 {
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
            // Non-DC, non-cardinal luma (SMOOTH / PAETH / angular / non-zero
            // AngleDeltaY): defer rather than emit an unproven prediction.
            _ => return Ok(()),
        }
        self.coverage[Self::coverage_index(PlaneId::Y)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_luma_4x4 = self
            .reconstructed_luma_4x4
            .saturating_add((1usize << log2_width >> 2) * (1usize << log2_height >> 2));
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
        if !residual_is_reconstructable(block, false, log2_width == log2_height) {
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
        self.coverage[Self::coverage_index(plane_id)].mark(mi_col, mi_row, mi_w, mi_h);
        self.reconstructed_chroma_4x4 = self
            .reconstructed_chroma_4x4
            .saturating_add((1usize << log2_width >> 2) * (1usize << log2_height >> 2));
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
    /// The block is DEFERRED (returns `Ok(())` without writing — never wrong samples
    /// claimed correct) unless ALL of the proven subset holds:
    /// * the frame dequant matches the zero-`QuantizerDeltas` assumption
    ///   (`quant_reconstructable`);
    /// * the block is a `skip` block (zero residual) — a non-skip IntrABC residual
    ///   needs the dequant / inverse-transform / residual-add path this brick has not
    ///   proven for the IntrABC tx type, so it is deferred;
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
        if !self.quant_reconstructable || !skip_flag {
            // Defer a frame whose dequant the primitive cannot honor, or a non-skip
            // IntrABC block whose residual this brick has not proven bit-exact.
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
        let (tgt_mi_col, tgt_mi_row) = (target.x() / MI_SIZE, target.y() / MI_SIZE);
        let (tgt_mi_w, tgt_mi_h) = (target.width() / MI_SIZE, target.height() / MI_SIZE);
        self.coverage[Self::coverage_index(PlaneId::Y)]
            .mark(tgt_mi_col, tgt_mi_row, tgt_mi_w, tgt_mi_h);
        self.reconstructed_luma_4x4 = self
            .reconstructed_luma_4x4
            .saturating_add(tgt_mi_w * tgt_mi_h);
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
    let mut sink = WienerNsLrReconSink::<u16>::new(
        frame_size.width as usize,
        frame_size.height as usize,
        bit_depth,
        frame_quant_reconstructable(core),
        enable_ibp,
    )?;
    // The walk reconstructs into the sink in decode order. With the AVM-faithful
    // §5.20.3.1 SDP chroma partition plane (plane 1 for the chroma tree) and the
    // §8.3.2 `is_cfl` neighbour-context fix (the chroma `is_cfl` CDF is keyed by the
    // above/left `UVCfls` neighbours, not a hardcoded `ctx == 0`), the parse stays
    // entropy-synced past the second superblock and stops at the first active IntrABC
    // block (§7.13.3.18), which needs populated `CurrFrame` samples for prediction;
    // the owned sink retains the region reconstructed before that expected rejection.
    // Swallow ONLY that known recon-subset frontier — any other error (an earlier
    // parse or reconstruction failure, e.g. a regression that fails before the
    // frontier after the verified region is written, OR a re-introduced earlier
    // desync) is propagated so the test fails loudly instead of silently passing on a
    // partial walk.
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
/// and propagates every other error. The full §7.12.2.6 above-row IntrABC SMVP scan
/// is now modelled — the within-SB (non-SB-border) steps 8/10/12/14 at 4x4
/// resolution AND the SB-border steps 8/10/12/14 8x8-aligned in the even-MiCol
/// no-op case — so the frame-0 IntrABC blocks MI(48,56) through MI(208,56) (and
/// siblings) admit their ref-MV stacks faithfully (bit-exact vs avmdec) instead of
/// deferring. The walk now advances to the new frontier MI(192,112): a `BLOCK_64X32`
/// IntrABC block with TWO distinct spatial candidates ((-1024,0) step 7 + (-512,0)
/// step 8), so the §7.12.2.19 max-weight DRL-reorder applies — a DISTINCT mechanism
/// (per-candidate §7.12.2.6 weighting, not an above-row position) this decoder does
/// not model. The admission DEFERS via the existing >1-distinct-spatial-candidate
/// guard, on the same `intrabc_ref_stack` reason — the genuinely distinct next wall.
#[cfg(test)]
const EXPECTED_RECON_FRONTIER_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_intrabc_ref_stack";

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
/// bare §7.13.2 flat DC prediction. A non-`all_zero` block is admitted when it has
/// no §5.20.7.29 IST secondary transform (`intra_ist`) and is not an FSC leaf.
///
/// A SQUARE non-`all_zero` residual is the proven case. A NON-square (rectangular)
/// residual is admitted only when it is DC-only (`eob == 1`): the §7.15.4 outer
/// process ([`inverse_transform_2d_outer`]) already drives the rectangular path
/// (the §7.15.4.1 `Adjusted_Tx_Size` per-side `Min(log2, 5)` cap, the
/// `Abs(log2W - log2H)` odd-ratio `Round2(x * 2896, 12)` √2 rescale, and the
/// nearest-neighbour duplication for any original side over 32), and a single DC
/// coefficient is shared by every DCT-family transform, so the (unretained) luma tx
/// type is irrelevant — proven bit-exact for the ac0ej3 `TX_16X64` `DC_PRED` leaf at
/// MI(4,0) (x[16,32), y[0,64)) against the AVM pre-filter oracle. A non-square
/// `eob > 1` block may carry a non-`DCT_DCT` luma tx type that `LumaCoeffBlock` does
/// not retain (the primitive always reconstructs `DCT_DCT`), so it stays deferred.
/// The §5.20.7.29 IST / FSC / quant-delta / incomplete-neighbour gates stay intact;
/// everything else is deferred.
fn residual_is_reconstructable(block: &LumaCoeffBlock, fsc_mode: bool, square: bool) -> bool {
    if block.all_zero {
        return true;
    }
    if block.intra_ist.is_some() || fsc_mode {
        return false;
    }
    // Square non-`all_zero` is the proven path; a non-square residual is only
    // DC-only-safe (`eob == 1`), where the unretained luma tx type cannot matter.
    square || block.eob == 1
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_core::span::ByteOffset;

    /// §3 `TxSize` index for TX_16X16 (`Tx_Width[2] == Tx_Height[2] == 16`).
    const TX_16X16: usize = 2;
    /// §3 `TxSize` index for TX_16X64 (`Tx_Width[17] == 16`, `Tx_Height[17] == 64`):
    /// a NON-SQUARE transform.
    const TX_16X64: usize = 17;

    /// An `all_zero` (`txb_skip`) DC block: reconstruction writes the bare §7.13.2
    /// DC prediction (zero residual).
    fn zero_block() -> LumaCoeffBlock {
        LumaCoeffBlock {
            all_zero: true,
            eob: 0,
            quant: Vec::new(),
            intra_ist: None,
        }
    }

    /// A non-`all_zero` block with a single decoded coefficient and `quant` sized
    /// for a 16x16 adjusted transform (256 entries), used to exercise the non-skip
    /// reconstruction path and its gates.
    fn coeff_block_16x16() -> LumaCoeffBlock {
        let mut quant = vec![0i32; 256];
        quant[0] = -355;
        LumaCoeffBlock {
            all_zero: false,
            eob: 1,
            quant,
            intra_ist: None,
        }
    }

    fn sink() -> WienerNsLrReconSink<u16> {
        // 64x64 luma frame (a positive multiple of 64), 10-bit 4:2:0 — matching
        // the ac0ej3 sample type. `quant_reconstructable = true` (no delta-q / qm).
        // `enable_ibp = false` keeps these flat-DC gate tests on the §7.13.2.10
        // prediction; the §7.13.2.12 IBP DC path has its own focused test.
        WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, true, false).unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    fn recon_luma(
        sink: &mut WienerNsLrReconSink<u16>,
        mi_col: usize,
        mi_row: usize,
        tx_size: usize,
        block: &LumaCoeffBlock,
        mode: Option<IntraYMode>,
        fsc_mode: bool,
    ) {
        sink.reconstruct_luma_transform(
            mi_col,
            mi_row,
            tx_size,
            block,
            mode,
            None,
            0,
            149,
            true,
            fsc_mode,
            false,
            ByteOffset::new(0),
        )
        .unwrap();
    }

    /// Drives a CARDINAL (`H_PRED` / `V_PRED`) directional luma transform through
    /// the sink: `leaf_y_mode` is the directional mode, `directional` the resolved
    /// cardinal predictor, and `mrl_index` the §5.20.5.5 multi-reference-line index
    /// (`0` for the immediate edge the cardinal primitive reads).
    #[allow(clippy::too_many_arguments)]
    fn recon_luma_cardinal(
        sink: &mut WienerNsLrReconSink<u16>,
        mi_col: usize,
        mi_row: usize,
        tx_size: usize,
        block: &LumaCoeffBlock,
        mode: IntraYMode,
        directional: SupportedDirectionalLumaMode,
        mrl_index: u8,
    ) {
        sink.reconstruct_luma_transform(
            mi_col,
            mi_row,
            tx_size,
            block,
            Some(mode),
            Some(directional),
            mrl_index,
            149,
            true,
            false,
            false,
            ByteOffset::new(0),
        )
        .unwrap();
    }

    #[test]
    fn dc_all_zero_top_left_writes_the_10bit_no_neighbour_fallback() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        // §7.13.2.1 no-neighbour DC fallback for 10-bit is `1 << (10 - 1)` == 512.
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 512);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 15).unwrap(), 512);
        let (luma4x4, _chroma4x4) = sink.reconstructed_counts();
        // TX_16X16 == 4x4 luma 4x4 units.
        assert_eq!(luma4x4, 16);
    }

    #[test]
    fn non_dc_luma_mode_leaves_the_region_unreconstructed() {
        let mut sink = sink();
        // A leaf without a DC_PRED luma mode (here `None`, an SDP chroma / inter
        // leaf) is deferred: only DC_PRED luma is in the verified subset.
        recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
        // The default 10-bit workspace fill is 0 (not the DC fallback): the sink
        // did not write the non-DC block, so the region stays at the fill value.
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // SMOOTH chroma is DEFERRED, never reconstructed. This is load-bearing for the
    // ac0ej3 mission: splot resolves the SB0 chroma leaf (and every reachable chroma
    // leaf past the first BLOCK_16X64 luma column) as `SMOOTH`, but AVM's mode oracle
    // resolves them as `DC` / `H` / `CfL` and its prediction-only buffer is flat 512
    // (no-neighbour DC). The §7.13.2.13 SMOOTH primitive over the §7.13.2.1
    // no-neighbour fallback edges (above 511, left 513) instead produces a 511..513
    // gradient, so admitting SMOOTH here would write confidently-wrong samples. The
    // sink DEFERS until the upstream mode resolution is reconciled with AVM.
    #[test]
    fn dc_chroma_non_dc_mode_leaves_the_region_unreconstructed() {
        let mut sink = sink();
        // SMOOTH chroma is not in the verified DC subset, so it is deferred.
        sink.reconstruct_chroma_transform(
            PlaneId::U,
            TX_16X16,
            0,
            0,
            &zero_block(),
            Some(SupportedChromaMode::Smooth),
            149,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().1, 0);
        // DC chroma reconstructs the bare DC fallback.
        sink.reconstruct_chroma_transform(
            PlaneId::U,
            TX_16X16,
            0,
            0,
            &zero_block(),
            Some(SupportedChromaMode::Dc),
            149,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 512);
        assert_eq!(sink.reconstructed_counts().1, 16);
    }

    #[test]
    fn second_block_dc_reads_first_block_reconstructed_neighbour() {
        let mut sink = sink();
        // First block at (0,0): no-neighbour DC -> 512.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        // Second block to the right at mi_col=4 (x=16): its DC reads the left
        // neighbour (the reconstructed 512 column), so the flat DC is again 512 —
        // proving the neighbour read path runs over the partially-built frame.
        recon_luma(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 512);
        assert_eq!(sink.reconstructed_counts().0, 32);
    }

    #[test]
    fn out_of_range_tx_size_leaves_the_region_unreconstructed() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            999,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    /// A non-`all_zero` DC block sized for a 16x64 adjusted transform (the
    /// `Min(16,32) x Min(64,32) == 16x32 == 512`-entry coefficient grid), with a
    /// single DC coefficient, used to exercise the non-square residual path.
    fn coeff_block_16x64() -> LumaCoeffBlock {
        let mut quant = vec![0i32; 512];
        quant[0] = -2;
        LumaCoeffBlock {
            all_zero: false,
            eob: 1,
            quant,
            intra_ist: None,
        }
    }

    // A non-`all_zero`, non-square DC leaf (e.g. TX_16X64) is now ADMITTED: the
    // §7.15.4 outer process drives the rectangular-residual inverse transform
    // (proven bit-exact for the ac0ej3 mi(4,0) leaf). The frame-origin no-neighbour
    // DC fallback (512) plus a flat DC-only residual reconstructs a flat block.
    #[test]
    fn non_square_nonzero_dc_leaf_is_reconstructed() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X64,
            &coeff_block_16x64(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        // The origin block has no reconstructed neighbour (off-frame edges), so the
        // §7.13.2.1 DC fallback is 512 (10-bit); the DC-only residual is applied
        // over the whole 16x64 block, so the sink wrote a real (non-fill) region.
        assert!(sink.reconstructed_counts().0 > 0);
        // 16x64 == 4 MI cols x 16 MI rows == 64 4x4 units.
        assert_eq!(sink.reconstructed_counts().0, 64);
    }

    // Finding #1: a non-`all_zero` DC leaf carrying §5.20.7.29 IST secondary
    // transform syntax is DEFERRED (the primitive is DCT_DCT-only).
    #[test]
    fn ist_nonzero_dc_leaf_is_deferred() {
        let mut sink = sink();
        let mut block = coeff_block_16x16();
        block.intra_ist = Some(crate::tile_payload::IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(0),
        });
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &block,
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // Codex review: a non-`all_zero`, NON-square DC leaf with `eob > 1` may carry a
    // non-`DCT_DCT` luma tx type (not retained in `LumaCoeffBlock`); the primitive
    // always reconstructs `DCT_DCT`, so it is DEFERRED. Only a DC-only (`eob == 1`)
    // non-square residual is tx-type-agnostic and admitted (the ac0ej3 mi(4,0) case).
    #[test]
    fn non_square_multi_coeff_dc_leaf_is_deferred() {
        let mut sink = sink();
        let mut block = coeff_block_16x64();
        block.eob = 2;
        block.quant[1] = 7;
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X64,
            &block,
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // Finding #1: an FSC DC leaf is DEFERRED (non-FSC primitive).
    #[test]
    fn fsc_nonzero_dc_leaf_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &coeff_block_16x16(),
            Some(IntraYMode::DC_PRED),
            true,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // Finding #2: a DC block bordering a DEFERRED (skipped) neighbour is deferred —
    // its DC prediction would read the workspace fill value, not reconstruction.
    #[test]
    fn dc_block_with_deferred_neighbour_is_deferred() {
        let mut sink = sink();
        // Block at (0,0) is deferred (non-DC leaf -> `None`). It is NOT reconstructed.
        recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
        assert_eq!(sink.reconstructed_counts().0, 0);
        // Block at (4,0) is DC_PRED but its LEFT neighbour (0,0) exists on-grid and
        // was deferred, so this block defers too (no wrong prediction from fill).
        recon_luma(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // Finding #2 (re-review): U and V chroma coverage are tracked SEPARATELY. A
    // reconstructed U block must not let a deferred-neighbour V block pass the
    // DC-edge guard (4:2:0 U and V share MI dimensions but not reconstruction
    // state); otherwise V would predict from its own workspace fill value.
    #[test]
    fn chroma_u_coverage_does_not_satisfy_v_edge_guard() {
        let mut sink = sink();
        // A U DC block at the chroma origin reconstructs (off-grid edges) and marks
        // U coverage across MI columns 0..4.
        sink.reconstruct_chroma_transform(
            PlaneId::U,
            TX_16X16,
            0,
            0,
            &zero_block(),
            Some(SupportedChromaMode::Dc),
            149,
            ByteOffset::new(0),
        )
        .unwrap();
        let chroma_after_u = sink.reconstructed_counts().1;
        assert!(chroma_after_u > 0, "U origin block should reconstruct");
        // A V DC block whose left neighbour (MI column 3) is covered ONLY on the U
        // plane must DEFER — it cannot borrow U's coverage to satisfy its own guard.
        sink.reconstruct_chroma_transform(
            PlaneId::V,
            TX_16X16,
            16,
            0,
            &zero_block(),
            Some(SupportedChromaMode::Dc),
            149,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(
            sink.reconstructed_counts().1,
            chroma_after_u,
            "deferred-neighbour V block must not reconstruct via U's coverage",
        );
    }

    // Finding #3: when the frame signals a non-zero quantizer delta / qmatrix
    // (`quant_reconstructable == false`), the sink reconstructs NOTHING.
    #[test]
    fn non_reconstructable_quant_defers_everything() {
        let mut sink =
            WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, false, false).unwrap();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        sink.reconstruct_chroma_transform(
            PlaneId::U,
            TX_16X16,
            0,
            0,
            &zero_block(),
            Some(SupportedChromaMode::Dc),
            149,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_sample(PlaneId::U, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts(), (0, 0));
    }

    // Cardinal H_PRED (§7.13.2.8 step 5, pAngle 180) over a REAL reconstructed left
    // column: a pure horizontal copy `pred[i][j] = LeftCol[i]`. The first DC block
    // at (0,0) reconstructs the flat `512` no-neighbour fallback; an `all_zero`
    // H_PRED block to its right copies that left column, so it is again flat `512` —
    // proving the cardinal copy reads the partially-built frame's real neighbour.
    #[test]
    fn cardinal_hpred_copies_reconstructed_left_column() {
        let mut sink = sink();
        // Left DC neighbour at (0,0): flat 512.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        // H_PRED block to the right at mi_col=4 (x=16): its left column (x=15) is the
        // reconstructed 512, so the horizontal copy is flat 512.
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
            0,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 512);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 31, 15).unwrap(), 512);
        // 16 (left DC) + 16 (H_PRED) == 32 4x4 units.
        assert_eq!(sink.reconstructed_counts().0, 32);
    }

    // Cardinal V_PRED (§7.13.2.8 step 4, pAngle 90) over a REAL reconstructed above
    // row: a pure vertical copy `pred[i][j] = AboveRow[j]`. The first DC block at
    // (0,0) reconstructs flat `512`; a V_PRED block below it copies that above row,
    // flat `512` — proving the cardinal copy reads the real above neighbour.
    #[test]
    fn cardinal_vpred_copies_reconstructed_above_row() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        // V_PRED block below at mi_row=4 (y=16): its above row (y=15) is the
        // reconstructed 512, so the vertical copy is flat 512.
        recon_luma_cardinal(
            &mut sink,
            0,
            4,
            TX_16X16,
            &zero_block(),
            IntraYMode::V_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Vertical,
            0,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 16).unwrap(), 512);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 31).unwrap(), 512);
        assert_eq!(sink.reconstructed_counts().0, 32);
    }

    // A cardinal block at the frame ORIGIN has no required edge to copy — H_PRED
    // has no left column (mi_col == 0), V_PRED has no above row (mi_row == 0) — so
    // the sink DEFERS both (the §7.13.2.1 no-neighbour fallback is a separate,
    // here-unmodelled path; never predict from the fill value). The V_PRED case
    // pins the exact ac0ej3 SB-column-4 V_PRED-at-y=0 deferral.
    #[test]
    fn cardinal_at_frame_edge_with_no_required_neighbour_is_deferred() {
        for (mode, direction) in [
            (
                IntraYMode::H_PRED_FOR_TEST,
                SupportedDirectionalLumaMode::Horizontal,
            ),
            (
                IntraYMode::V_PRED_FOR_TEST,
                SupportedDirectionalLumaMode::Vertical,
            ),
        ] {
            let mut sink = sink();
            recon_luma_cardinal(&mut sink, 0, 0, TX_16X16, &zero_block(), mode, direction, 0);
            assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
            assert_eq!(sink.reconstructed_counts().0, 0);
        }
    }

    // A cardinal H_PRED block whose LEFT neighbour exists on-grid but was DEFERRED
    // (still the fill value) must defer too — never copy a fill-value left column.
    #[test]
    fn cardinal_hpred_with_deferred_left_neighbour_is_deferred() {
        let mut sink = sink();
        // (0,0) is deferred (non-DC `None` leaf), so it is NOT reconstructed.
        recon_luma(&mut sink, 0, 0, TX_16X16, &zero_block(), None, false);
        assert_eq!(sink.reconstructed_counts().0, 0);
        // H_PRED at mi_col=4: its left neighbour (0,0) is on-grid but uncovered.
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
            0,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    // A NON-SQUARE cardinal transform is DEFERRED: the cardinal recon primitive is
    // square-only. (TX_16X64 H_PRED with a covered left column still defers.)
    #[test]
    fn cardinal_nonsquare_transform_is_deferred() {
        let mut sink = sink();
        // Reconstruct a left DC neighbour column first so coverage is not the gate.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X64,
            &zero_block(),
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
            0,
        );
        // Non-square cardinal deferred: count unchanged, region stays fill.
        assert_eq!(sink.reconstructed_counts().0, before);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    }

    // An ANGULAR directional mode (e.g. D135) is DEFERRED by the sink even when its
    // neighbours are covered: only the cardinal V/H copy subset is admitted here.
    #[test]
    fn angular_directional_mode_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            IntraYMode::D135_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::D135,
            0,
        );
        assert_eq!(sink.reconstructed_counts().0, before);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    }

    // Finding 1: a cardinal H_PRED leaf using a §5.20.5.5 multi-reference line
    // (`mrl_index > 0`) is DEFERRED. The cardinal recon primitive copies the
    // IMMEDIATE left/above edge (`MrlIndex == 0`); for `mrl_index > 0` it would
    // copy the adjacent samples instead of the selected reference line — wrong.
    // The left neighbour is covered, so only the MRL gate causes the deferral.
    #[test]
    fn cardinal_with_active_mrl_index_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        // H_PRED at mi_col=4 with a covered left column but `mrl_index == 1`: defer.
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X16,
            &zero_block(),
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
            1,
        );
        assert_eq!(sink.reconstructed_counts().0, before);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    }

    // Finding 2: a cardinal H_PRED leaf with a NON-`all_zero`, non-DC-only residual
    // (`eob > 1`) is DEFERRED. Such a block may carry a non-`DCT_DCT` luma tx type
    // that `LumaCoeffBlock` does not retain; the cardinal primitive always
    // inverse-transforms `DCT_DCT`, so it would write wrong residual samples. Only a
    // DC-only (`eob == 1`) residual is tx-type-agnostic and admitted. The left
    // neighbour is covered, so only the residual gate causes the deferral.
    #[test]
    fn cardinal_with_multi_coeff_residual_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        // A non-`all_zero` block with two decoded coefficients (`eob == 2`).
        let mut block = coeff_block_16x16();
        block.eob = 2;
        block.quant[1] = 7;
        recon_luma_cardinal(
            &mut sink,
            4,
            0,
            TX_16X16,
            &block,
            IntraYMode::H_PRED_FOR_TEST,
            SupportedDirectionalLumaMode::Horizontal,
            0,
        );
        assert_eq!(sink.reconstructed_counts().0, before);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 0);
    }

    /// A §7.13.3.18 IntrABC integer-vector skip copy whose source rectangle is fully
    /// reconstructed copies the source samples into the target (target == source).
    #[test]
    fn intrabc_integer_skip_copy_reconstructs_target_from_reconstructed_source() {
        let mut sink = sink();
        // Reconstruct a DC source block at the origin (16x16, flat 512), then copy a
        // 16x16 region of it down to a non-overlapping target.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        let source = PlaneRect::new(0, 0, 16, 16).unwrap();
        let target = PlaneRect::new(0, 32, 16, 16).unwrap();
        sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
            .unwrap();
        // The whole 16x16 target now carries the copied source samples (flat 512).
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 512);
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 15, 47).unwrap(), 512);
        // 16x16 == 16 4x4 luma units added.
        assert_eq!(sink.reconstructed_counts().0, before + 16);
    }

    /// A §7.13.3.18 IntrABC block whose source rectangle is NOT fully reconstructed
    /// is DEFERRED — never copies a workspace fill value as if it were a real sample.
    #[test]
    fn intrabc_copy_with_unreconstructed_source_is_deferred() {
        let mut sink = sink();
        // No block reconstructed yet: the source region (0,0,16,16) is all fill.
        let source = PlaneRect::new(0, 0, 16, 16).unwrap();
        let target = PlaneRect::new(0, 32, 16, 16).unwrap();
        sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
            .unwrap();
        // The target stays at the unreconstructed fill value, and nothing is counted.
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }

    /// Codex finding 1: a NON-4x4-aligned integer source whose CEIL'd MI span includes
    /// an unreconstructed trailing MI is DEFERRED. The covered-MI span must be computed
    /// from the source's actual sample extent (`ceil((x+width)/4) - floor(x/4)`), not a
    /// floored `width / 4` that would drop the trailing partial MI and copy its fill.
    #[test]
    fn intrabc_unaligned_source_with_uncovered_trailing_mi_is_deferred() {
        let mut sink = sink();
        // Reconstruct a 16x16 DC block at the origin: covers luma x[0,16) == MI cols
        // 0..4. MI col 4 (x[16,20)) stays unreconstructed.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        // A 16px source at x=2 spans x[2,18) == MI cols 0..=4 (ceil(18/4)==5): MI col 4
        // is uncovered, so the copy must DEFER. (A floored `16/4==4` span would wrongly
        // see only cols 0..4 and copy the trailing fill.)
        let source = PlaneRect::new(2, 0, 16, 16).unwrap();
        let target = PlaneRect::new(2, 32, 16, 16).unwrap();
        sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
            .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 2, 32).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, before);
    }

    /// A non-skip IntrABC block (non-zero residual) is DEFERRED even with a fully
    /// reconstructed integer-vector source — its residual is not yet proven bit-exact.
    #[test]
    fn intrabc_non_skip_block_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        let source = PlaneRect::new(0, 0, 16, 16).unwrap();
        let target = PlaneRect::new(0, 32, 16, 16).unwrap();
        sink.reconstruct_intrabc_block(source, target, false, ByteOffset::new(0))
            .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, before);
    }

    /// A fractional-vector IntrABC block (source and target differ in shape — the
    /// BILINEAR border) is DEFERRED: the copy primitive only models the integer copy.
    #[test]
    fn intrabc_fractional_vector_block_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        let before = sink.reconstructed_counts().0;
        // A fractional vector widens the source by a one-sample BILINEAR border, so
        // source.size() != target.size().
        let source = PlaneRect::new(0, 0, 17, 17).unwrap();
        let target = PlaneRect::new(0, 32, 16, 16).unwrap();
        sink.reconstruct_intrabc_block(source, target, true, ByteOffset::new(0))
            .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 32).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, before);
    }
}
