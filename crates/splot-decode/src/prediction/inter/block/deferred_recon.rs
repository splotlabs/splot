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
use super::super::find_mv_stack::{TemporalMotionBlock, TemporalMotionField, TemporalMvContext};
use super::super::mc::WorkspaceSink;
use super::super::{InterReferenceState, PlacedInterBlock};
use super::compound_path::append_compound_temporal_motion;
use super::temporal::{commit_temporal_motion_blocks, temporal_motion_block};
use super::tip::{self, TipReconstructScratch};
use crate::Result;
use crate::bitstream::tile_payload::{
    FrameQmSegmentScope, TileBlockDecodedState, current_frame_qm_segment_id,
};

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
pub(super) struct InterReconCommand<T: ReconSample> {
    placed: PlacedInterBlock,
    kind: PendingKind,
    segment_id: usize,
    qindex: u32,
    tile_offset: ByteOffset,
    tip_scratch: Option<TipReconstructScratch<T>>,
}

#[derive(Debug, Default)]
pub(super) struct InterReconScratch<T: ReconSample> {
    tip: Vec<TipReconstructScratch<T>>,
    temporal: Vec<TemporalMotionBlock>,
}

struct ReconShared<'a, 'r, T: ReconSample> {
    reference: &'a InterReferenceState<'r, T>,
    ref_frame_idx: &'a [u32],
    temporal_context: &'a TemporalMvContext,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
}

const fn reads_current_frame(bawp: bool, interintra: bool) -> bool {
    bawp || interintra
}

impl<T: ReconSample> InterReconCommand<T> {
    pub(super) fn new(
        placed: PlacedInterBlock,
        kind: PendingKind,
        qindex: u32,
        tile_offset: ByteOffset,
    ) -> Self {
        Self {
            placed,
            kind,
            segment_id: current_frame_qm_segment_id(),
            qindex,
            tile_offset,
            tip_scratch: None,
        }
    }

    pub(super) fn reads_current_frame(&self) -> bool {
        reads_current_frame(
            self.placed.block.bawp.enabled,
            self.placed.block.interintra.is_some(),
        )
    }

