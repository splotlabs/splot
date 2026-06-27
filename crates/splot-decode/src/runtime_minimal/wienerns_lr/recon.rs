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
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::Result;
#[cfg(test)]
use crate::runtime_minimal_recon::new_general_intra_workspace;
use crate::runtime_minimal_recon::reconstruct_general_intra_block_rect_into;
use crate::tile_payload::{IntraYMode, LumaCoeffBlock, SupportedChromaMode};

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
    pub(in crate::runtime_minimal) chroma_mode: Option<SupportedChromaMode>,
    pub(in crate::runtime_minimal) qindex: u32,
    pub(in crate::runtime_minimal) luma_use_tcq: bool,
    pub(in crate::runtime_minimal) fsc_mode: bool,
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
    /// Per-plane MI-unit coverage (`coverage[plane]`, row-major over the plane's MI
    /// grid). Luma uses plane 0; both chroma planes share plane 1 (4:2:0, same MI
    /// grid). `true` where the sink has written spec-correct samples.
    coverage: [PlaneCoverage; 2],
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
    ) -> Result<Self> {
        // 4:2:0 chroma planes are half the luma dimensions in each axis.
        let chroma_width = luma_width.div_ceil(2);
        let chroma_height = luma_height.div_ceil(2);
        Ok(Self {
            workspace: new_general_intra_workspace::<T>(luma_width, luma_height, bit_depth)?,
            bit_depth,
            quant_reconstructable,
            coverage: [
                PlaneCoverage::new(luma_width, luma_height),
                PlaneCoverage::new(chroma_width, chroma_height),
            ],
            reconstructed_luma_4x4: 0,
            reconstructed_chroma_4x4: 0,
        })
    }

    /// The coverage-grid index for a plane: luma is grid 0, both chroma planes
    /// share grid 1 (4:2:0 U and V occupy the same MI grid).
    const fn coverage_index(plane_id: PlaneId) -> usize {
        match plane_id {
            PlaneId::Y => 0,
            PlaneId::U | PlaneId::V => 1,
        }
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

    /// Reconstructs one luma transform block at the given MI position into the
    /// workspace, reading the §7.13.2 DC prediction from the partially-built
    /// frame's reconstructed neighbours and adding the decoded residual (a flat DC
    /// for an `all_zero` block). The block is DEFERRED (returns `Ok(())` without
    /// writing — never wrong samples claimed correct) unless ALL of the proven
    /// subset holds:
    /// * `leaf_y_mode` is `DC_PRED`;
    /// * the frame dequant matches the primitive's zero-`QuantizerDeltas`
    ///   assumption (`quant_reconstructable`);
    /// * the residual is the proven primitive kind ([`residual_is_reconstructable`]:
    ///   an `all_zero` flat-DC block, or a square non-`all_zero` block with no IST
    ///   and no FSC);
    /// * the §7.13.2 DC-prediction edges are off-frame or already reconstructed by
    ///   this sink ([`Self::dc_edges_reconstructed`]).
    ///
    /// `use_tcq` carries the §7.14.4 luma TCQ `dqDenom` term; `qindex` is the
    /// per-block dequant index (the §5.20.6.5 `DeltaQState.current_q_index`);
    /// `fsc_mode` is the leaf's FSC flag. `mi_col` / `mi_row` are the transform's §3
    /// MI coordinates and `tx_size` its §5.20.6 `TxSize` index.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime_minimal) fn reconstruct_luma_transform(
        &mut self,
        mi_col: usize,
        mi_row: usize,
        tx_size: usize,
        block: &LumaCoeffBlock,
        leaf_y_mode: Option<IntraYMode>,
        qindex: u32,
        use_tcq: bool,
        fsc_mode: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if leaf_y_mode != Some(IntraYMode::DC_PRED) || !self.quant_reconstructable {
            // Defer non-DC luma or a frame whose dequant the primitive cannot honor:
            // leave the region unreconstructed rather than emitting a prediction this
            // brick has not proven bit-exact.
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return Ok(());
        };
        if !residual_is_reconstructable(block, fsc_mode, log2_width == log2_height) {
            return Ok(());
        }
        let (mi_w, mi_h) = mi_extent(log2_width, log2_height);
        if !self.dc_edges_reconstructed(PlaneId::Y, mi_col, mi_row, mi_w, mi_h) {
            // A DC-prediction edge neighbour exists on-grid but was deferred; its
            // workspace samples are the fill value, not reconstruction, so the DC
            // prediction would be wrong. Defer this block too.
            return Ok(());
        }
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
            self.bit_depth,
        )
        .map_err(|_| {
            wienerns_lr_selectable_transform_record_error_reason(
                tile_offset,
                "unsupported_wienerns_lr_selectable_transform_records_recon_luma_write",
            )
        })?;
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
    let bit_depth = BitDepth::from_av2_bit_depth_idc(sequence.general.bit_depth_idc.get())
        .map_err(|source| crate::error::DecodeError::Reconstruction { source })?;
    let mut sink = WienerNsLrReconSink::<u16>::new(
        frame_size.width as usize,
        frame_size.height as usize,
        bit_depth,
        frame_quant_reconstructable(core),
    )?;
    // The walk reconstructs into the sink in decode order. The ac0ej3 stream
    // rejects at the first active IntrABC block; the owned sink retains the region
    // reconstructed before that expected rejection. Swallow ONLY that known
    // IntrABC-currframe-samples rejection — any other error (an earlier parse or
    // reconstruction failure, e.g. a regression that fails before the IntrABC
    // frontier after the verified region is written) is propagated so the test
    // fails loudly instead of silently passing on a partial walk.
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
            if unsupported.reason() == EXPECTED_INTRABC_STOP_REASON =>
        {
            Ok(sink)
        }
        Err(other) => Err(other),
    }
}

