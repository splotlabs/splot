// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use splot_parallel::prelude::*;
use splot_recon::{PlaneId, PlaneRect};

use super::*;

// Covers the pixel workspace plus the worker's full-frame per-MI decode grids.
const PARALLEL_TILE_SCRATCH_MEMORY_MULTIPLIER: usize = 16;

pub(super) struct TileDecodeOutput {
    pub(super) cdef_state: CdefState,
    pub(super) gdf_state: GdfState,
    pub(super) ccso_state: CcsoState,
    pub(super) motion_field: TemporalMotionField,
    pub(super) deblock_blocks: Vec<crate::filters::deblock::DeblockBlock>,
    pub(super) chroma_deblock_blocks: [Vec<crate::filters::deblock::DeblockBlock>; 2],
    pub(super) tx_skip_records: Vec<crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord>,
    pub(super) active_source_blocks: Vec<crate::bitstream::tile_payload::WienerNsLrSourceBlock>,
    pub(super) unit_filters: Vec<crate::bitstream::tile_payload::WienerNsLrUnitFilter>,
}

struct DecodedTileChunk<T: ReconSample> {
    workspace: CurrentFrameWorkspace<T>,
    output: TileDecodeOutput,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_tiles<T: ReconSample>(
    work_units: &mut [DecodeTileWorkUnit<'_>],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
    chroma_smooth_rows: usize,
    chroma_smooth_cols: usize,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<'_, T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    cdef_state: CdefState,
    gdf_state: GdfState,
    ccso_state: CcsoState,
    motion_field: TemporalMotionField,
) -> Result<TileDecodeOutput> {
    let worker_count = parallel_tile_worker_count(work_units.len(), workspace, limits);
    if worker_count < 2 {
        return decode_tile_chunk(
            work_units,
            sequence,
            core,
            limits,
            chroma_smooth_rows,
            chroma_smooth_cols,
            mi_rows,
            mi_cols,
            sb_h4,
            max_drl_bits_minus_1,
            frame_interpolation_filter,
            residual_tool_policy,
            num_total_refs,
            reference_select,
            num_same_ref_compound,
            temporal_context,
            reference,
            workspace,
            luma_use_tcq,
            residual_use_ddt,
            ref_frame_idx,
            bit_depth,
            enable_adaptive_mvd,
            allow_bawp,
            allow_warpmv_mode,
            frame_is_switch,
            current_order_hint,
            cdef_state,
            gdf_state,
            ccso_state,
            motion_field,
        );
    }

    let chunk_size = work_units.len().div_ceil(worker_count);
    let workspace_info = workspace.info();
    let quantizer = crate::bitstream::tile_payload::FrameQuantizerSnapshot::capture();
    let segment_id = current_frame_qm_segment_id();
    let chunks: Vec<Result<DecodedTileChunk<T>>> = work_units
        .par_chunks_mut(chunk_size)
        .map(|chunk| {
            let _quantizer_scopes = quantizer.install(segment_id);
            let mut chunk_workspace = CurrentFrameWorkspace::new(workspace_info, T::default())?;
            let output = decode_tile_chunk(
                chunk,
                sequence,
                core,
                limits,
                chroma_smooth_rows,
                chroma_smooth_cols,
                mi_rows,
                mi_cols,
                sb_h4,
                max_drl_bits_minus_1,
                frame_interpolation_filter,
                residual_tool_policy,
                num_total_refs,
                reference_select,
                num_same_ref_compound,
                temporal_context,
                reference,
                &mut chunk_workspace,
                luma_use_tcq,
                residual_use_ddt,
                ref_frame_idx,
                bit_depth,
                enable_adaptive_mvd,
                allow_bawp,
                allow_warpmv_mode,
                frame_is_switch,
                current_order_hint,
                cdef_state.clone(),
                gdf_state.clone(),
                ccso_state.clone(),
                motion_field.clone(),
            )?;
            Ok(DecodedTileChunk {
                workspace: chunk_workspace,
                output,
            })
        })
        .collect();
    let chunks = chunks.into_iter().collect::<Result<Vec<_>>>()?;

    let mut output = TileDecodeOutput {
        cdef_state,
        gdf_state,
        ccso_state,
        motion_field,
        deblock_blocks: Vec::new(),
        chroma_deblock_blocks: [Vec::new(), Vec::new()],
        tx_skip_records: Vec::new(),
        active_source_blocks: Vec::new(),
        unit_filters: Vec::new(),
    };
    for (tiles, decoded) in work_units.chunks(chunk_size).zip(chunks) {
        merge_decoded_chunk(&mut output, workspace, tiles, decoded)?;
    }
    Ok(output)
}

fn parallel_tile_worker_count<T: ReconSample>(
    tile_count: usize,
    workspace: &CurrentFrameWorkspace<T>,
    limits: crate::DecodeLimits,
) -> usize {
    if tile_count < 2 || !splot_parallel::on_multiworker_pool() {
        return 1;
    }
    let workspace_bytes = [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .take(workspace.info().pixel_format().num_planes())
        .try_fold(0usize, |total, plane| {
            total.checked_add(workspace.plane(plane).ok()?.allocation_bytes())
        });
    let Some(workspace_bytes) = workspace_bytes.filter(|&bytes| bytes != 0) else {
        return 1;
    };
    let Some(charged_worker_bytes) =
        workspace_bytes.checked_mul(PARALLEL_TILE_SCRATCH_MEMORY_MULTIPLIER)
    else {
        return 1;
    };
    let memory_workers = limits
        .max_decoded_frame_bytes()
        .max_value()
        .and_then(|bytes| usize::try_from(bytes).ok())
        .map_or(tile_count, |bytes| bytes / charged_worker_bytes);
    tile_count
        .min(splot_parallel::current_pool_width())
        .min(memory_workers)
        .max(1)
}

fn merge_decoded_chunk<T: ReconSample>(
    output: &mut TileDecodeOutput,
    workspace: &mut CurrentFrameWorkspace<T>,
    tiles: &[DecodeTileWorkUnit<'_>],
    decoded: DecodedTileChunk<T>,
) -> Result<()> {
    for tile in tiles {
        let tile_offset = tile.tile_byte_span().start;
        let mi_rows = tile.mi_row_range().start as usize..tile.mi_row_range().end as usize;
        let mi_cols = tile.mi_col_range().start as usize..tile.mi_col_range().end as usize;
        output.cdef_state.merge_tile_from(
            &decoded.output.cdef_state,
            mi_rows.clone(),
            mi_cols.clone(),
            tile_offset,
        )?;
        output.gdf_state.merge_tile_from(
            &decoded.output.gdf_state,
            mi_rows.clone(),
            mi_cols.clone(),
            tile_offset,
        )?;
        output.ccso_state.merge_tile_from(
            &decoded.output.ccso_state,
            mi_rows.clone(),
            mi_cols.clone(),
            tile_offset,
        )?;
        output
            .motion_field
            .merge_tile_from(&decoded.output.motion_field, mi_rows, mi_cols);
        merge_tile_workspace(workspace, &decoded.workspace, tile)?;
    }

    let TileDecodeOutput {
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        active_source_blocks,
        unit_filters,
        ..
    } = decoded.output;
    output.deblock_blocks.extend(deblock_blocks);
    for (target, source) in output
        .chroma_deblock_blocks
        .iter_mut()
        .zip(chroma_deblock_blocks)
    {
        target.extend(source);
    }
    output.tx_skip_records.extend(tx_skip_records);
    output.active_source_blocks.extend(active_source_blocks);
    output.unit_filters.extend(unit_filters);
    Ok(())
}

fn merge_tile_workspace<T: ReconSample>(
    target: &mut CurrentFrameWorkspace<T>,
    source: &CurrentFrameWorkspace<T>,
    tile: &DecodeTileWorkUnit<'_>,
) -> Result<()> {
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .take(source.info().pixel_format().num_planes())
    {
        let rect = tile_plane_rect(source, tile, plane)?;
        let source_plane = source.plane(plane)?;
        let stride = source_plane.stride_samples();
        let start = rect
            .y()
            .checked_mul(stride)
            .and_then(|row| row.checked_add(rect.x()))
            .ok_or(splot_recon::ReconError::ArithmeticOverflow {
                context: "parallel tile workspace source offset",
            })?;
        let samples = source_plane.samples().get(start..).ok_or(
            splot_recon::ReconError::WorkspaceRectOutOfBounds {
                plane,
                storage: source_plane.storage_size(),
                rect,
            },
        )?;
        target.write_rect(plane, rect, samples, stride)?;
    }
    Ok(())
}

fn tile_plane_rect<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    tile: &DecodeTileWorkUnit<'_>,
    plane: PlaneId,
) -> Result<PlaneRect> {
    let luma_size = workspace.plane(PlaneId::Y)?.storage_size();
    let luma_x = (tile.mi_col_range().start as usize * MI_SIZE).min(luma_size.width());
    let luma_y = (tile.mi_row_range().start as usize * MI_SIZE).min(luma_size.height());
    let luma_end_x = (tile.mi_col_range().end as usize * MI_SIZE).min(luma_size.width());
    let luma_end_y = (tile.mi_row_range().end as usize * MI_SIZE).min(luma_size.height());
    let (sub_x, sub_y) = if plane == PlaneId::Y {
        (0, 0)
    } else {
        let format = workspace.info().pixel_format();
        (format.subsampling_x(), format.subsampling_y())
    };
    let scale_x = 1usize << sub_x;
    let scale_y = 1usize << sub_y;
    let plane_size = workspace.plane(plane)?.storage_size();
    let x = (luma_x / scale_x).min(plane_size.width());
    let y = (luma_y / scale_y).min(plane_size.height());
    let end_x = luma_end_x.div_ceil(scale_x).min(plane_size.width());
    let end_y = luma_end_y.div_ceil(scale_y).min(plane_size.height());
    Ok(PlaneRect::new(x, y, end_x - x, end_y - y)?)
}

#[allow(clippy::too_many_arguments)]
fn decode_tile_chunk<T: ReconSample>(
    work_units: &mut [DecodeTileWorkUnit<'_>],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    limits: crate::DecodeLimits,
    chroma_smooth_rows: usize,
    chroma_smooth_cols: usize,
    mi_rows: usize,
    mi_cols: usize,
    sb_h4: usize,
    max_drl_bits_minus_1: u32,
    frame_interpolation_filter: FrameInterpolationFilter,
    residual_tool_policy: TransformToolResidualPolicy,
    num_total_refs: usize,
    reference_select: bool,
    num_same_ref_compound: u8,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<'_, T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    ref_frame_idx: &[u32],
    bit_depth: BitDepth,
    enable_adaptive_mvd: bool,
    allow_bawp: bool,
    allow_warpmv_mode: bool,
    frame_is_switch: bool,
    current_order_hint: u32,
    mut cdef_state: CdefState,
    mut gdf_state: GdfState,
    mut ccso_state: CcsoState,
    mut motion_field: TemporalMotionField,
) -> Result<TileDecodeOutput> {
    let chroma = sequence.general.chroma_format_idc;
    let mut deblock_blocks = Vec::new();
    let mut chroma_deblock_blocks = [Vec::new(), Vec::new()];
    let mut tx_skip_records = Vec::new();
    let (mut active_source_blocks, mut unit_filters) = (Vec::new(), Vec::new());
    let mut decoded_any = false;
    let chunk_offset = work_units
        .first()
        .map_or(ByteOffset::new(0), |tile| tile.tile_byte_span().start);

    for tile in work_units.iter_mut() {
        let tile_offset = tile.tile_byte_span().start;
        let mut coeff_ctx =
            TileCoeffContextState::new_chroma(mi_rows, mi_cols, chroma).map_err(|_| {
                inter_cap!(
                    "inter_coeff_context_state",
                    tile_offset,
                    "inter.residual_context_state",
                    SPEC_MODE_INFO
                )
            })?;
        let mut delta_q_state = DeltaQState::new(sequence, core, tile_offset)?;
        let mut intrabc_state = TileIntrabcPreludeState::new(
            mi_rows,
            mi_cols,
            sequence,
            core.frame_is_intra == Some(true),
            tile_offset,
        )?;
        let mut segment_id_state = TileSegmentIdState::new(mi_rows, mi_cols).map_err(|_| {
            inter_missing!(
                "inter_segment_id_grid",
                tile_offset,
                "inter.segment_id_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut mv_grid = NeighbourMvGrid::new(mi_rows, mi_cols).ok_or_else(|| {
            inter_cap!(
                "inter_mv_grid",
                tile_offset,
                "inter.mv_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut y_smooth = crate::prediction::intra_edge::TileYSmoothGrid::new(mi_rows, mi_cols)
            .ok_or_else(|| {
                inter_cap!(
                    "inter_y_smooth_grid",
                    tile_offset,
                    "inter.y_smooth_grid",
                    SPEC_MODE_INFO
                )
            })?;
        let mut chroma_smooth = crate::prediction::intra_edge::TileChromaSmoothGrid::new(
            chroma_smooth_rows,
            chroma_smooth_cols,
        )
        .ok_or_else(|| {
            inter_cap!(
                "inter_chroma_smooth_grid",
                tile_offset,
                "inter.chroma_smooth_grid",
                SPEC_MODE_INFO
            )
        })?;
        let mut deferred = deferred_recon::DeferredInterRecon::new();
        let mut ref_mv_bank = sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_refmvbank)
            .then(super::super::find_mv_stack::RefMvBank::new);
        let mut warp_param_bank = super::super::find_mv_stack::WarpParamBank::new();
        let walk = decode_general_intra_multiblock_tree_with_lr_source_blocks(
            tile,
            sequence,
            core,
            limits,
            |work_unit,
             symbols,
             frontier,
             joint_modes,
             uses_mrls,
             use_dip,
             fsc_modes,
             palette_state,
             is_cfl_ctx,
             block_decoded| {
                let leaf = decode_block(
                    work_unit,
                    symbols,
                    frontier,
                    sequence,
                    core,
                    &mut coeff_ctx,
                    &mut gdf_state,
                    &mut cdef_state,
                    &mut ccso_state,
                    &mut delta_q_state,
                    &mut intrabc_state,
                    &mut segment_id_state,
                    &mut mv_grid,
                    temporal_context,
                    &mut motion_field,
                    &mut y_smooth,
                    &mut chroma_smooth,
                    &mut ref_mv_bank,
                    &mut warp_param_bank,
                    sb_h4,
                    mi_rows,
                    mi_cols,
                    max_drl_bits_minus_1,
                    frame_interpolation_filter,
                    residual_tool_policy,
                    num_total_refs,
                    reference_select,
                    num_same_ref_compound,
                    joint_modes,
                    uses_mrls,
                    use_dip,
                    fsc_modes,
                    palette_state,
                    is_cfl_ctx,
                    block_decoded,
                    &mut deferred,
                    workspace,
                    &mut deblock_blocks,
                    &mut chroma_deblock_blocks,
                    &mut tx_skip_records,
                    luma_use_tcq,
                    residual_use_ddt,
                    ref_frame_idx,
                    reference,
                    bit_depth,
                    enable_adaptive_mvd,
                    allow_bawp,
                    allow_warpmv_mode,
                    frame_is_switch,
                    current_order_hint,
                    tile_offset,
                )?;
                decoded_any = true;
                Ok(leaf)
            },
        )
        .map_err(|error| map_inter_multiblock_error(error, tile_offset))?;
        deferred_recon::flush_deferred(
            &mut deferred,
            workspace,
            &mut motion_field,
            Some(temporal_context),
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
        )?;
        let crate::bitstream::tile_payload::GeneralIntraMultiblockOutput {
            symbols,
            active_source_blocks: tile_source_blocks,
            unit_filters: tile_unit_filters,
        } = walk;
        symbols.exit_symbol().map_err(|_| {
            if reference_select {
                compound_cap!(
                    "compound_exit_symbol",
                    tile_offset,
                    "inter.compound.exit_symbol",
                    SPEC_MODE_INFO
                )
            } else {
                inter_cap!(
                    "inter_exit_symbol",
                    tile_offset,
                    "inter.exit_symbol",
                    SPEC_MODE_INFO
                )
            }
        })?;
        active_source_blocks.extend(tile_source_blocks);
        unit_filters.extend(tile_unit_filters);
    }
    if !decoded_any {
        return Err(inter_missing!(
            "inter_no_decoded_block",
            chunk_offset,
            "inter.block",
            SPEC_MODE_INFO
        ));
    }

    Ok(TileDecodeOutput {
        cdef_state,
        gdf_state,
        ccso_state,
        motion_field,
        deblock_blocks,
        chroma_deblock_blocks,
        tx_skip_records,
        active_source_blocks,
        unit_filters,
    })
}
