// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Palette color-map symbol decoding.

use core::num::NonZeroU32;

use splot_core::symbol::SymbolDecoder;

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, PositionedLumaCoeffBlock, TileCdfSelector,
};

use super::transform_units::tx_size_log2;
use super::{LumaPalette, RectLumaPlan, ResidualPlanePlan, ResidualReconstructionPlan};
use crate::tile::block_context::FrameTileCtx;

const PALETTE_MAX_SIZE: usize = 8;
const PALETTE_COLOR_CONTEXTS: usize = 5;
const PALETTE_ROW_COPY_PREVIOUS: u8 = 2;
const PALETTE_ROW_COPY_LAST: u8 = 1;
const PALETTE_DIRECTION_REASON: &str = "palette_direction";
const PALETTE_UNIFORM_REASON: &str = "palette_color_idx_uniform";

/// Spare palette color maps. The worker that parses a block is often not the
/// one that reconstructs it, so the spares are shared rather than per thread.
static SPARE_PALETTE_MAPS: parking_lot::Mutex<Vec<Vec<u8>>> = parking_lot::Mutex::new(Vec::new());

/// Spares kept; a map beyond this many in flight is freed.
const MAX_SPARE_PALETTE_MAPS: usize = 64;

/// A color map covers one palette plane, which is at most a 64x64 transform.
const MAX_PALETTE_MAP_SAMPLES: usize = 64 * 64;

/// Takes a spare color map, or a new one sized so it never has to grow.
fn take_palette_map() -> Vec<u8> {
    SPARE_PALETTE_MAPS
        .lock()
        .pop()
        .unwrap_or_else(|| Vec::with_capacity(MAX_PALETTE_MAP_SAMPLES))
}

/// Gives a reconstructed block's color map back for the next palette block.
pub(super) fn recycle_palette_map(map: Option<Vec<u8>>) {
    let Some(mut map) = map else {
        return;
    };
    map.clear();
    let mut spares = SPARE_PALETTE_MAPS.lock();
    if spares.len() < MAX_SPARE_PALETTE_MAPS {
        spares.push(map);
    }
}

/// Moves `map` into the row's `maps` and returns its one-based index.
pub(super) fn store_palette_map(
    maps: &mut Vec<Vec<u8>>,
    map: Option<Vec<u8>>,
) -> Option<NonZeroU32> {
    let map = map?;
    let index = NonZeroU32::new(u32::try_from(maps.len()).ok()?.checked_add(1)?)?;
    maps.push(map);
    Some(index)
}

/// Takes the map that [`store_palette_map`] stored at `index` out of `maps`.
pub(super) fn take_stored_palette_map(
    maps: &mut [Vec<u8>],
    index: Option<NonZeroU32>,
) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
    let Some(index) = index else {
        return Ok(None);
    };
    maps.get_mut(index.get() as usize - 1)
        .map(|map| Some(core::mem::take(map)))
        .ok_or(GeneralIntraResidualError::InvalidReconstructionState {
            context: "palette color map index",
        })
}

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
        let (log2_width, log2_height) = tx_size_log2(usize::from(block.tx_size))?;
        let unit_width = 1usize << log2_width;
        let unit_height = 1usize << log2_height;
        let Some(local_x) = (block.x as usize).checked_sub(self.x as usize) else {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "palette transform X origin",
            });
        };
        let Some(local_y) = (block.y as usize).checked_sub(self.y as usize) else {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "palette transform Y origin",
            });
        };
        if local_x.saturating_add(unit_width) > parent_width
            || local_y.saturating_add(unit_height) > parent_height
        {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "palette transform extent",
            });
        }
        let mut unit_map = take_palette_map();
        for row in 0..unit_height {
            let start = (local_y + row) * parent_width + local_x;
            let end = start + unit_width;
            unit_map.extend_from_slice(&parent_map[start..end]);
        }
        Ok(Some(unit_map))
    }

    #[inline]
    pub(super) fn read_palette_color_map(
        &self,
        frame: FrameTileCtx,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let ResidualReconstructionPlan::Luma(RectLumaPlan::Palette { palette, .. }) =
            self.reconstruction
        else {
            return Ok(None);
        };
        self.read_palette_colors(frame, palette, work_unit, symbols)
    }

    #[inline(never)]
    fn read_palette_colors(
        &self,
        frame: FrameTileCtx,
        palette: LumaPalette,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let plane_width = 1usize << self.tx.width_log2();
        let plane_height = 1usize << self.tx.height_log2();
        let frame_width = frame.frame_mi_cols().saturating_mul(4);
        let frame_height = frame.frame_mi_rows().saturating_mul(4);
        let cols = plane_width.min(frame_width.saturating_sub(self.x as usize));
        let rows = plane_height.min(frame_height.saturating_sub(self.y as usize));
        let mut color_map = take_palette_map();
        color_map.resize(plane_width.saturating_mul(plane_height), 0);
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
            let line = ax2 * plane_width;
            if identity_row_flag == PALETTE_ROW_COPY_PREVIOUS {
                if direction {
                    for at in (ax2..).step_by(plane_width).take(rows) {
                        color_map[at] = color_map[at - 1];
                    }
                } else {
                    color_map.copy_within(line - plane_width..line - plane_width + cols, line);
                }
            } else {
                let one_color = identity_row_flag == PALETTE_ROW_COPY_LAST;
                let coded = if one_color {
                    axis1_limit.min(1)
                } else {
                    axis1_limit
                };
                for ax1 in 0..coded {
                    let (y, x) = if direction { (ax1, ax2) } else { (ax2, ax1) };
                    color_map[y * plane_width + x] = if ax2 == 0 && ax1 == 0 {
                        read_palette_uniform(symbols, palette.size())? as u8
                    } else {
                        let (color_ctx, color_order) =
                            palette_color_index_context(&color_map, plane_width, y, x);
                        let color_idx = work_unit
                            .cdf_mut()
                            .tile_cdfs_mut()
                            .read_palette_y_color(palette.size(), color_ctx, symbols)
                            .map_err(|source| GeneralIntraResidualError::PaletteSymbolRead {
                                source,
                            })? as usize;
                        *color_order.get(color_idx).ok_or_else(|| {
                            GeneralIntraResidualError::PaletteColorIndex {
                                color_index: color_idx,
                                palette_size: palette.size(),
                            }
                        })?
                    };
                }
                if coded < axis1_limit {
                    if direction {
                        let first = color_map[ax2];
                        for at in (ax2..).step_by(plane_width).take(rows).skip(1) {
                            color_map[at] = first;
                        }
                    } else {
                        let first = color_map[line];
                        color_map[line + 1..line + cols].fill(first);
                    }
                }
            }
            prev_identity_row_flag = usize::from(identity_row_flag);
        }
        extend_palette_map(&mut color_map, plane_width, cols, rows);
        Ok(Some(color_map))
    }
}

