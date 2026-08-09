// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Owned inter reconstruction commands and consumer-local scratch.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_recon::{BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, PlaneId, ReconSample};

use super::super::compound::CompoundBlockSyntax;
use super::super::find_mv_stack::{TemporalMotionBlock, TemporalMvContext};
use super::super::mc::{self, WorkspaceSink};
use super::super::{
    InterReferenceState, InterResidual, InterResidualBlock, InterResidualReconScratch,
    PlacedInterBlock,
};
use super::compound_path::append_compound_temporal_motion;
use super::temporal::{MotionFieldUnits, temporal_motion_block};
use super::tip::{self, TipReconstructScratch};
use crate::Result;
use crate::bitstream::tile_payload::{FrameQmSegmentScope, TileBlockDecodedState};

#[derive(Clone, Copy, Debug)]
pub(super) enum PendingKind {
    Single,
    Compound {
        syntax: CompoundBlockSyntax,
        warp_params: [Option<[i32; 6]>; 2],
        mi_row: usize,
        mi_col: usize,
        use_refinemv: bool,
        refinemv_switchable: bool,
    },
    Tip,
}

#[derive(Debug)]
pub(super) struct InterReconCommand {
    placed: PlacedInterBlock,
    kind: PendingKind,
    segment_id: usize,
    qindex: u32,
    tile_offset: ByteOffset,
}

#[derive(Default)]
#[repr(align(64))]
pub(super) struct InterReconScratch<T: ReconSample> {
    general_intra: crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    tip: TipReconstructScratch<T>,
    temporal: Vec<TemporalMotionBlock>,
    interintra: super::interintra::InterIntraScratch<T>,
    residual: InterResidualReconScratch<T>,
    mc: super::super::mc::McScratch,
}

/// The frame-level facts every inter reconstruction reads, gathered once so the
/// motion and prediction halves take one argument instead of eight.
pub(super) struct ReconShared<'a, T: ReconSample> {
    pub(super) reference: &'a InterReferenceState<T>,
    pub(super) ref_frame_idx: &'a [u32],
    pub(super) temporal_context: &'a TemporalMvContext,
    pub(super) sequence: &'a SequenceHeader,
    pub(super) core: &'a FrameHeaderCore,
    pub(super) luma_use_tcq: bool,
    pub(super) residual_use_ddt: bool,
    pub(super) bit_depth: BitDepth,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    pub(super) current_order_hint: u32,
}

pub(super) const fn reads_current_frame(bawp: bool, interintra: bool) -> bool {
    bawp || interintra
}

impl InterReconCommand {
    /// `segment_id` is the § 7.14 quantizer-matrix segment in force while the
    /// leaf was parsed, captured there because the resolve pass runs after the
    /// leaf's segment scope has been dropped.
    pub(super) const fn new(
        placed: PlacedInterBlock,
        kind: PendingKind,
        segment_id: usize,
        qindex: u32,
        tile_offset: ByteOffset,
    ) -> Self {
        Self {
            placed,
            kind,
            segment_id,
            qindex,
            tile_offset,
        }
    }

    /// The placed block this command reconstructs.
    pub(super) const fn placed(&self) -> &PlacedInterBlock {
        &self.placed
    }

    /// Whether § 7.13.5 TIP synthesis reconstructs this command, which reads
    /// its reference frames through the TIP motion field rather than through
    /// the block's own motion vectors.
    pub(super) const fn is_tip(&self) -> bool {
        matches!(self.kind, PendingKind::Tip)
    }

    pub(super) fn reads_current_frame(&self) -> bool {
        reads_current_frame(
            self.placed.block.bawp.enabled,
            self.placed.block.interintra.is_some(),
        )
    }

    pub(super) fn temporal_record_capacity(&self) -> usize {
        match self.kind {
            PendingKind::Single => 1,
            PendingKind::Compound { .. } | PendingKind::Tip => self
                .placed
                .luma_w
                .div_ceil(8)
                .saturating_mul(self.placed.luma_h.div_ceil(8)),
        }
    }

