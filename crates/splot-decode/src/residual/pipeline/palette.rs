// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Palette color-map symbol decoding.

use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, PositionedLumaCoeffBlock, TileCdfSelector,
};

use super::transform_units::tx_size_log2;
use super::{ResidualPlanePlan, ResidualReconstructionPlan};

const PALETTE_MAX_SIZE: usize = 8;
const PALETTE_COLOR_CONTEXTS: usize = 5;
const PALETTE_ROW_COPY_PREVIOUS: u8 = 2;
const PALETTE_ROW_COPY_LAST: u8 = 1;
const PALETTE_DIRECTION_REASON: &str = "palette_direction";
const PALETTE_UNIFORM_REASON: &str = "palette_color_idx_uniform";

impl ResidualPlanePlan {
    pub(super) fn palette_color_map_for_unit(
        &self,
        parent_map: Option<&[u8]>,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let Some(parent_map) = parent_map else {
            return Ok(None);
        };
        let parent_width = 1usize << self.tx.width_log2();
        let parent_height = 1usize << self.tx.height_log2();
        let expected_parent = parent_width.saturating_mul(parent_height);
        if parent_map.len() != expected_parent {
            return Err(GeneralIntraResidualError::PredictionLength {
                expected: expected_parent,
                actual: parent_map.len(),
            });
        }
        let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
        let unit_width = 1usize << log2_width;
        let unit_height = 1usize << log2_height;
        let local_x = block
            .x
            .checked_sub(self.x)
            .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
        let local_y = block
            .y
            .checked_sub(self.y)
            .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
        if local_x.saturating_add(unit_width) > parent_width
            || local_y.saturating_add(unit_height) > parent_height
        {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
        let mut unit_map = Vec::with_capacity(unit_width.saturating_mul(unit_height));
        for row in 0..unit_height {
            let start = (local_y + row) * parent_width + local_x;
            let end = start + unit_width;
            unit_map.extend_from_slice(&parent_map[start..end]);
        }
        Ok(Some(unit_map))
    }

    pub(super) fn read_palette_color_map(
        self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let ResidualReconstructionPlan::LumaPalette { palette, .. } = self.reconstruction else {
            return Ok(None);
        };
        let plane_width = 1usize << self.tx.width_log2();
        let plane_height = 1usize << self.tx.height_log2();
        let frame_width = self.block_ctx.frame_mi_cols().saturating_mul(4);
        let frame_height = self.block_ctx.frame_mi_rows().saturating_mul(4);
        let cols = plane_width.min(frame_width.saturating_sub(self.x));
        let rows = plane_height.min(frame_height.saturating_sub(self.y));
        let mut color_map = vec![0u8; plane_width.saturating_mul(plane_height)];
        let direction = if plane_width < 64 && plane_height < 64 {
            read_palette_literal(symbols, 1, PALETTE_DIRECTION_REASON)? != 0
        } else {
            false
        };
        let axis1_limit = if direction { rows } else { cols };
        let axis2_limit = if direction { cols } else { rows };
        let mut prev_identity_row_flag = 0usize;

        for ax2 in 0..axis2_limit {
            let ctx = if ax2 == 0 { 3 } else { prev_identity_row_flag };
            let identity_row_flag = work_unit
                .cdf_mut()
                .tile_cdfs_mut()
                .read_block_symbol_trace(TileCdfSelector::IdentityRowY { ctx }, symbols)
                .map(splot_core::symbol::Symbol::get)
                .map_err(|source| GeneralIntraResidualError::PaletteSymbolRead { source })?;
            if identity_row_flag == PALETTE_ROW_COPY_PREVIOUS && ax2 == 0 {
                return Err(GeneralIntraResidualError::PaletteInvalidIdentityRow);
            }
            for ax1 in 0..axis1_limit {
                let y = if direction { ax1 } else { ax2 };
                let x = if direction { ax2 } else { ax1 };
                let offset = y * plane_width + x;
                color_map[offset] = if identity_row_flag == PALETTE_ROW_COPY_PREVIOUS {
                    if direction {
                        color_map[y * plane_width + x - 1]
                    } else {
                        color_map[(y - 1) * plane_width + x]
                    }
                } else if identity_row_flag == PALETTE_ROW_COPY_LAST && ax1 > 0 {
                    if direction {
                        color_map[(y - 1) * plane_width + x]
                    } else {
                        color_map[y * plane_width + x - 1]
                    }
                } else if ax2 == 0 && ax1 == 0 {
                    read_palette_uniform(symbols, palette.size())? as u8
                } else {
                    let (color_ctx, color_order) =
                        palette_color_index_context(&color_map, plane_width, y, x);
                    let color_idx = work_unit
                        .cdf_mut()
                        .tile_cdfs_mut()
                        .read_block_symbol_trace(
                            TileCdfSelector::PaletteYColorIndex {
                                palette_size: palette.size(),
                                ctx: color_ctx,
                            },
                            symbols,
                        )
                        .map(splot_core::symbol::Symbol::get)
                        .map_err(|source| GeneralIntraResidualError::PaletteSymbolRead { source })?
                        as usize;
                    *color_order.get(color_idx).ok_or(
                        GeneralIntraResidualError::PaletteColorIndex {
                            color_index: color_idx,
                            palette_size: palette.size(),
                        },
                    )?
                };
            }
            prev_identity_row_flag = usize::from(identity_row_flag);
        }
        if cols != 0 && cols < plane_width {
            for y in 0..rows {
                let fill = color_map[y * plane_width + cols - 1];
                for x in cols..plane_width {
                    color_map[y * plane_width + x] = fill;
                }
            }
        }
        if rows != 0 {
            for y in rows..plane_height {
                let src = (rows - 1) * plane_width;
                let dst = y * plane_width;
                for x in 0..plane_width {
                    color_map[dst + x] = color_map[src + x];
                }
            }
        }
        Ok(Some(color_map))
    }
}

fn read_palette_uniform(
    symbols: &mut SymbolDecoder<'_>,
    num_values: usize,
) -> core::result::Result<usize, GeneralIntraResidualError> {
    let bits = unsigned_bits(num_values);
    if bits == 0 {
        return Ok(0);
    }
    let m = (1usize << bits) - num_values;
    let value = read_palette_literal(symbols, (bits - 1) as u32, PALETTE_UNIFORM_REASON)? as usize;
    if value < m {
        Ok(value)
    } else {
        let extra = read_palette_literal(symbols, 1, PALETTE_UNIFORM_REASON)? as usize;
        Ok((value << 1) - m + extra)
    }
}

fn read_palette_literal(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> core::result::Result<u32, GeneralIntraResidualError> {
    symbols
        .read_literal(bits)
        .map_err(|source| GeneralIntraResidualError::PaletteLiteral { reason, source })
}

const fn unsigned_bits(num_values: usize) -> usize {
    if num_values == 0 {
        0
    } else {
        usize::BITS as usize - num_values.leading_zeros() as usize
    }
}

fn palette_color_index_context(
    color_map: &[u8],
    stride: usize,
    row: usize,
    col: usize,
) -> (usize, [u8; PALETTE_MAX_SIZE]) {
    let mut color_order = [0u8; PALETTE_MAX_SIZE];
    let mut color_status = [false; PALETTE_MAX_SIZE];
    for (index, value) in color_order.iter_mut().enumerate() {
        *value = index as u8;
    }
    let mut color_count = 0usize;
    let color_index_ctx = if row > 0 && col > 0 {
        let left = usize::from(color_map[row * stride + col - 1]);
        let top_left = usize::from(color_map[(row - 1) * stride + col - 1]);
        let top = usize::from(color_map[(row - 1) * stride + col]);
        if left == top_left && left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            4
        } else if left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top_left,
                &mut color_count,
            );
            3
        } else if left == top_left {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top,
                &mut color_count,
            );
            2
        } else if top_left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                top,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                left,
                &mut color_count,
            );
            2
        } else {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                2,
                top_left,
                &mut color_count,
            );
            1
        }
    } else if col == 0 && row > 0 {
        let top = usize::from(color_map[(row - 1) * stride + col]);
        swap_color_order(
            &mut color_order,
            &mut color_status,
            0,
            top,
            &mut color_count,
        );
        0
    } else if col > 0 && row == 0 {
        let left = usize::from(color_map[row * stride + col - 1]);
        swap_color_order(
            &mut color_order,
            &mut color_status,
            0,
            left,
            &mut color_count,
        );
        0
    } else {
        0
    };
    let mut write_idx = color_count;
    for (read_idx, status) in color_status.iter().enumerate() {
        if !status && write_idx < color_order.len() {
            color_order[write_idx] = read_idx as u8;
            write_idx += 1;
        }
    }
    debug_assert!(color_index_ctx < PALETTE_COLOR_CONTEXTS);
    (color_index_ctx, color_order)
}