/// The single §7.13.3.18 IntrABC fail-closed reason the ac0ej3 selectable walk is
/// expected to stop on after reconstructing the verified region; the test driver
/// swallows only this one and propagates every other error.
#[cfg(test)]
const EXPECTED_INTRABC_STOP_REASON: &str =
    "unsupported_wienerns_lr_selectable_transform_records_intrabc_currframe_samples";

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
/// The primitive assumes the §5.20.7.29 `DCT_DCT` no-secondary-transform path and
/// dequantizes with zero `QuantizerDeltas`. An `all_zero` (`txb_skip`) block is
/// always safe: there is no residual, so the output is the bare §7.13.2 flat DC
/// prediction. A non-`all_zero` block is admitted only when it has no IST secondary
/// transform (`intra_ist`), is not an FSC leaf, and is SQUARE — the
/// rectangular-residual inverse transform for non-square transforms is not yet
/// proven bit-exact against AVM (the ac0ej3 `TX_16X64` `DC_PRED` leaf reconstructs
/// with a wrong AC residual), so it is deferred. Anything else is deferred.
fn residual_is_reconstructable(block: &LumaCoeffBlock, fsc_mode: bool, square: bool) -> bool {
    if block.all_zero {
        return true;
    }
    block.intra_ist.is_none() && !fsc_mode && square
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
        WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, true).unwrap()
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
            149,
            true,
            fsc_mode,
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

    // Finding #1: a non-`all_zero`, non-square DC leaf (e.g. TX_16X64) is DEFERRED
    // — the rectangular-residual inverse transform is not yet proven bit-exact.
    #[test]
    fn non_square_nonzero_dc_leaf_is_deferred() {
        let mut sink = sink();
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X64,
            &coeff_block_16x16(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
        // The SAME non-square geometry IS reconstructed when `all_zero` (flat DC,
        // no residual): the gate defers only the unproven non-square residual.
        recon_luma(
            &mut sink,
            0,
            0,
            TX_16X64,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            false,
        );
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 512);
        assert!(sink.reconstructed_counts().0 > 0);
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

    // Finding #3: when the frame signals a non-zero quantizer delta / qmatrix
    // (`quant_reconstructable == false`), the sink reconstructs NOTHING.
    #[test]
    fn non_reconstructable_quant_defers_everything() {
        let mut sink = WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten, false).unwrap();
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
}
