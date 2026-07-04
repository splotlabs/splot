// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.23 inter residual reads: transform partitioning, coefficient
//! decode, and skipped-block coefficient-context resets.

#[allow(clippy::wildcard_imports)]
use super::*;

const INTER_UV_MODE_DC: usize = 0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    mi_rows: usize,
    mi_cols: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<InterResidual> {
    let (subsampling_x, subsampling_y) = chroma_subsampling(sequence.general.chroma_format_idc);
    let width_chunks = (n4w >> 4).max(1);
    let height_chunks = (n4h >> 4).max(1);
    let multi_chunk = width_chunks > 1 || height_chunks > 1;
    let mi_size_chunk = if multi_chunk {
        BlockSize::new(BLOCK_64X64).map_err(|_| residual_geometry_error(tile_offset))?
    } else {
        frontier.b_size
    };
    let luma_tx_size = max_tx_size(frontier.b_size.index(), tile_offset)?;
    let luma_tx_records = if inter_uses_selectable_tx_partitions(core) {
        Some(derive_inter_luma_tx_records_for_block(
            work_unit,
            symbols,
            frontier,
            (mi_rows, mi_cols),
            0,
            tile_offset,
        )?)
    } else {
        None
    };
    let mut blocks = Vec::new();

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    let at_start = (!subsampling_x || chunk_x % 2 == 0)
                        && (!subsampling_y || chunk_y % 2 == 0);
                    let luma_chunk_x = frontier.c + (chunk_x << 4);
                    let luma_chunk_y = frontier.r + (chunk_y << 4);
                    read_inter_residual_luma_chunk(
                        work_unit,
                        symbols,
                        coeff_ctx,
                        &mut blocks,
                        luma_tx_records.as_deref(),
                        luma_tx_size,
                        frontier,
                        luma_chunk_x,
                        luma_chunk_y,
                        n4w,
                        n4h,
                        residual_tool_policy,
                        tile_offset,
                    )?;
                    if frontier.has_chroma && at_start {
                        read_inter_residual_chroma_group(
                            work_unit,
                            symbols,
                            coeff_ctx,
                            &mut blocks,
                            frontier,
                            mi_size_chunk,
                            subsampling_x,
                            subsampling_y,
                            chunk_x,
                            chunk_y,
                            residual_tool_policy,
                            tile_offset,
                        )?;
                    }
                }
            }
        }
    }

    Ok(InterResidual { blocks })
}