/// Extends the visible `cols` x `rows` corner of the row-major `map` over the
/// whole block, as § 5.20.8.4 `palette_tokens` ends: each visible row repeats
/// its last visible sample, and each row below repeats the last visible row.
/// An empty corner leaves `map` as it is.
fn extend_palette_map(map: &mut [u8], width: usize, cols: usize, rows: usize) {
    if cols == 0 || rows == 0 {
        return;
    }
    let (visible, below) = map.split_at_mut(rows * width);
    if cols < width {
        for row in visible.chunks_exact_mut(width) {
            let (seen, hidden) = row.split_at_mut(cols);
            hidden.fill(seen[cols - 1]);
        }
    }
    let last = &visible[(rows - 1) * width..];
    for row in below.chunks_exact_mut(width) {
        row.copy_from_slice(last);
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

pub(crate) fn palette_color_index_context(
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
mod tests {
    use super::*;
    use crate::bitstream::tile_payload::FrameCdfSubset;

    /// The § 5.20.8.4 extension, one sample at a time.
    fn extend_palette_map_oracle(map: &mut [u8], width: usize, cols: usize, rows: usize) {
        let height = map.len() / width;
        for y in 0..rows {
            for x in cols..width {
                map[y * width + x] = map[y * width + cols - 1];
            }
        }
        for y in rows..height {
            for x in 0..width {
                map[y * width + x] = map[(rows - 1) * width + x];
            }
        }
    }

    #[test]
    fn palette_map_extension_matches_the_per_sample_oracle() {
        for (width, height) in [(4, 4), (8, 4), (4, 16), (16, 8), (32, 64), (64, 64)] {
            for rows in 1..=height {
                for cols in 1..=width {
                    let mut map = vec![0xff_u8; width * height];
                    for (y, row) in map.chunks_exact_mut(width).take(rows).enumerate() {
                        for (x, index) in row[..cols].iter_mut().enumerate() {
                            *index = ((x * 3 + y * 5) % 8) as u8;
                        }
                    }
                    let mut expected = map.clone();
                    extend_palette_map_oracle(&mut expected, width, cols, rows);
                    extend_palette_map(&mut map, width, cols, rows);
                    assert_eq!(map, expected, "{width}x{height} visible {cols}x{rows}");
                }
            }
        }
        for (cols, rows) in [(0, 2), (2, 0)] {
            let mut map = [7_u8; 16];
            extend_palette_map(&mut map, 4, cols, rows);
            assert_eq!(map, [7; 16], "visible {cols}x{rows}");
        }
    }

    #[test]
    fn palette_cdf_alphabets_and_color_order_stay_inside_palette_size() {
        let mut tile = FrameCdfSubset::from_defaults().tile_copy();
        let palette_size_row = tile
            .with_row_mut(TileCdfSelector::PaletteYSize, |row| row.to_vec())
            .expect("palette-size selector");
        assert_eq!(palette_size_row.len() - 1, PALETTE_MAX_SIZE - 1);

        for palette_size in 2..=PALETTE_MAX_SIZE {
            for ctx in 0..PALETTE_COLOR_CONTEXTS {
                let row = tile
                    .with_row_mut(
                        TileCdfSelector::PaletteYColorIndex { palette_size, ctx },
                        |row| row.to_vec(),
                    )
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
