// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrParams};
use splot_core::headers::sequence::{ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::SymbolDecoder;

use super::DecodeTileWorkUnit;
use super::intra_joint_modes::{
    IsCflContext, TileFscModeState, TileFscModeStateError, TileIntraJointModeState,
    TileIntraJointModeStateError, TileLumaPaletteState, TileLumaPaletteStateError, TileUseDipState,
    TileUseDipStateError, TileUsesMrlsState, TileUsesMrlsStateError, TileUvCflState,
    TileUvCflStateError,
};
use super::mi_size_state::{TileMiSizeState, TileMiSizeStateError};
use super::partition_allowed::PartitionFeatureFlags;
pub(crate) use super::partition_traversal::GeneralIntraPartitionTreeOutput as GeneralIntraMultiblockOutput;
use super::partition_traversal::{
    DecodeBlockFrontier, DecodedLeafPublication, GeneralIntraLeafMode,
    GeneralIntraPartitionTreeCursor, GeneralIntraTreeWalkError, LrTileRecords,
    TilePartitionFrameFacts, TilePartitionLoopRestorationFrameState,
    TilePartitionLoopRestorationPlaneTool, TilePartitionLoopRestorationState,
    TilePartitionTraversalError,
};
use crate::DecodeLimits;

const BLOCK_64X64_INDEX: usize = 12;
const BLOCK_128X128_INDEX: usize = 15;
const BLOCK_256X256_INDEX: usize = 18;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TilePartitionFrontierError {
    #[error("partition frontier missing fact: {fact}")]
    MissingFact { fact: &'static str },
    #[error("partition traversal failed: {0}")]
    Traversal(#[from] TilePartitionTraversalError),
    #[error("MI-size state failed: {0}")]
    MiSizeState(#[from] TileMiSizeStateError),
    #[error("intra joint-mode state failed: {0}")]
    IntraJointModeState(#[from] TileIntraJointModeStateError),
    #[error("intra UsesMrls state failed: {0}")]
    UsesMrlsState(#[from] TileUsesMrlsStateError),
    #[error("intra UseDip state failed: {0}")]
    UseDipState(#[from] TileUseDipStateError),
    #[error("intra FscModes state failed: {0}")]
    FscModeState(#[from] TileFscModeStateError),
    #[error("luma palette state failed: {0}")]
    LumaPaletteState(#[from] TileLumaPaletteStateError),
    #[error("intra UVCfls state failed: {0}")]
    UvCflState(#[from] TileUvCflStateError),
    #[error("partition frontier mismatch: {reason}")]
    UnexpectedFrontier { reason: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GeneralIntraMultiblockError<E> {
    #[error("general intra multi-block setup failed: {0}")]
    Setup(#[from] TilePartitionFrontierError),
    #[error("general intra multi-block tree walk failed: {0}")]
    Walk(GeneralIntraTreeWalkError<E>),
}

pub(crate) struct GeneralIntraMultiblockCursor<'payload> {
    tree: GeneralIntraPartitionTreeCursor<'payload>,
    mi_size_state: TileMiSizeState,
    joint_modes: TileIntraJointModeState,
    uses_mrls: TileUsesMrlsState,
    use_dip: TileUseDipState,
    fsc_modes: TileFscModeState,
    palette_y: TileLumaPaletteState,
    uv_cfls: TileUvCflState,
}

impl<'payload> GeneralIntraMultiblockCursor<'payload> {
    pub(crate) fn new(
        work_unit: &DecodeTileWorkUnit<'payload>,
        sequence: &SequenceHeader,
        core: &FrameHeaderCore,
        limits: DecodeLimits,
        lr_records: LrTileRecords,
    ) -> Result<Self, TilePartitionFrontierError> {
        let frame = minimal_partition_frame_facts(sequence, core)?;
        let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
        let tile_rows = work_unit.mi_row_range().start as usize
            ..(work_unit.mi_row_range().end as usize).min(mi_rows);
        let tile_cols = work_unit.mi_col_range().start as usize
            ..(work_unit.mi_col_range().end as usize).min(mi_cols);
        let mi_size_state =
            TileMiSizeState::new_for_tile(tile_rows.clone(), tile_cols.clone(), frame.sb_size())?;
        let joint_modes =
            TileIntraJointModeState::new_for_tile(tile_rows.clone(), tile_cols.clone())?;
        let sb_size4 = frame
            .sb_size()
            .num_4x4_wide()
            .map_err(TilePartitionTraversalError::from)?
            .max(1);
        let uses_mrls =
            TileUsesMrlsState::new_for_tile(tile_rows.clone(), tile_cols.clone(), sb_size4)?;
        let use_dip =
            TileUseDipState::new_for_tile(tile_rows.clone(), tile_cols.clone(), sb_size4)?;
        let fsc_modes =
            TileFscModeState::new_for_tile(tile_rows.clone(), tile_cols.clone(), sb_size4)?;
        let palette_y =
            TileLumaPaletteState::new_for_tile(tile_rows.clone(), tile_cols.clone(), sb_size4)?;
        let uv_cfls = TileUvCflState::new(tile_rows.len(), tile_cols.len())?
            .with_origin(tile_rows.start, tile_cols.start);
        let tree = GeneralIntraPartitionTreeCursor::new(work_unit, frame, limits, lr_records)?;
        Ok(Self {
            tree,
            mi_size_state,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_y,
            uv_cfls,
        })
    }

    pub(crate) fn decode_next_superblock<E, C, F, P>(
        &mut self,
        work_unit: &mut DecodeTileWorkUnit<'payload>,
        on_leaf: &mut F,
        on_published: &mut P,
    ) -> Result<Option<[usize; 2]>, GeneralIntraMultiblockError<E>>
    where
        F: FnMut(
            &mut DecodeTileWorkUnit<'payload>,
            &mut SymbolDecoder<'payload>,
            &DecodeBlockFrontier,
            &TileIntraJointModeState,
            &TileUsesMrlsState,
            &TileUseDipState,
            &TileFscModeState,
            &TileLumaPaletteState,
            IsCflContext,
        ) -> Result<(GeneralIntraLeafMode, C), E>,
        P: FnMut(DecodedLeafPublication, C),
    {
        let Self {
            tree,
            mi_size_state,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_y,
            uv_cfls,
        } = self;
        tree.decode_next_superblock_with_publication(
            work_unit,
            mi_size_state,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_y,
            uv_cfls,
            on_leaf,
            on_published,
        )
        .map_err(GeneralIntraMultiblockError::Walk)
    }

    pub(crate) fn into_output(self) -> GeneralIntraMultiblockOutput<'payload> {
        let Self {
            tree,
            mi_size_state: _,
            joint_modes,
            uses_mrls,
            use_dip,
            fsc_modes,
            palette_y,
            uv_cfls,
        } = self;
        joint_modes.recycle();
        uses_mrls.recycle();
        use_dip.recycle();
        fsc_modes.recycle();
        palette_y.recycle();
        uv_cfls.recycle();
        tree.into_output()
    }
}

pub(crate) fn minimal_partition_frame_facts(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> Result<TilePartitionFrameFacts, TilePartitionFrontierError> {
    let partition = sequence
        .partition
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "sequence.partition",
        })?;
    let frame_is_intra = core
        .frame_is_intra
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "frame_is_intra",
        })?;
    let (mi_rows, mi_cols) = frame_mi_dimensions(core)?;
    let chroma = sequence.general.chroma_format_idc;
    let num_planes = if chroma.is_monochrome() { 1 } else { 3 };
    let (subsampling_x, subsampling_y) = chroma_subsampling(chroma);
    let filter = sequence
        .filter
        .ok_or(TilePartitionFrontierError::MissingFact {
            fact: "sequence.filter",
        })?;
    let lr_params = core
        .lr_params
        .as_ref()
        .ok_or(TilePartitionFrontierError::MissingFact { fact: "lr_params" })?;
    let loop_restoration = loop_restoration_state(lr_params, num_planes);

    let sb_size_index = frame_sb_size_index(partition.seq_sb_size(), frame_is_intra);

    Ok(TilePartitionFrameFacts::new(
        mi_rows,
        mi_cols,
        sb_size_index,
        num_planes,
        subsampling_x,
        subsampling_y,
        frame_is_intra,
        partition.enable_sdp,
        partition.enable_extended_sdp,
        filter.disable_loopfilters_across_tiles,
        loop_restoration,
        PartitionFeatureFlags::new(
            partition.enable_ext_partitions,
            partition.enable_uneven_4way_partitions,
        ),
        partition.max_pb_aspect_ratio as usize,
    )?)
}

fn loop_restoration_state(lr: &LrParams, num_planes: usize) -> TilePartitionLoopRestorationState {
    if !lr.uses_lr {
        return TilePartitionLoopRestorationState::NoSyntax;
    }
    let mut plane_tool = [TilePartitionLoopRestorationPlaneTool::None; 3];
    let mut frame_filters_on = [false; 3];
    let mut unit_size = [0usize; 3];
    for plane in 0..num_planes.min(3) {
        let Some(params) = lr.planes.get(plane) else {
            return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
        };
        match params.restoration_type {
            FrameRestorationType::None => {}
            FrameRestorationType::WienerNonsep => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::WienerNs;
                frame_filters_on[plane] = params.frame_filters_on;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener if plane == 0 => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::PcWiener;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::Switchable if plane == 0 => {
                plane_tool[plane] = TilePartitionLoopRestorationPlaneTool::Switchable;
                frame_filters_on[plane] = params.frame_filters_on;
                unit_size[plane] = lr.loop_restoration_size[plane] as usize;
            }
            FrameRestorationType::PcWiener | FrameRestorationType::Switchable => {
                return TilePartitionLoopRestorationState::UnsupportedReadLrSyntax;
            }
        }
    }
    if plane_tool
        .iter()
        .any(|tool| *tool != TilePartitionLoopRestorationPlaneTool::None)
    {
        TilePartitionLoopRestorationState::Frame(TilePartitionLoopRestorationFrameState::new(
            plane_tool,
            frame_filters_on,
            unit_size,
        ))
    } else {
        TilePartitionLoopRestorationState::UnsupportedReadLrSyntax
    }
}

pub(crate) fn frame_mi_dimensions(
    core: &FrameHeaderCore,
) -> Result<(usize, usize), TilePartitionFrontierError> {
    let tile_info = core
        .tile_info
        .as_ref()
        .ok_or(TilePartitionFrontierError::MissingFact { fact: "tile_info" })?;
    let mi_rows =
        *tile_info
            .mi_row_starts
            .last()
            .ok_or(TilePartitionFrontierError::MissingFact {
                fact: "mi_row_starts",
            })? as usize;
    let mi_cols =
        *tile_info
            .mi_col_starts
            .last()
            .ok_or(TilePartitionFrontierError::MissingFact {
                fact: "mi_col_starts",
            })? as usize;
    if mi_rows == 0 || mi_cols == 0 {
        return Err(TilePartitionFrontierError::UnexpectedFrontier {
            reason: "empty_mi_dimensions",
        });
    }
    Ok((mi_rows, mi_cols))
}

fn frame_sb_size_index(seq_sb_size: SuperblockSize, frame_is_intra: bool) -> usize {
    match (seq_sb_size, frame_is_intra) {
        (SuperblockSize::Block256x256, true) | (SuperblockSize::Block128x128, _) => {
            BLOCK_128X128_INDEX
        }
        (SuperblockSize::Block256x256, false) => BLOCK_256X256_INDEX,
        (SuperblockSize::Block64x64, _) => BLOCK_64X64_INDEX,
    }
}

pub(crate) fn chroma_subsampling(chroma: ChromaFormatIdc) -> (bool, bool) {
    match chroma {
        ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (true, true),
        ChromaFormatIdc::Yuv422 => (true, false),
        ChromaFormatIdc::Yuv444 => (false, false),
    }
}

#[cfg(test)]
#[path = "tile_frontier_tests.rs"]
mod tests;