fn inter_uses_selectable_tx_partitions(core: &FrameHeaderCore) -> bool {
    core.inter_tail
        .as_ref()
        .is_some_and(|tail| tail.tx_mode == TxMode::Select)
        && core
            .lossless_info
            .as_ref()
            .is_some_and(|lossless| !lossless.lossless_array[0])
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_luma_chunk(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    luma_tx_records: Option<&[SelectableLumaTxRecord]>,
    tx_size: usize,
    frontier: &DecodeBlockFrontier,
    luma_chunk_x4: usize,
    luma_chunk_y4: usize,
    block_n4w: usize,
    block_n4h: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chunk_end_x4 = (frontier.c + block_n4w).min(luma_chunk_x4 + CHUNK_64_N4);
    let chunk_end_y4 = (frontier.r + block_n4h).min(luma_chunk_y4 + CHUNK_64_N4);
    if let Some(luma_tx_records) = luma_tx_records {
        return read_inter_residual_luma_records_for_chunk(
            work_unit,
            symbols,
            coeff_ctx,
            blocks,
            luma_tx_records,
            frontier,
            luma_chunk_x4,
            luma_chunk_y4,
            chunk_end_x4,
            chunk_end_y4,
            block_n4w,
            block_n4h,
            residual_tool_policy,
            tile_offset,
        );
    }
    let tx_w4 = tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
    let mut y4 = luma_chunk_y4;
    while y4 < chunk_end_y4 {
        let mut x4 = luma_chunk_x4;
        while x4 < chunk_end_x4 {
            let tx_fills_block = tx_w4 == block_n4w && tx_h4 == block_n4h;
            if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS") {
                let start_x = x4 * MI_SIZE;
                let start_y = y4 * MI_SIZE;
                if start_x >= 1760 && (160..=224).contains(&start_y) {
                    eprintln!(
                        "inter residual luma read r={} c={} b={} n4={}x{} chunk=({}, {}) start=({start_x},{start_y}) tx_size={tx_size} tx4={}x{} fills={tx_fills_block} has_chroma={} chroma_offset={} luma_part={} chroma_part={}",
                        frontier.r,
                        frontier.c,
                        frontier.b_size.index(),
                        block_n4w,
                        block_n4h,
                        luma_chunk_x4,
                        luma_chunk_y4,
                        tx_w4,
                        tx_h4,
                        frontier.has_chroma,
                        frontier.chroma_offset,
                        frontier.is_luma_part(),
                        frontier.is_chroma_part()
                    );
                }
            }
            let coeffs = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                0,
                tx_size,
                x4 * MI_SIZE,
                y4 * MI_SIZE,
                tx_fills_block,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            push_inter_residual_block(
                blocks,
                ReconPlaneId::Y,
                x4,
                y4,
                tx_size,
                coeffs,
                tile_offset,
            )?;
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_luma_records_for_chunk(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    luma_tx_records: &[SelectableLumaTxRecord],
    frontier: &DecodeBlockFrontier,
    luma_chunk_x4: usize,
    luma_chunk_y4: usize,
    chunk_end_x4: usize,
    chunk_end_y4: usize,
    block_n4w: usize,
    block_n4h: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let mut decoded_any = false;
    for record in luma_tx_records.iter().copied().filter(|record| {
        record.row >= luma_chunk_y4
            && record.col >= luma_chunk_x4
            && record.row < chunk_end_y4
            && record.col < chunk_end_x4
    }) {
        decoded_any = true;
        let tx_fills_block = record.cols == block_n4w && record.rows == block_n4h;
        let trace_residual = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS");
        let start_x = record.col * MI_SIZE;
        let start_y = record.row * MI_SIZE;
        let trace_this = trace_residual
            && ((512..=640).contains(&start_x) && start_y <= 128
                || (start_x >= 1760 && (160..=224).contains(&start_y)));
        if trace_this {
            eprintln!(
                "inter residual luma read start r={} c={} b={} n4={}x{} chunk=({}, {}) start=({start_x},{start_y}) tx_size={} tx4={}x{} fills={tx_fills_block} has_chroma={} chroma_offset={} luma_part={} chroma_part={} checkpoint={:?}",
                frontier.r,
                frontier.c,
                frontier.b_size.index(),
                block_n4w,
                block_n4h,
                luma_chunk_x4,
                luma_chunk_y4,
                record.tx_size,
                record.cols,
                record.rows,
                frontier.has_chroma,
                frontier.chroma_offset,
                frontier.is_luma_part(),
                frontier.is_chroma_part(),
                symbols.checkpoint()
            );
        }
        let coeffs = read_inter_residual_plane(
            work_unit,
            symbols,
            coeff_ctx,
            0,
            record.tx_size,
            record.col * MI_SIZE,
            record.row * MI_SIZE,
            tx_fills_block,
            false,
            residual_tool_policy,
            tile_offset,
        )?;
        if trace_this {
            eprintln!(
                "inter residual luma read done start=({start_x},{start_y}) tx_size={} all_zero={} eob={} checkpoint={:?}",
                record.tx_size,
                coeffs.all_zero,
                coeffs.eob,
                symbols.checkpoint()
            );
        }
        push_inter_residual_block(
            blocks,
            ReconPlaneId::Y,
            record.col,
            record.row,
            record.tx_size,
            coeffs,
            tile_offset,
        )?;
    }
    if decoded_any {
        Ok(())
    } else {
        Err(residual_geometry_error(tile_offset))
    }
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_chroma_group(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    blocks: &mut Vec<InterResidualBlock>,
    frontier: &DecodeBlockFrontier,
    mi_size_chunk: BlockSize,
    subsampling_x: bool,
    subsampling_y: bool,
    chunk_x: usize,
    chunk_y: usize,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chroma_ref = frontier.chroma_ref_geometry();
    let chroma_mi_size = chroma_ref.size();
    let chroma_ref_size = get_plane_residual_size(chroma_mi_size, 1, subsampling_x, subsampling_y)
        .map_err(|_| residual_geometry_error(tile_offset))?
        .valid()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let tx_size = max_tx_size(chroma_ref_size.index(), tile_offset)?;
    let plane_source_size = if chroma_mi_size != frontier.b_size {
        chroma_mi_size
    } else {
        mi_size_chunk
    };
    let plane_size = get_plane_residual_size(plane_source_size, 1, subsampling_x, subsampling_y)
        .map_err(|_| residual_geometry_error(tile_offset))?
        .valid()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let block_width_chunks = (frontier
        .b_size
        .num_4x4_wide()
        .map_err(|_| residual_geometry_error(tile_offset))?
        >> 4)
        .max(1);
    let block_height_chunks = (frontier
        .b_size
        .num_4x4_high()
        .map_err(|_| residual_geometry_error(tile_offset))?
        >> 4)
        .max(1);
    let mut num4x4_w = plane_size
        .num_4x4_wide()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    let mut num4x4_h = plane_size
        .num_4x4_high()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    if subsampling_x && chunk_x + 1 < block_width_chunks {
        num4x4_w <<= 1;
    }
    if subsampling_y && chunk_y + 1 < block_height_chunks {
        num4x4_h <<= 1;
    }
    let tx_w4 = tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
    let x_offset4 = (chunk_x << 4) >> usize::from(subsampling_x);
    let y_offset4 = (chunk_y << 4) >> usize::from(subsampling_y);
    let base_x4 = chroma_ref.col() >> usize::from(subsampling_x);
    let base_y4 = chroma_ref.row() >> usize::from(subsampling_y);
    let mut y4 = y_offset4;
    while y4 < y_offset4 + num4x4_h {
        let mut x4 = x_offset4;
        while x4 < x_offset4 + num4x4_w {
            let start_x = (base_x4 + x4) * MI_SIZE;
            let start_y = (base_y4 + y4) * MI_SIZE;
            let trace_this = crate::trace_flags::trace_flag!("SPLOT_TRACE_INTER_RESIDUAL_BLOCKS")
                && ((256..=320).contains(&start_x) && start_y <= 64
                    || (start_x >= 880 && (80..=112).contains(&start_y)));
            if trace_this {
                eprintln!(
                    "inter residual chroma read start block=({},{} b={}) local=({}, {}) start=({start_x},{start_y}) tx_size={tx_size} tx4={}x{} plane_size={}x{} checkpoint={:?}",
                    frontier.r,
                    frontier.c,
                    frontier.b_size.index(),
                    x4,
                    y4,
                    tx_w4,
                    tx_h4,
                    num4x4_w,
                    num4x4_h,
                    symbols.checkpoint()
                );
            }
            let u = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                1,
                tx_size,
                start_x,
                start_y,
                tx_w4 == num4x4_w && tx_h4 == num4x4_h,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            let u_nonzero = !u.all_zero;
            if trace_this {
                eprintln!(
                    "inter residual chroma read u done start=({start_x},{start_y}) all_zero={} eob={} checkpoint={:?}",
                    u.all_zero,
                    u.eob,
                    symbols.checkpoint()
                );
            }
            push_inter_residual_block(
                blocks,
                ReconPlaneId::U,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                u,
                tile_offset,
            )?;
            let v = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                2,
                tx_size,
                start_x,
                start_y,
                tx_w4 == num4x4_w && tx_h4 == num4x4_h,
                u_nonzero,
                residual_tool_policy,
                tile_offset,
            )?;
            if trace_this {
                eprintln!(
                    "inter residual chroma read v done start=({start_x},{start_y}) all_zero={} eob={} checkpoint={:?}",
                    v.all_zero,
                    v.eob,
                    symbols.checkpoint()
                );
            }
            push_inter_residual_block(
                blocks,
                ReconPlaneId::V,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                v,
                tile_offset,
            )?;
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

fn push_inter_residual_block(
    blocks: &mut Vec<InterResidualBlock>,
    plane: ReconPlaneId,
    x4: usize,
    y4: usize,
    tx_size: usize,
    coeffs: LumaCoeffBlock,
    tile_offset: ByteOffset,
) -> Result<()> {
    let log2_width = tx_size_dimension("Tx_Width_Log2", &TX_WIDTH_LOG2, tx_size, tile_offset)?;
    let log2_height = tx_size_dimension("Tx_Height_Log2", &TX_HEIGHT_LOG2, tx_size, tile_offset)?;
    let log2_width = u32::try_from(log2_width).map_err(|_| residual_geometry_error(tile_offset))?;
    let log2_height =
        u32::try_from(log2_height).map_err(|_| residual_geometry_error(tile_offset))?;
    blocks.try_reserve(1).map_err(|_| {
        inter_cap!(
            "inter_block_residual_allocation",
            tile_offset,
            "inter.residual.transform_block_list",
            SPEC_MODE_INFO
        )
    })?;
    blocks.push(InterResidualBlock {
        plane,
        x: x4 * MI_SIZE,
        y: y4 * MI_SIZE,
        tx_size,
        log2_width,
        log2_height,
        coeffs,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_inter_residual_plane(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    plane: usize,
    tx_size: usize,
    start_x: usize,
    start_y: usize,
    tx_fills_block: bool,
    chroma_eob_ctx: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<LumaCoeffBlock> {
    decode_general_intra_plane_coeffs(
        work_unit,
        symbols,
        coeff_ctx,
        plane,
        tx_size,
        start_x,
        start_y,
        tx_fills_block,
        None,
        chroma_eob_ctx,
        INTER_UV_MODE_DC,
        0,
        true,
        false,
        false,
        residual_tool_policy,
    )
    .map_err(|error| {
        if crate::trace_flags::trace_flag!("SPLOT_TRACE_INTER_RESIDUAL_ERROR") {
            eprintln!(
                "inter residual error offset={} plane={plane} tx_size={tx_size} start=({start_x},{start_y}) fills={tx_fills_block}: {error:?}",
                tile_offset.get(),
            );
        }
        residual_read_error(tile_offset)
    })
}

pub(crate) fn transform_tool_residual_policy(
    sequence: &SequenceHeader,
) -> TransformToolResidualPolicy {
    TransformToolResidualPolicy::from_sequence_tools(
        sequence,
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
    )
}

pub(crate) fn max_tx_size(block_size: usize, tile_offset: ByteOffset) -> Result<usize> {
    table_value_usize(
        "Max_Tx_Size_Rect",
        &MAX_TX_SIZE_RECT,
        block_size,
        tile_offset,
    )
}

pub(crate) fn tx_size_dimension(
    table: &'static str,
    values: &[i32],
    tx_size: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    table_value_usize(table, values, tx_size, tile_offset)
}

fn table_value_usize(
    _table: &'static str,
    values: &[i32],
    index: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    let value = values
        .get(index)
        .copied()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    usize::try_from(value).map_err(|_| residual_geometry_error(tile_offset))
}

fn residual_geometry_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_cap!(
        "inter_block_residual_geometry",
        tile_offset,
        "inter.residual.transform_geometry",
        SPEC_MODE_INFO
    )
}

pub(crate) fn reset_inter_skip_coeff_contexts(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let plane_count = 1 + usize::from(frontier.has_chroma) * (COEFF_CONTEXT_PLANES.len() - 1);
    for &(plane, sub) in COEFF_CONTEXT_PLANES.iter().take(plane_count) {
        coeff_ctx
            .reset_block_context_plane(CoeffContextReset {
                plane,
                c: frontier.c,
                r: frontier.r,
                w4: n4w,
                h4: n4h,
                sub_x: sub,
                sub_y: sub,
            })
            .map_err(|_| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

fn residual_read_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_missing!(
        "inter_block_residual_parse",
        tile_offset,
        "inter.residual.coefficients",
        SPEC_MODE_INFO
    )
}