    pub(super) fn prepass_write_is_contained(
        &self,
        superblock_origin: [usize; 2],
        sb_h4: usize,
        info: DecodedFrameInfo,
    ) -> bool {
        if self.reads_current_frame() {
            return false;
        }
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
            self.placed.luma_x,
            self.placed.luma_y,
            self.placed.luma_w,
            self.placed.luma_h,
            origin_x,
            origin_y,
            side,
            side,
            luma.width(),
            luma.height(),
        ) {
            return false;
        }
        if self.placed.predict_chroma
            && !clipped_rect_is_inside_band(
                self.placed.chroma_luma_x,
                self.placed.chroma_luma_y,
                self.placed.chroma_luma_w,
                self.placed.chroma_luma_h,
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
        self.placed.block.residual.as_ref().is_none_or(|residual| {
            residual.blocks.iter().all(|block| {
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

    #[allow(clippy::too_many_arguments)]
    fn reconstruct_ordered(
        &mut self,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        shared: &ReconShared<'_, '_, T>,
        mi_rows: usize,
        mi_cols: usize,
        current_order_hint: u32,
    ) -> Result<()> {
        let _segment_scope = FrameQmSegmentScope::install(self.segment_id);
        if matches!(self.kind, PendingKind::Tip) {
            let allow_unit_parallelism = matches!(sink, WorkspaceSink::Frame(_));
            let scratch = self.tip_scratch.as_mut().ok_or_else(|| {
                super::super::unsupported_at(
                    "inter_recon_missing_tip_scratch",
                    self.tile_offset,
                    "missing tip scratch buffer",
                    "7.10.6",
                )
            })?;
            tip::reconstruct(
                scratch,
                temporal_records,
                sink,
                allow_unit_parallelism,
                &self.placed,
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
            )?;
            return Ok(());
        }

        let (use_refinemv, refinemv_switchable) = self.refinemv();
        let grid = match sink {
            WorkspaceSink::Frame(workspace) => super::prediction::reconstruct_placed_inter_block(
                workspace,
                &self.placed,
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
            )?,
            WorkspaceSink::Row(row) => {
                let mut sink = WorkspaceSink::Row(&mut **row);
                super::prediction::reconstruct_pure_inter_block(
                    &mut sink,
                    &self.placed,
                    use_refinemv,
                    refinemv_switchable,
                    shared.ref_frame_idx,
                    shared.reference,
                    self.qindex,
                    shared.luma_use_tcq,
                    shared.residual_use_ddt,
                    shared.bit_depth,
                    self.tile_offset,
                )?
            }
        };
        match self.kind {
            PendingKind::Single => {
                let block = &self.placed.block;
                temporal_records.push(temporal_motion_block(
                    shared.reference,
                    shared.ref_frame_idx,
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
                ));
                Ok(())
            }
            PendingKind::Compound {
                syntax,
                warp_params,
                mi_row,
                mi_col,
                ..
            } => append_compound_temporal_motion(
                temporal_records,
                shared.reference,
                shared.ref_frame_idx,
                &self.placed,
                syntax,
                warp_params,
                grid.as_ref(),
                mi_row,
                mi_col,
                mi_rows,
                mi_cols,
                current_order_hint,
            ),
            PendingKind::Tip => Ok(()),
        }
    }
}

impl<T: ReconSample> InterReconScratch<T> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct_logged(
        &mut self,
        mut command: InterReconCommand<T>,
        sink: &mut WorkspaceSink<'_, '_, T>,
        block_decoded: &TileBlockDecodedState,
        temporal_records: &mut Vec<TemporalMotionBlock>,
        temporal_context: &TemporalMvContext,
        reference: &InterReferenceState<'_, T>,
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
        if matches!(command.kind, PendingKind::Tip) {
            command.tip_scratch = Some(self.tip.pop().unwrap_or_default());
        }
        let result = command.reconstruct_ordered(
            sink,
            block_decoded,
            temporal_records,
            &ReconShared {
                reference,
                ref_frame_idx,
                temporal_context,
                sequence,
                core,
                luma_use_tcq,
                residual_use_ddt,
                bit_depth,
            },
            mi_rows,
            mi_cols,
            current_order_hint,
        );
        if let Some(scratch) = command.tip_scratch.take() {
            self.tip.push(scratch);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct(
        &mut self,
        command: InterReconCommand<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
        motion_field: &mut TemporalMotionField,
        temporal_context: &TemporalMvContext,
        reference: &InterReferenceState<'_, T>,
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
            commit_temporal_motion_blocks(motion_field, &temporal);
        }
        temporal.clear();
        self.temporal = temporal;
        result
    }
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

    use super::{InterReconCommand, PendingKind};
    use crate::bitstream::tile_payload::LumaCoeffBlock;
    use crate::prediction::inter::{
        BawpSyntax, InterBlock, InterResidual, InterResidualBlock, Mv, PlacedInterBlock, mc,
    };

    fn assert_send<T: Send>() {}

    #[test]
    fn inter_recon_command_is_send() {
        assert_send::<InterReconCommand<u8>>();
        assert_send::<InterReconCommand<u16>>();
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

    fn command(x: usize, y: usize, width: usize, height: usize) -> InterReconCommand<u8> {
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
            ByteOffset::new(0),
        )
    }

    fn residual(
        plane: PlaneId,
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
    ) -> InterResidual {
        InterResidual {
            blocks: vec![InterResidualBlock {
                plane,
                x,
                y,
                tx_size: 0,
                log2_width,
                log2_height,
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
            }],
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
            assert!(full.prepass_write_is_contained([0, 0], 16, info(128, 128, format)));
        }

        let edge = command(64, 64, 4, 4);
        assert!(edge.prepass_write_is_contained(
            [16, 16],
            16,
            info(65, 65, PixelFormat::Monochrome)
        ));
    }

    #[test]
    fn footprint_rejects_cross_sb_and_residual_bottom_by_one() {
        let crossing = command(63, 0, 2, 8);
        assert!(!crossing.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome)
        ));

        let mut residual_crossing = command(0, 0, 64, 64);
        residual_crossing.placed.block.residual = Some(residual(PlaneId::Y, 0, 63, 1, 1));
        assert!(!residual_crossing.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome)
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
            let mut exact = command(0, 0, 64, 64);
            exact.placed.block.residual = Some(residual(PlaneId::U, 0, 0, 1, log2_height));
            assert!(exact.prepass_write_is_contained([0, 0], 16, info(128, 128, format)));

            let mut crossing = command(0, 0, 64, 64);
            crossing.placed.block.residual = Some(residual(PlaneId::V, 0, plane_height - 1, 1, 1));
            assert!(!crossing.prepass_write_is_contained([0, 0], 16, info(128, 128, format)));
        }

        let mut monochrome = command(0, 0, 64, 64);
        monochrome.placed.block.residual = Some(residual(PlaneId::U, 0, 0, 1, 1));
        assert!(!monochrome.prepass_write_is_contained(
            [0, 0],
            16,
            info(128, 128, PixelFormat::Monochrome)
        ));
    }
}
