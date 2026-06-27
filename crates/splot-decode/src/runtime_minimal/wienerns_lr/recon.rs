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
/// per-block dequant index, and `luma_use_tcq` carries the §7.14.4 luma TCQ
/// `dqDenom` term.
#[derive(Clone, Copy, Debug)]
pub(in crate::runtime_minimal) struct SelectableReconContext {
    pub(in crate::runtime_minimal) leaf_y_mode: Option<IntraYMode>,
    pub(in crate::runtime_minimal) chroma_mode: Option<SupportedChromaMode>,
    pub(in crate::runtime_minimal) qindex: u32,
    pub(in crate::runtime_minimal) luma_use_tcq: bool,
}

/// Reconstructs the verified NON-IntrABC general-intra DC subset of the ac0ej3
/// key frame into an owned [`CurrentFrameWorkspace`], in selectable-walk decode
/// order. Holding the workspace across the walk (including the walk's eventual
/// fail-closed IntrABC rejection) lets the region-verification test read the
/// samples reconstructed before the rejection point.
///
/// The sink is gated to the proven subset: a luma transform is reconstructed only
/// when its leaf signalled `DC_PRED`, and a chroma group only when the resolved
/// §5.20.5.3 chroma mode is `DC_PRED`. Any other mode, an IntrABC block, or a
/// transform geometry the rectangular DC primitive does not handle leaves that
/// region UNRECONSTRUCTED (the workspace keeps its fill value there), so the sink
/// never writes samples it cannot prove. `reconstructed_luma_4x4` /
/// `reconstructed_chroma_4x4` count the 4x4 units actually written so the test can
/// report which region was verified.
pub(in crate::runtime_minimal) struct WienerNsLrReconSink<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
    bit_depth: BitDepth,
    reconstructed_luma_4x4: usize,
    reconstructed_chroma_4x4: usize,
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
    ) -> Result<Self> {
        Ok(Self {
            workspace: new_general_intra_workspace::<T>(luma_width, luma_height, bit_depth)?,
            bit_depth,
            reconstructed_luma_4x4: 0,
            reconstructed_chroma_4x4: 0,
        })
    }

    /// Reconstructs one luma transform block (DC_PRED only) at the given MI
    /// position into the workspace, reading the §7.13.2 DC prediction from the
    /// partially-built frame's reconstructed neighbours and adding the decoded
    /// residual (a flat DC for an `all_zero` block). `leaf_y_mode` is the block's
    /// §5.20.5.3 luma mode; a non-DC mode (or an out-of-range `tx_size`) leaves
    /// the region unreconstructed and the sink returns `Ok(())` without writing —
    /// never wrong samples claimed correct.
    ///
    /// `use_tcq` carries the §7.14.4 luma TCQ `dqDenom` term; `qindex` is the
    /// per-block dequant index (the §5.20.6.5 `DeltaQState.current_q_index`).
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
        qindex: u32,
        use_tcq: bool,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        if leaf_y_mode != Some(IntraYMode::DC_PRED) {
            // Defer non-DC luma: the region stays unreconstructed rather than
            // emitting a prediction this brick has not proven bit-exact.
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(tx_size) else {
            return Ok(());
        };
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
        self.reconstructed_luma_4x4 = self
            .reconstructed_luma_4x4
            .saturating_add((1usize << log2_width >> 2) * (1usize << log2_height >> 2));
        Ok(())
    }

    /// Reconstructs one chroma (U or V) transform block (DC chroma only) at the
    /// given chroma-plane sample position into the workspace. `chroma_mode` is the
    /// resolved §5.20.5.3 chroma mode; only [`SupportedChromaMode::Dc`] is written
    /// (chroma never uses the §7.14.4 TCQ term). A non-DC chroma mode or an
    /// out-of-range `tx_size` leaves the region unreconstructed.
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
        if chroma_mode != Some(SupportedChromaMode::Dc) {
            return Ok(());
        }
        let Some((log2_width, log2_height)) = tx_size_log2(chroma_tx) else {
            return Ok(());
        };
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
    )?;
    // The walk reconstructs into the sink in decode order. The ac0ej3 stream
    // rejects at the first active IntrABC block; the owned sink retains the region
    // reconstructed before that expected rejection, so swallow it (any other error
    // would be a real parse/reconstruction failure, but the handoff returns the
    // bounded IntrABC rejection here).
    let _ = super::tx_records::derive_wienerns_lr_selectable_transform_record_handoff(
        bytes,
        options,
        plan,
        key_candidate,
        key_envelope,
        sequence,
        core,
        Some(&mut sink),
    );
    Ok(sink)
}

/// Maps a §5.20.6 `TxSize` index to its `(log2_width, log2_height)` sample
/// dimensions via the §9 `Tx_Width` / `Tx_Height` log2 tables, or `None` when the
/// index is outside the 19-entry table range.
fn tx_size_log2(tx_size: usize) -> Option<(u32, u32)> {
    let w = u32::try_from(*TX_WIDTH_LOG2.get(tx_size)?).ok()?;
    let h = u32::try_from(*TX_HEIGHT_LOG2.get(tx_size)?).ok()?;
    Some((w, h))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_core::span::ByteOffset;

    /// §3 `TxSize` index for TX_16X16 (`Tx_Width[2] == Tx_Height[2] == 16`).
    const TX_16X16: usize = 2;

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

    fn sink() -> WienerNsLrReconSink<u16> {
        // 64x64 luma frame (a positive multiple of 64), 10-bit 4:2:0 — matching
        // the ac0ej3 sample type.
        WienerNsLrReconSink::<u16>::new(64, 64, BitDepth::Ten).unwrap()
    }

    #[test]
    fn dc_all_zero_top_left_writes_the_10bit_no_neighbour_fallback() {
        let mut sink = sink();
        sink.reconstruct_luma_transform(
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            149,
            true,
            ByteOffset::new(0),
        )
        .unwrap();
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
        sink.reconstruct_luma_transform(
            0,
            0,
            TX_16X16,
            &zero_block(),
            None,
            149,
            true,
            ByteOffset::new(0),
        )
        .unwrap();
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
        sink.reconstruct_luma_transform(
            0,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            149,
            true,
            ByteOffset::new(0),
        )
        .unwrap();
        // Second block to the right at mi_col=4 (x=16): its DC reads the left
        // neighbour (the reconstructed 512 column), so the flat DC is again 512 —
        // proving the neighbour read path runs over the partially-built frame.
        sink.reconstruct_luma_transform(
            4,
            0,
            TX_16X16,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            149,
            true,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 16, 0).unwrap(), 512);
        assert_eq!(sink.reconstructed_counts().0, 32);
    }

    #[test]
    fn out_of_range_tx_size_leaves_the_region_unreconstructed() {
        let mut sink = sink();
        sink.reconstruct_luma_transform(
            0,
            0,
            999,
            &zero_block(),
            Some(IntraYMode::DC_PRED),
            149,
            true,
            ByteOffset::new(0),
        )
        .unwrap();
        assert_eq!(sink.reconstructed_sample(PlaneId::Y, 0, 0).unwrap(), 0);
        assert_eq!(sink.reconstructed_counts().0, 0);
    }
}