    pub(super) fn prepass_write_is_contained(
        &self,
        superblock_origin: [usize; 2],
        sb_h4: usize,
        info: DecodedFrameInfo,
        residual_blocks: &[InterResidualBlock],
    ) -> bool {
        prepass_write_is_contained(
            [
                self.placed.luma_x,
                self.placed.luma_y,
                self.placed.luma_w,
                self.placed.luma_h,
            ],
            [
                self.placed.chroma_luma_x,
                self.placed.chroma_luma_y,
                self.placed.chroma_luma_w,
                self.placed.chroma_luma_h,
            ],
            self.placed.predict_chroma,
            self.reads_current_frame(),
            self.placed.block.residual.as_ref(),
            superblock_origin,
            sb_h4,
            info,
            residual_blocks,
        )
    }

    fn single_temporal_record<T: ReconSample>(
        &self,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
    ) -> TemporalMotionBlock {
        let block = &self.placed.block;
        temporal_motion_block(
            reference,
            ref_frame_idx,
            self.placed.luma_y / 4,
            self.placed.luma_x / 4,
            self.placed.luma_w / 4,
            self.placed.luma_h / 4,
            mi_rows,
            mi_cols,
            current_order_hint,
            block.ref_frame0,
            block.ref_frame1,
            block.mv,
            block.mv1,
            block.warp_params,
        )
    }

    fn refinemv(&self) -> (bool, bool) {
        match self.kind {
            PendingKind::Compound {
                use_refinemv,
                refinemv_switchable,
                ..
            } => (use_refinemv, refinemv_switchable),
            PendingKind::Single | PendingKind::Tip => (false, false),
        }
    }

    /// Derives the block's motion: the refinement grid its prediction samples
    /// through, and its § 7.22 temporal records.
    ///
    /// This reads reference samples and writes none, so it is the half a motion
    /// resolution pass runs ahead of reconstruction; the grid it returns is
    /// what [`Self::reconstruct_from_motion`] predicts from, and no other call
    /// can derive one.
    fn derive_motion<T: ReconSample>(
        &self,
        sink: &WorkspaceSink<'_, '_, T>,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        shared: &ReconShared<'_, T>,
        tip_scratch: &mut TipReconstructScratch<T>,
    ) -> Result<Option<mc::CompoundMotionGrid>> {
        match self.kind {
            PendingKind::Tip => tip::motion(
                tip_scratch,
                temporal_records,
                sink,
                &self.placed,
                shared.temporal_context,
                shared.sequence,
                shared.core,
                shared.ref_frame_idx,
                shared.reference,
                self.tile_offset,
            ),
            PendingKind::Single => {
                temporal_records.push(self.single_temporal_record(
                    shared.reference,
                    shared.ref_frame_idx,
                    shared.mi_rows,
                    shared.mi_cols,
                    shared.current_order_hint,
                ));
                Ok(None)
            }
            PendingKind::Compound {
                syntax,
                warp_params,
                mi_row,
                mi_col,
                use_refinemv,
                refinemv_switchable,
            } => {
                let held = super::super::hold_inter_block_references(
                    shared.ref_frame_idx,
                    shared.reference,
                    &self.placed,
                )?;
                let grid = mc::inter_block_motion_grid(
                    sink,
                    held.block_params(&self.placed, self.placed.motion_compensation_rect())?
                        .with_refinemv(use_refinemv)
                        .with_switchable_refinemv(refinemv_switchable),
                    None,
                    self.tile_offset,
                )?;
                drop(held);
                append_compound_temporal_motion(
                    temporal_records,
                    shared.reference,
                    shared.ref_frame_idx,
                    &self.placed,
                    syntax,
                    warp_params,
                    grid.as_ref(),
                    mi_row,
                    mi_col,
                    shared.mi_rows,
                    shared.mi_cols,
                    shared.current_order_hint,
                )?;
                Ok(grid)
            }
        }
    }

