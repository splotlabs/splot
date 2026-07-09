// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.23 inter residual reads: transform partitioning, coefficient
//! decode, and skipped-block coefficient-context resets.

#[allow(clippy::wildcard_imports)]
use super::*;

const INTER_UV_MODE_DC: usize = 0;
const DCT_DCT: usize = 0;
const TX_4X4: usize = 0;
#[cfg(test)]
const V_DCT: usize = 10;
const TX_TYPE_MAP_UNIT_4X4: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
struct InterLumaTxTypeMap {
    row: usize,
    col: usize,
    rows: usize,
    cols: usize,
    values: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InterChromaUnit {
    x4: usize,
    y4: usize,
    tx_fills_block: bool,
    chroma_inter_tx_type: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InterChromaURead {
    unit: InterChromaUnit,
    u_nonzero: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterResidualLumaTxSizeMode {
    Inter,
    Intrabc,
}

impl InterLumaTxTypeMap {
    fn new(
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let len = rows
            .checked_mul(cols)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
        let mut values = Vec::new();
        values
            .try_reserve(len)
            .map_err(|_| residual_geometry_error(tile_offset))?;
        values.resize(len, DCT_DCT);
        Ok(Self {
            row,
            col,
            rows,
            cols,
            values,
        })
    }

    fn update(
        &mut self,
        row: usize,
        col: usize,
        tx_size: usize,
        tx_type: usize,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let tx_w4 = tx_size_dimension("Tx_Width", &TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
        let tx_h4 = tx_size_dimension("Tx_Height", &TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
        for dy in (0..tx_h4).step_by(TX_TYPE_MAP_UNIT_4X4) {
            for dx in (0..tx_w4).step_by(TX_TYPE_MAP_UNIT_4X4) {
                let map_row = row
                    .checked_add(dy)
                    .ok_or_else(|| residual_geometry_error(tile_offset))?;
                let map_col = col
                    .checked_add(dx)
                    .ok_or_else(|| residual_geometry_error(tile_offset))?;
                if let Some(index) = self.index(map_row, map_col) {
                    self.values[index] = tx_type;
                }
            }
        }
        Ok(())
    }

    fn chroma_inter_tx_type(
        &self,
        mi_row: usize,
        mi_col: usize,
        chroma_row: usize,
        chroma_col: usize,
        subsampling_x: bool,
        subsampling_y: bool,
    ) -> usize {
        let luma_row = mi_row.max(chroma_row << usize::from(subsampling_y));
        let luma_col = mi_col.max(chroma_col << usize::from(subsampling_x));
        self.index(luma_row, luma_col)
            .map_or(DCT_DCT, |index| self.values[index])
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        let rel_row = row.checked_sub(self.row)?;
        let rel_col = col.checked_sub(self.col)?;
        if rel_row >= self.rows || rel_col >= self.cols {
            return None;
        }
        rel_row
            .checked_mul(self.cols)
            .and_then(|start| start.checked_add(rel_col))
    }
}

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
    lossless: bool,
    luma_tx_size_mode: InterResidualLumaTxSizeMode,
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
    let luma_tx_size = inter_residual_tx_size(
        work_unit,
        symbols,
        frontier.b_size.index(),
        lossless,
        luma_tx_size_mode,
        tile_offset,
    )?;
    let luma_tx_records = if residual_uses_selectable_tx_partitions(core) {
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
    let mut luma_tx_types = InterLumaTxTypeMap::new(frontier.r, frontier.c, n4h, n4w, tile_offset)?;

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            let group_end_y = (start_chunk_y + 2).min(height_chunks);
            let group_end_x = (start_chunk_x + 2).min(width_chunks);
            for chunk_y in start_chunk_y..group_end_y {
                for chunk_x in start_chunk_x..group_end_x {
                    let luma_chunk_x = frontier.c + (chunk_x << 4);
                    let luma_chunk_y = frontier.r + (chunk_y << 4);
                    read_inter_residual_luma_chunk(
                        work_unit,
                        symbols,
                        coeff_ctx,
                        &mut blocks,
                        &mut luma_tx_types,
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
                    let chroma_group = completed_chroma_group_start(
                        chunk_x,
                        chunk_y,
                        width_chunks,
                        height_chunks,
                        subsampling_x,
                        subsampling_y,
                    );
                    if let (true, Some((chroma_chunk_x, chroma_chunk_y))) =
                        (frontier.has_chroma, chroma_group)
                    {
                        read_inter_residual_chroma_group(
                            work_unit,
                            symbols,
                            coeff_ctx,
                            &mut blocks,
                            &luma_tx_types,
                            frontier,
                            mi_size_chunk,
                            subsampling_x,
                            subsampling_y,
                            chroma_chunk_x,
                            chroma_chunk_y,
                            lossless,
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

fn completed_chroma_group_start(
    chunk_x: usize,
    chunk_y: usize,
    width_chunks: usize,
    height_chunks: usize,
    subsampling_x: bool,
    subsampling_y: bool,
) -> Option<(usize, usize)> {
    let start_x = if subsampling_x { chunk_x & !1 } else { chunk_x };
    let start_y = if subsampling_y { chunk_y & !1 } else { chunk_y };
    let end_x = if subsampling_x {
        start_x.checked_add(2)?.min(width_chunks)
    } else {
        start_x.checked_add(1)?
    };
    let end_y = if subsampling_y {
        start_y.checked_add(2)?.min(height_chunks)
    } else {
        start_y.checked_add(1)?
    };
    let next_x = chunk_x.checked_add(1)?;
    let next_y = chunk_y.checked_add(1)?;
    (next_x == end_x && next_y == end_y).then_some((start_x, start_y))
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

fn residual_uses_selectable_tx_partitions(core: &FrameHeaderCore) -> bool {
    inter_uses_selectable_tx_partitions(core) || intra_frame_needs_selectable_tx_partitions(core)
}

fn intra_frame_needs_selectable_tx_partitions(core: &FrameHeaderCore) -> bool {
    core.inter_tail.is_none()
        && core
            .intra_tail
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
    luma_tx_types: &mut InterLumaTxTypeMap,
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
            luma_tx_types,
            luma_tx_records,
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
            let coeffs = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                0,
                tx_size,
                x4 * MI_SIZE,
                y4 * MI_SIZE,
                tx_fills_block,
                DCT_DCT,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            luma_tx_types.update(y4, x4, tx_size, coeffs.plane_tx_type, tile_offset)?;
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
    luma_tx_types: &mut InterLumaTxTypeMap,
    luma_tx_records: &[SelectableLumaTxRecord],
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
        let coeffs = read_inter_residual_plane(
            work_unit,
            symbols,
            coeff_ctx,
            0,
            record.tx_size,
            record.col * MI_SIZE,
            record.row * MI_SIZE,
            tx_fills_block,
            DCT_DCT,
            false,
            residual_tool_policy,
            tile_offset,
        )?;
        luma_tx_types.update(
            record.row,
            record.col,
            record.tx_size,
            coeffs.plane_tx_type,
            tile_offset,
        )?;
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
    luma_tx_types: &InterLumaTxTypeMap,
    frontier: &DecodeBlockFrontier,
    mi_size_chunk: BlockSize,
    subsampling_x: bool,
    subsampling_y: bool,
    chunk_x: usize,
    chunk_y: usize,
    lossless: bool,
    residual_tool_policy: TransformToolResidualPolicy,
    tile_offset: ByteOffset,
) -> Result<()> {
    let chroma_ref = frontier.chroma_ref_geometry();
    let chroma_mi_size = chroma_ref.size();
    let chroma_ref_size = get_plane_residual_size(chroma_mi_size, 1, subsampling_x, subsampling_y)
        .map_err(|_| residual_geometry_error(tile_offset))?
        .valid()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let tx_size = fixed_inter_residual_tx_size(chroma_ref_size.index(), lossless, tile_offset)?;
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
    let mut u_reads = Vec::new();
    let mut y4 = y_offset4;
    while y4 < y_offset4 + num4x4_h {
        let mut x4 = x_offset4;
        while x4 < x_offset4 + num4x4_w {
            let start_x = (base_x4 + x4) * MI_SIZE;
            let start_y = (base_y4 + y4) * MI_SIZE;
            let chroma_inter_tx_type = luma_tx_types.chroma_inter_tx_type(
                frontier.r,
                frontier.c,
                base_y4 + y4,
                base_x4 + x4,
                subsampling_x,
                subsampling_y,
            );
            let unit = InterChromaUnit {
                x4,
                y4,
                tx_fills_block: tx_w4 == num4x4_w && tx_h4 == num4x4_h,
                chroma_inter_tx_type,
            };
            let u = read_inter_residual_plane(
                work_unit,
                symbols,
                coeff_ctx,
                1,
                tx_size,
                start_x,
                start_y,
                unit.tx_fills_block,
                unit.chroma_inter_tx_type,
                false,
                residual_tool_policy,
                tile_offset,
            )?;
            let u_nonzero = !u.all_zero;
            push_inter_residual_block(
                blocks,
                ReconPlaneId::U,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                u,
                tile_offset,
            )?;
            u_reads
                .try_reserve(1)
                .map_err(|_| residual_geometry_error(tile_offset))?;
            u_reads.push(InterChromaURead { unit, u_nonzero });
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }

    for read in u_reads {
        let start_x = (base_x4 + read.unit.x4) * MI_SIZE;
        let start_y = (base_y4 + read.unit.y4) * MI_SIZE;
        let v = read_inter_residual_plane(
            work_unit,
            symbols,
            coeff_ctx,
            2,
            tx_size,
            start_x,
            start_y,
            read.unit.tx_fills_block,
            read.unit.chroma_inter_tx_type,
            read.u_nonzero,
            residual_tool_policy,
            tile_offset,
        )?;
        push_inter_residual_block(
            blocks,
            ReconPlaneId::V,
            base_x4 + read.unit.x4,
            base_y4 + read.unit.y4,
            tx_size,
            v,
            tile_offset,
        )?;
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
    chroma_inter_tx_type: usize,
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
        chroma_inter_tx_type,
        true,
        false,
        false,
        residual_tool_policy,
    )
    .map_err(|_| residual_read_error(tile_offset))
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

fn inter_residual_tx_size(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    block_size: usize,
    lossless: bool,
    mode: InterResidualLumaTxSizeMode,
    tile_offset: ByteOffset,
) -> Result<usize> {
    if !lossless {
        return max_tx_size(block_size, tile_offset);
    }
    match mode {
        InterResidualLumaTxSizeMode::Inter => Ok(TX_4X4),
        InterResidualLumaTxSizeMode::Intrabc => {
            read_lossless_tx_size(work_unit, symbols, block_size, false, true, true)
                .map_err(|_| residual_read_error(tile_offset))
        }
    }
}

fn fixed_inter_residual_tx_size(
    block_size: usize,
    lossless: bool,
    tile_offset: ByteOffset,
) -> Result<usize> {
    if lossless {
        Ok(TX_4X4)
    } else {
        max_tx_size(block_size, tile_offset)
    }
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

#[cfg(test)]
#[path = "residual_tests.rs"]
mod tests;
