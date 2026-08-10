// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! § 5.20.7.23 inter residual reads: transform partitioning, coefficient
//! decode, and skipped-block coefficient-context resets.
//!
//! Feature tracking: `DECODE-INTER-RESIDUAL-DCT`.

#[allow(clippy::wildcard_imports)]
use super::*;

const INTER_UV_MODE_DC: usize = 0;
const DCT_DCT: usize = 0;
const TX_4X4: usize = 0;
const SPEC_RESIDUAL: &str = "5.20.7.23";
const SPEC_READ_QUANT: &str = "5.20.7.28";
const SPEC_TX_SIZE: &str = "5.20.6.1";
const SPEC_TRANSFORM_TYPE: &str = "5.20.8.2";
#[cfg(test)]
const V_DCT: usize = 10;
const TX_TYPE_MAP_UNIT_4X4: usize = 4;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
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
    block_index: usize,
    uses_cctx: bool,
    u_nonzero: bool,
}

#[derive(Default)]
pub(crate) struct InterResidualParseScratch {
    luma_tx_types: InterLumaTxTypeMap,
    luma_tx_records: Vec<SelectableLumaTxRecord>,
    chroma_reads: Vec<InterChromaURead>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterResidualLumaTxSizeMode {
    Inter,
    Intrabc,
}

impl InterLumaTxTypeMap {
    #[cfg(test)]
    fn new(
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
        tile_offset: ByteOffset,
    ) -> Result<Self> {
        let mut map = Self::default();
        map.reset(row, col, rows, cols, tile_offset)?;
        Ok(map)
    }

    fn reset(
        &mut self,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let len = rows
            .checked_mul(cols)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
        self.values
            .try_reserve(len.saturating_sub(self.values.len()))
            .map_err(|_| inter_allocation!("inter residual luma transform-type map"))?;
        self.values.resize(len, DCT_DCT);
        self.values.fill(DCT_DCT);
        self.row = row;
        self.col = col;
        self.rows = rows;
        self.cols = cols;
        Ok(())
    }

    fn update(
        &mut self,
        row: usize,
        col: usize,
        tx_size: usize,
        tx_type: usize,
        tile_offset: ByteOffset,
    ) -> Result<()> {
        let tx_w4 = tx_size_dimension(&TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
        let tx_h4 = tx_size_dimension(&TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
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
        subsampling: (bool, bool),
        lossless_uses_current_block: bool,
    ) -> usize {
        let (subsampling_x, subsampling_y) = subsampling;
        let (luma_row, luma_col) = if lossless_uses_current_block {
            (mi_row, mi_col)
        } else {
            (
                mi_row.max(chroma_row << usize::from(subsampling_y)),
                mi_col.max(chroma_col << usize::from(subsampling_x)),
            )
        };
        self.index(luma_row, luma_col)
            .map_or(DCT_DCT, |index| self.values[index])
    }

    fn index(&self, row: usize, col: usize) -> Option<usize> {
        crate::tile::local_grid_index(row, col, self.row, self.col, self.rows, self.cols)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_inter_residual(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    scratch: &mut InterResidualParseScratch,
    blocks: &mut Vec<InterResidualBlock>,
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
    let _phase = crate::timing::PhaseScope::new(crate::timing::Phase::Coeff);
    let block_start = blocks.len();
    let result = (|| -> Result<()> {
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
            frontier.b_size,
            lossless,
            luma_tx_size_mode,
            tile_offset,
        )?;
        let selectable_luma_tx = inter_luma_tx_records_are_selectable(
            residual_uses_selectable_tx_partitions(core),
            lossless,
        );
        scratch.luma_tx_records.clear();
        if selectable_luma_tx {
            derive_inter_luma_tx_records_for_block(
                work_unit,
                symbols,
                frontier,
                (mi_rows, mi_cols),
                0,
                tile_offset,
                &mut scratch.luma_tx_records,
            )?;
        }
        let luma_tx_records = selectable_luma_tx.then_some(scratch.luma_tx_records.as_slice());
        scratch
            .luma_tx_types
            .reset(frontier.r, frontier.c, n4h, n4w, tile_offset)?;

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
                            blocks,
                            &mut scratch.luma_tx_types,
                            luma_tx_records,
                            luma_tx_size,
                            frontier,
                            luma_chunk_x,
                            luma_chunk_y,
                            n4w,
                            n4h,
                            residual_tool_policy,
                            tile_offset,
                        )?;
                        let chroma_group = chroma_parse_group_start(
                            chunk_x,
                            chunk_y,
                            width_chunks,
                            height_chunks,
                            subsampling_x,
                            subsampling_y,
                            lossless,
                        );
                        if let (true, Some((chroma_chunk_x, chroma_chunk_y))) =
                            (frontier.has_chroma, chroma_group)
                        {
                            read_inter_residual_chroma_group(
                                work_unit,
                                symbols,
                                coeff_ctx,
                                blocks,
                                &scratch.luma_tx_types,
                                &mut scratch.chroma_reads,
                                frontier,
                                mi_size_chunk,
                                subsampling_x,
                                subsampling_y,
                                chroma_chunk_x,
                                chroma_chunk_y,
                                lossless,
                                luma_tx_size_mode == InterResidualLumaTxSizeMode::Inter,
                                residual_tool_policy,
                                tile_offset,
                            )?;
                        }
                    }
                }
            }
        }