fn swap_color_order(
    color_order: &mut [u8; PALETTE_MAX_SIZE],
    color_status: &mut [bool; PALETTE_MAX_SIZE],
    switch_idx: usize,
    max_idx: usize,
    color_count: &mut usize,
) {
    if switch_idx < color_order.len() && max_idx < color_status.len() {
        color_order[switch_idx] = max_idx as u8;
        color_status[max_idx] = true;
        *color_count += 1;
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::bitstream::tile_payload::FrameCdfSubset;

    #[test]
    fn palette_cdf_alphabets_and_color_order_stay_inside_palette_size() {
        let tile = FrameCdfSubset::from_defaults().tile_copy();
        let palette_size_row = tile
            .row(TileCdfSelector::PaletteYSize)
            .expect("palette-size selector");
        assert_eq!(palette_size_row.len() - 1, PALETTE_MAX_SIZE - 1);

        for palette_size in 2..=PALETTE_MAX_SIZE {
            for ctx in 0..PALETTE_COLOR_CONTEXTS {
                let row = tile
                    .row(TileCdfSelector::PaletteYColorIndex { palette_size, ctx })
                    .expect("palette color-index selector");
                assert_eq!(row.len() - 1, palette_size);
            }

            let (_, origin_order) = palette_color_index_context(&[0], 1, 0, 0);
            assert!(
                origin_order[..palette_size]
                    .iter()
                    .all(|&index| usize::from(index) < palette_size)
            );

            for neighbour in 0..palette_size {
                let (_, top_order) = palette_color_index_context(&[neighbour as u8, 0], 1, 1, 0);
                let (_, left_order) = palette_color_index_context(&[neighbour as u8, 0], 2, 0, 1);
                for order in [top_order, left_order] {
                    assert!(
                        order[..palette_size]
                            .iter()
                            .all(|&index| usize::from(index) < palette_size)
                    );
                }
            }

            for top_left in 0..palette_size {
                for top in 0..palette_size {
                    for left in 0..palette_size {
                        let map = [top_left as u8, top as u8, left as u8, 0];
                        let (_, order) = palette_color_index_context(&map, 2, 1, 1);
                        let prefix = &order[..palette_size];
                        assert!(
                            prefix
                                .iter()
                                .all(|&index| usize::from(index) < palette_size)
                        );
                        for index in 0..palette_size {
                            assert_eq!(
                                prefix
                                    .iter()
                                    .filter(|&&value| usize::from(value) == index)
                                    .count(),
                                1
                            );
                        }
                    }
                }
            }
        }
    }
}