    /// Reconstructs the block's samples from the grid the motion half derived.
    #[allow(clippy::too_many_arguments)]
    fn reconstruct_from_motion<T: ReconSample>(
        &self,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        motion: Option<mc::CompoundMotionGrid>,
        residual_blocks: &[InterResidualBlock],
        shared: &ReconShared<'_, T>,
        tip_scratch: &mut TipReconstructScratch<T>,
        interintra_scratch: &mut super::interintra::InterIntraScratch<T>,
        residual_scratch: &mut InterResidualReconScratch<T>,
    ) -> Result<()> {
        let _segment_scope = FrameQmSegmentScope::install(self.segment_id);
        if matches!(self.kind, PendingKind::Tip) {
            return tip::predict(
                tip_scratch,
                residual_scratch,
                sink,
                matches!(sink, WorkspaceSink::Frame(_)),
                motion,
                &self.placed,
                residual_blocks,
                shared.temporal_context,
                shared.sequence,
                shared.core,
                shared.ref_frame_idx,
                shared.reference,
                self.qindex,
                shared.luma_use_tcq,
                shared.residual_use_ddt,
                shared.bit_depth,
                self.tile_offset,
            );
        }
        let (use_refinemv, refinemv_switchable) = self.refinemv();
        match sink {
            WorkspaceSink::Frame(workspace) => super::prediction::reconstruct_placed_inter_block(
                interintra_scratch,
                residual_scratch,
                workspace,
                &self.placed,
                residual_blocks,
                motion,
                use_refinemv,
                refinemv_switchable,
                block_decoded,
                shared.ref_frame_idx,
                shared.reference,
                self.qindex,
                shared.luma_use_tcq,
                shared.residual_use_ddt,
                shared.bit_depth,
                super::sequence_enables_ibp(shared.sequence),
                self.tile_offset,
            ),
            sink @ (WorkspaceSink::Rect(_) | WorkspaceSink::OwnedRect(_)) => {
                super::prediction::reconstruct_pure_inter_block(
                    sink,
                    residual_scratch,
                    &self.placed,
                    residual_blocks,
                    motion,
                    use_refinemv,
                    refinemv_switchable,
                    shared.ref_frame_idx,
                    shared.reference,
                    self.qindex,
                    shared.luma_use_tcq,
                    shared.residual_use_ddt,
                    shared.bit_depth,
                    self.tile_offset,
                )
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reconstruct_ordered<T: ReconSample>(
        &self,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        residual_blocks: &[InterResidualBlock],
        shared: &ReconShared<'_, T>,
        tip_scratch: &mut TipReconstructScratch<T>,
        interintra_scratch: &mut super::interintra::InterIntraScratch<T>,
        residual_scratch: &mut InterResidualReconScratch<T>,
    ) -> Result<()> {
        let motion = self.derive_motion(sink, temporal_records, shared, tip_scratch)?;
        self.reconstruct_from_motion(
            sink,
            block_decoded,
            motion,
            residual_blocks,
            shared,
            tip_scratch,
            interintra_scratch,
            residual_scratch,
        )
    }
}

impl<T: ReconSample> InterReconScratch<T> {
    pub(super) fn with_installed<R>(
        &mut self,
        f: impl FnOnce(&mut InterReconScratch<T>) -> R,
    ) -> R {
        let mut mc = core::mem::replace(&mut self.mc, mc::McScratch::empty());
        let result = mc.with_installed(|| f(self));
        self.mc = mc;
        result
    }

    pub(super) fn general_intra_mut(
        &mut self,
    ) -> &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T> {
        &mut self.general_intra
    }

    pub(super) fn reconstruct_intrabc(
        &mut self,
        command: super::intrabc::IntrabcReconCommand,
        residual_blocks: &[InterResidualBlock],
        workspace: &mut CurrentFrameWorkspace<T>,
    ) -> Result<()> {
        let Self { residual, mc, .. } = self;
        mc.with_installed(|| command.reconstruct(residual, residual_blocks, workspace))
    }

    /// Derives one command's motion into `temporal_records`, writing no sample.
    pub(super) fn motion(
        &mut self,
        command: &InterReconCommand,
        sink: &WorkspaceSink<'_, '_, T>,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        shared: &ReconShared<'_, T>,
    ) -> Result<Option<mc::CompoundMotionGrid>> {
        let Self { tip, mc, .. } = self;
        mc.with_installed(|| command.derive_motion(sink, temporal_records, shared, tip))
    }

    /// Reconstructs one command from the grid its motion half derived.
    pub(super) fn reconstruct_from_motion(
        &mut self,
        command: &InterReconCommand,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        motion: Option<mc::CompoundMotionGrid>,
        residual_blocks: &[InterResidualBlock],
        shared: &ReconShared<'_, T>,
    ) -> Result<()> {
        let Self {
            tip,
            interintra,
            residual,
            mc,
            ..
        } = self;
        mc.with_installed(|| {
            command.reconstruct_from_motion(
                sink,
                block_decoded,
                motion,
                residual_blocks,
                shared,
                tip,
                interintra,
                residual,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct_logged(
        &mut self,
        command: &InterReconCommand,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        residual_blocks: &[InterResidualBlock],
        temporal_context: &TemporalMvContext,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
        luma_use_tcq: bool,
        residual_use_ddt: bool,
        bit_depth: BitDepth,
    ) -> Result<()> {
        let Self {
            tip,
            interintra,
            residual,
            mc,
            ..
        } = self;
        mc.with_installed(|| {
            command.reconstruct_ordered(
                sink,
                block_decoded,
                temporal_records,
                residual_blocks,
                &ReconShared {
                    reference,
                    ref_frame_idx,
                    temporal_context,
                    sequence,
                    core,
                    luma_use_tcq,
                    residual_use_ddt,
                    bit_depth,
                    mi_rows,
                    mi_cols,
                    current_order_hint,
                },
                tip,
                interintra,
                residual,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct(
        &mut self,
        command: &InterReconCommand,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
        motion: &MotionFieldUnits,
        ordinal: usize,
        residual_blocks: &[InterResidualBlock],
        temporal_context: &TemporalMvContext,
        reference: &InterReferenceState<T>,
        ref_frame_idx: &[u32],
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
        luma_use_tcq: bool,
        residual_use_ddt: bool,
        bit_depth: BitDepth,
    ) -> Result<()> {
        let mut temporal = core::mem::take(&mut self.temporal);
        temporal.clear();
        let result = self.reconstruct_logged(
            command,
            &mut WorkspaceSink::Frame(workspace),
            block_decoded,
            &mut temporal,
            residual_blocks,
            temporal_context,
            reference,
            ref_frame_idx,
            sequence,
            core,
            mi_rows,
            mi_cols,
            current_order_hint,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
        );
        if result.is_ok() {
            motion.fold_unit(ordinal, &temporal);
        }
        temporal.clear();
        self.temporal = temporal;
        result
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepass_write_is_contained(
    luma_rect: [usize; 4],
    chroma_luma_rect: [usize; 4],
    predict_chroma: bool,
    reads_current_frame: bool,
    residual: Option<&InterResidual>,
    superblock_origin: [usize; 2],
    sb_h4: usize,
    info: DecodedFrameInfo,
    residual_blocks: &[InterResidualBlock],
) -> bool {
    if reads_current_frame {
        return false;
    }
    let [luma_x, luma_y, luma_w, luma_h] = luma_rect;
    let [chroma_luma_x, chroma_luma_y, chroma_luma_w, chroma_luma_h] = chroma_luma_rect;
    let Some(origin_y) = superblock_origin[0].checked_mul(4) else {
        return false;
    };
    let Some(origin_x) = superblock_origin[1].checked_mul(4) else {
        return false;
    };
    let Some(side) = sb_h4.checked_mul(4) else {
        return false;
    };
    let luma = info.coded_luma_size();
    if !clipped_rect_is_inside_band(
        luma_x,
        luma_y,
        luma_w,
        luma_h,
        origin_x,
        origin_y,
        side,
        side,
        luma.width(),
        luma.height(),
    ) {
        return false;
    }
    if predict_chroma
        && !clipped_rect_is_inside_band(
            chroma_luma_x,
            chroma_luma_y,
            chroma_luma_w,
            chroma_luma_h,
            origin_x,
            origin_y,
            side,
            side,
            luma.width(),
            luma.height(),
        )
    {
        return false;
    }
    residual.is_none_or(|residual| {
        residual.blocks(residual_blocks).is_some_and(|blocks| {
            blocks.iter().all(|block| {
                let (sub_x, sub_y, storage) = match block.plane {
                    PlaneId::Y => (0, 0, Some(luma)),
                    PlaneId::U | PlaneId::V => {
                        let format = info.pixel_format();
                        (
                            usize::from(format.subsampling_x()),
                            usize::from(format.subsampling_y()),
                            format.chroma_size(luma).ok().flatten(),
                        )
                    }
                };
                let Some(storage) = storage else {
                    return false;
                };
                let Some(width) = 1usize.checked_shl(block.log2_width) else {
                    return false;
                };
                let Some(height) = 1usize.checked_shl(block.log2_height) else {
                    return false;
                };
                clipped_rect_is_inside_band(
                    block.x,
                    block.y,
                    width,
                    height,
                    origin_x >> sub_x,
                    origin_y >> sub_y,
                    side >> sub_x,
                    side >> sub_y,
                    storage.width(),
                    storage.height(),
                )
            })
        })
    })
}

#[allow(clippy::too_many_arguments)]
fn clipped_rect_is_inside_band(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    band_x: usize,
    band_y: usize,
    band_w: usize,
    band_h: usize,
    storage_w: usize,
    storage_h: usize,
) -> bool {
    let Some(end_x) = x.checked_add(width) else {
        return false;
    };
    let Some(end_y) = y.checked_add(height) else {
        return false;
    };
    let Some(band_end_x) = band_x.checked_add(band_w) else {
        return false;
    };
    let Some(band_end_y) = band_y.checked_add(band_h) else {
        return false;
    };
    width != 0
        && height != 0
        && x < storage_w
        && y < storage_h
        && x >= band_x
        && y >= band_y
        && end_x.min(storage_w) <= band_end_x.min(storage_w)
        && end_y.min(storage_h) <= band_end_y.min(storage_h)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use splot_core::span::ByteOffset;
    use splot_recon::{
        BitDepth, DecodedFrameInfo, InterpolationFilter, OutputIndex, PixelFormat, PlaneId,
        PlaneRect, PlaneSize,
    };

    use super::{InterReconCommand, InterReconScratch, PendingKind};
    use crate::bitstream::tile_payload::LumaCoeffBlock;
    use crate::prediction::inter::{
        BawpSyntax, InterBlock, InterResidual, InterResidualBlock, Mv, PlacedInterBlock, mc,
    };

    fn assert_send<T: Send>() {}

    #[test]
    fn inter_recon_command_is_send() {
        assert_send::<InterReconCommand>();
    }

    #[test]
    fn worker_reconstruction_scratch_is_cache_aligned() {
        assert_eq!(core::mem::align_of::<InterReconScratch<u8>>(), 64);
    }

    #[test]
    fn current_frame_dependency_is_limited_to_bawp_and_interintra() {
        assert!(!super::reads_current_frame(false, false));
        assert!(super::reads_current_frame(true, false));
        assert!(super::reads_current_frame(false, true));
        assert!(super::reads_current_frame(true, true));
    }

    fn info(width: usize, height: usize, format: PixelFormat) -> DecodedFrameInfo {
        let size = PlaneSize::new(width, height).expect("frame size");
        DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            format,
            size,
            PlaneRect::new(0, 0, width, height).expect("visible rect"),
        )
        .expect("frame info")
    }

    fn command(x: usize, y: usize, width: usize, height: usize) -> InterReconCommand {
        InterReconCommand::new(
            PlacedInterBlock {
                luma_x: x,
                luma_y: y,
                luma_w: width,
                luma_h: height,
                chroma_luma_x: x,
                chroma_luma_y: y,
                chroma_luma_w: width,
                chroma_luma_h: height,
                predict_chroma: false,
                sub8x8_chroma: false,
                interintra_chroma: false,
                block: InterBlock {
                    ref_frame0: 0,
                    ref_frame1: None,
                    mv: Mv::ZERO,
                    mv1: Mv::ZERO,
                    interp: InterpolationFilter::EightTap,
                    warp_params: [None, None],
                    bawp: BawpSyntax::default(),
                    interintra: None,
                    compound_blend: mc::CompoundBlend::default(),
                    optflow_distances: None,
                    residual: None,
                },
            },
            PendingKind::Single,
            0,
            0,
            ByteOffset::new(0),
        )
    }

    fn residual(
        blocks: &mut Vec<InterResidualBlock>,
        plane: PlaneId,
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
    ) -> InterResidual {
        let start = blocks.len();
        blocks.push(InterResidualBlock {
            plane,
            x,
            y,
            tx_size: 0,
            log2_width,
            log2_height,
            cctx_pair_delta: 0,
            coeffs: LumaCoeffBlock {
                all_zero: true,
                eob: 0,
                quant: Vec::new(),
                intra_ist: None,
                cctx_type: None,
                plane_tx_type: 0,
                use_tcq: false,
                lossless: false,
            },
        });
        InterResidual {
            block_range: start..blocks.len(),
        }
    }

    #[test]
    fn footprint_accepts_contained_chroma_sub8_and_partial_edge() {
        for format in [
            PixelFormat::Monochrome,
            PixelFormat::Yuv420,
            PixelFormat::Yuv422,
            PixelFormat::Yuv444,
        ] {
            let mut full = command(0, 0, 64, 64);
            full.placed.predict_chroma = format != PixelFormat::Monochrome;
            full.placed.sub8x8_chroma = true;
            full.placed.chroma_luma_x = 4;
            full.placed.chroma_luma_y = 4;
            full.placed.chroma_luma_w = 60;
            full.placed.chroma_luma_h = 60;
            assert!(full.prepass_write_is_contained([0, 0], 16, info(128, 128, format), &[]));
        }

        let edge = command(64, 64, 4, 4);
        assert!(edge.prepass_write_is_contained(
            [16, 16],
            16,
            info(65, 65, PixelFormat::Monochrome),
            &[],
        ));
    }

    #[test]
    fn footprint_rejects_cross_sb_and_residual_bottom_by_one() {
        let crossing = command(63, 0, 2, 8);
        assert!(!crossing.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome),
            &[],
        ));

        let mut residual_crossing = command(0, 0, 64, 64);
        let mut blocks = Vec::new();
        residual_crossing.placed.block.residual =
            Some(residual(&mut blocks, PlaneId::Y, 0, 63, 1, 1));
        assert!(!residual_crossing.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome),
            &blocks,
        ));
    }

    #[test]
    fn footprint_checks_native_chroma_plane_bounds() {
        for (format, plane_height) in [
            (PixelFormat::Yuv420, 32usize),
            (PixelFormat::Yuv422, 64usize),
            (PixelFormat::Yuv444, 64usize),
        ] {
            let log2_height = plane_height.ilog2();
            let mut blocks = Vec::new();
            let mut exact = command(0, 0, 64, 64);
            exact.placed.block.residual =
                Some(residual(&mut blocks, PlaneId::U, 0, 0, 1, log2_height));
            assert!(exact.prepass_write_is_contained([0, 0], 16, info(128, 128, format), &blocks,));

            let mut crossing = command(0, 0, 64, 64);
            crossing.placed.block.residual =
                Some(residual(&mut blocks, PlaneId::V, 0, plane_height - 1, 1, 1));
            assert!(!crossing.prepass_write_is_contained(
                [0, 0],
                16,
                info(128, 128, format),
                &blocks,
            ));
        }

        let mut monochrome = command(0, 0, 64, 64);
        let mut blocks = Vec::new();
        monochrome.placed.block.residual = Some(residual(&mut blocks, PlaneId::U, 0, 0, 1, 1));
        assert!(!monochrome.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome),
            &blocks,
        ));
    }
}
