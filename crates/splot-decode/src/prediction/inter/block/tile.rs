// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Tile-local block decode and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use super::*;

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

#[allow(clippy::too_many_arguments)]
pub(super) fn decode_tiles<T: DeferredReconSample>(
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