        Ok(())
    })();
    if let Err(error) = result {
        blocks.truncate(block_start);
        return Err(error);
    }
    Ok(InterResidual {
        block_range: block_start..blocks.len(),
    })
}

/// Returns the chroma group's top-left chunk position when `(chunk_x, chunk_y)`
/// is the first collocated luma 64x64 chunk of a chroma group, or `None` when
/// this chunk's chroma is folded into an earlier chunk's group.
///
/// AV2 § 5.20.7.23 `residual( )` parses each chroma transform unit once, at the
/// group's `atStart` chunk interleaved before the remaining luma chunks, where
/// `doubleChromaW/H = Subsampling && chunks > 1 && !Lossless`
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-23`). Emitting chroma
/// at `atEnd` instead desynchronizes the entropy coder for blocks taller or
/// wider than 64 luma samples.
fn chroma_parse_group_start(
    chunk_x: usize,
    chunk_y: usize,
    width_chunks: usize,
    height_chunks: usize,
    subsampling_x: bool,
    subsampling_y: bool,
    lossless: bool,
) -> Option<(usize, usize)> {
    let double_chroma_w = subsampling_x && width_chunks > 1 && !lossless;
    let double_chroma_h = subsampling_y && height_chunks > 1 && !lossless;
    let at_start = (!double_chroma_w || chunk_x.is_multiple_of(2))
        && (!double_chroma_h || chunk_y.is_multiple_of(2));
    at_start.then_some((chunk_x, chunk_y))
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

fn inter_luma_tx_records_are_selectable(selectable_tx_partitions: bool, lossless: bool) -> bool {
    selectable_tx_partitions && !lossless
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
    let tx_w4 = tx_size_dimension(&TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension(&TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
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
    chroma_reads: &mut Vec<InterChromaURead>,
    frontier: &DecodeBlockFrontier,
    mi_size_chunk: BlockSize,
    subsampling_x: bool,
    subsampling_y: bool,
    chunk_x: usize,
    chunk_y: usize,
    lossless: bool,
    is_inter: bool,
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
    let cctx_allowed = is_cctx_geometry_allowed(
        subsampling_x && subsampling_y,
        chroma_ref_size
            .num_4x4_wide()
            .map_err(|_| residual_geometry_error(tile_offset))?
            * MI_SIZE,
        chroma_ref_size
            .num_4x4_high()
            .map_err(|_| residual_geometry_error(tile_offset))?
            * MI_SIZE,
    );
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
    if subsampling_x && chunk_x + 1 < block_width_chunks && !lossless {
        num4x4_w <<= 1;
    }
    if subsampling_y && chunk_y + 1 < block_height_chunks && !lossless {
        num4x4_h <<= 1;
    }
    let tx_w4 = tx_size_dimension(&TX_WIDTH, tx_size, tile_offset)? / MI_SIZE;
    let tx_h4 = tx_size_dimension(&TX_HEIGHT, tx_size, tile_offset)? / MI_SIZE;
    let x_offset4 = (chunk_x << 4) >> usize::from(subsampling_x);
    let y_offset4 = (chunk_y << 4) >> usize::from(subsampling_y);
    let base_x4 = chroma_ref.col() >> usize::from(subsampling_x);
    let base_y4 = chroma_ref.row() >> usize::from(subsampling_y);
    let lossless_uses_current_block =
        lossless && is_inter && (frontier.r != chroma_ref.row() || frontier.c != chroma_ref.col());
    chroma_reads.clear();
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
                (subsampling_x, subsampling_y),
                lossless_uses_current_block,
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
                cctx_allowed,
                residual_tool_policy,
                tile_offset,
            )?;
            let u_nonzero = u.eob != 0;
            let uses_cctx = u.cctx_type.unwrap_or(0) != 0;
            let block_index = push_inter_residual_block(
                blocks,
                ReconPlaneId::U,
                base_x4 + x4,
                base_y4 + y4,
                tx_size,
                u,
                tile_offset,
            )?;
            chroma_reads
                .try_reserve(1)
                .map_err(|_| inter_allocation!("inter residual chroma read list"))?;
            chroma_reads.push(InterChromaURead {
                unit,
                block_index,
                uses_cctx,
                u_nonzero,
            });
            x4 = x4
                .checked_add(tx_w4)
                .ok_or_else(|| residual_geometry_error(tile_offset))?;
        }
        y4 = y4
            .checked_add(tx_h4)
            .ok_or_else(|| residual_geometry_error(tile_offset))?;
    }

    for read in chroma_reads.iter().copied() {
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
            cctx_allowed,
            residual_tool_policy,
            tile_offset,
        )?;
        let v_index = push_inter_residual_block(
            blocks,
            ReconPlaneId::V,
            base_x4 + read.unit.x4,
            base_y4 + read.unit.y4,
            tx_size,
            v,
            tile_offset,
        )?;
        if read.uses_cctx {
            record_inter_residual_chroma_pair(blocks, read.block_index, v_index, tile_offset)?;
        }
    }
    Ok(())
}

fn record_inter_residual_chroma_pair(
    blocks: &mut [InterResidualBlock],
    u_index: usize,
    v_index: usize,
    tile_offset: ByteOffset,
) -> Result<()> {
    let delta = v_index
        .checked_sub(u_index)
        .and_then(|delta| i16::try_from(delta).ok())
        .filter(|&delta| delta > 0)
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    let [u, v] = blocks
        .get_disjoint_mut([u_index, v_index])
        .map_err(|_| residual_geometry_error(tile_offset))?;
    u.cctx_pair_delta = delta;
    v.cctx_pair_delta = -delta;
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
) -> Result<usize> {
    let log2_width = tx_size_dimension(&TX_WIDTH_LOG2, tx_size, tile_offset)?;
    let log2_height = tx_size_dimension(&TX_HEIGHT_LOG2, tx_size, tile_offset)?;
    let log2_width = u32::try_from(log2_width).map_err(|_| residual_geometry_error(tile_offset))?;
    let log2_height =
        u32::try_from(log2_height).map_err(|_| residual_geometry_error(tile_offset))?;
    blocks
        .try_reserve(1)
        .map_err(|_| residual_allocation_error())?;
    let index = blocks.len();
    blocks.push(InterResidualBlock {
        plane,
        x: x4 * MI_SIZE,
        y: y4 * MI_SIZE,
        tx_size,
        log2_width,
        log2_height,
        cctx_pair_delta: 0,
        coeffs,
    });
    Ok(index)
}

fn residual_allocation_error() -> crate::error::DecodeError {
    inter_allocation!("inter residual transform-block list")
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
    cctx_allowed: bool,
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
        chroma_eob_ctx,
        INTER_UV_MODE_DC,
        0,
        chroma_inter_tx_type,
        true,
        false,
        false,
        cctx_allowed,
        residual_tool_policy,
    )
    .map_err(|error| residual_plane_read_error(&error, tile_offset))
}

pub(crate) fn max_tx_size(block_size: usize, tile_offset: ByteOffset) -> Result<usize> {
    table_value_usize(&MAX_TX_SIZE_RECT, block_size, tile_offset)
}

fn inter_residual_tx_size(
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    block_size: BlockSize,
    lossless: bool,
    mode: InterResidualLumaTxSizeMode,
    tile_offset: ByteOffset,
) -> Result<usize> {
    if !lossless {
        return max_tx_size(block_size.index(), tile_offset);
    }
    match mode {
        InterResidualLumaTxSizeMode::Inter | InterResidualLumaTxSizeMode::Intrabc => {
            read_lossless_tx_size(work_unit, symbols, block_size, false, true, true)
                .map_err(|error| residual_read_error(&error, SPEC_TX_SIZE, tile_offset))
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
    values: &[i32],
    tx_size: usize,
    tile_offset: ByteOffset,
) -> Result<usize> {
    table_value_usize(values, tx_size, tile_offset)
}

fn table_value_usize(values: &[i32], index: usize, tile_offset: ByteOffset) -> Result<usize> {
    let value = values
        .get(index)
        .copied()
        .ok_or_else(|| residual_geometry_error(tile_offset))?;
    usize::try_from(value).map_err(|_| residual_geometry_error(tile_offset))
}

pub(super) fn residual_geometry_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_internal!("inter_block_residual_geometry", tile_offset)
}

/// § 5.20.4 `reset_block_context`.
///
/// The spec takes `c`/`r`/`w4`/`h4` from `ChromaMiCol`, `ChromaMiRow` and
/// `ChromaMiSize` for the chroma planes, and from the luma block only for plane
/// 0. For a sub-8x8 luma block the two differ: the chroma reference is anchored
/// at the group's base and spans the whole group, so clearing from the luma
/// position leaves the group's leading chroma columns holding a stale level.
pub(crate) fn reset_inter_skip_coeff_contexts(
    coeff_ctx: &mut TileCoeffContextState,
    frontier: &DecodeBlockFrontier,
    n4w: usize,
    n4h: usize,
    subsampling: (u32, u32),
    tile_offset: ByteOffset,
) -> Result<()> {
    let plane_count = 1 + usize::from(frontier.has_chroma) * (COEFF_CONTEXT_PLANES.len() - 1);
    let chroma_ref = frontier.chroma_ref_geometry();
    let chroma_n4w = chroma_ref
        .size()
        .num_4x4_wide()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    let chroma_n4h = chroma_ref
        .size()
        .num_4x4_high()
        .map_err(|_| residual_geometry_error(tile_offset))?;
    let (sub_x, sub_y) = subsampling;
    for &plane in COEFF_CONTEXT_PLANES.iter().take(plane_count) {
        let (r, c, w4, h4) = if plane == 0 {
            (frontier.r, frontier.c, n4w, n4h)
        } else {
            (chroma_ref.row(), chroma_ref.col(), chroma_n4w, chroma_n4h)
        };
        coeff_ctx
            .reset_block_context_plane(CoeffContextReset {
                plane,
                c,
                r,
                w4,
                h4,
                sub_x: if plane == 0 { 0 } else { sub_x },
                sub_y: if plane == 0 { 0 } else { sub_y },
            })
            .map_err(|_| residual_geometry_error(tile_offset))?;
    }
    Ok(())
}

fn residual_read_error(
    error: &(dyn std::error::Error + 'static),
    spec_section: &'static str,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    let mut source = Some(error);
    let mut source_spec_section = spec_section;
    while let Some(current) = source {
        if current
            .downcast_ref::<std::collections::TryReserveError>()
            .is_some()
        {
            return inter_allocation!("inter residual coefficient parse state");
        }
        if matches!(
            current.downcast_ref::<crate::bitstream::tile_payload::CoeffLoopContextError>(),
            Some(
                crate::bitstream::tile_payload::CoeffLoopContextError::InvalidPt512EobExtra { .. }
            )
        ) {
            return crate::pipeline::malformed_tile_payload(tile_offset, spec_section, error);
        }
        if let Some(read_quant_error) =
            current.downcast_ref::<crate::bitstream::tile_payload::CoeffReadQuantError>()
        {
            source_spec_section = SPEC_READ_QUANT;
            if matches!(
                read_quant_error,
                crate::bitstream::tile_payload::CoeffReadQuantError::OverlongGolombPrefix { .. }
            ) {
                return crate::pipeline::malformed_tile_payload(
                    tile_offset,
                    source_spec_section,
                    error,
                );
            }
        }
        if let Some(core_error) = current.downcast_ref::<splot_core::Error>() {
            return if matches!(core_error, splot_core::Error::UnexpectedEof { .. }) {
                crate::pipeline::malformed_tile_payload(tile_offset, source_spec_section, error)
            } else {
                residual_read_internal_error(tile_offset)
            };
        }
        source = current.source();
    }
    residual_read_internal_error(tile_offset)
}

fn residual_plane_read_error(
    error: &crate::bitstream::tile_payload::GeneralIntraResidualError,
    tile_offset: ByteOffset,
) -> crate::error::DecodeError {
    let spec_section = if matches!(
        error,
        crate::bitstream::tile_payload::GeneralIntraResidualError::TransformTypeRead { .. }
    ) {
        SPEC_TRANSFORM_TYPE
    } else {
        SPEC_RESIDUAL
    };
    residual_read_error(error, spec_section, tile_offset)
}

fn residual_read_internal_error(tile_offset: ByteOffset) -> crate::error::DecodeError {
    inter_internal!("inter_block_residual_parse", tile_offset)
}

#[cfg(test)]
#[path = "residual_tests.rs"]
mod tests;
