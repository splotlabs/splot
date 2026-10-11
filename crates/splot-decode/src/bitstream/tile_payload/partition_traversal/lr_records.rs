// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Loop-restoration selection and source-block records.

use super::lr_syntax::{MI_SIZE, WIENER_NS_CHROMA_COEFFS, WienerNsUnitFilterState};
use super::{
    DecodeLimitName, DecodeLimits, TilePartitionBounds, TilePartitionFrameFacts,
    TilePartitionTraversalError, checked_add, checked_mul, checked_mul_shifted, checked_shl,
    checked_sub,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LrUnitRestorationType {
    None,
    PcWiener,
    WienerNonsep,
}

impl LrUnitRestorationType {
    pub(super) const fn is_active(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrSourceBlock {
    pub(crate) restoration_type: LrUnitRestorationType,
    pub(crate) plane: usize,
    pub(crate) unit_row: usize,
    pub(crate) unit_col: usize,
    pub(crate) unit_filter_index: Option<usize>,
    pub(crate) tile_mi_row_start: usize,
    pub(crate) tile_mi_row_end: usize,
    pub(crate) tile_mi_col_end: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) luma_start_x: usize,
    pub(crate) luma_end_x: usize,
    pub(crate) luma_start_y: usize,
    pub(crate) luma_end_y: usize,
    pub(crate) luma_stripe_start_y: usize,
    pub(crate) luma_stripe_end_y: usize,
}

impl WienerNsLrSourceBlock {
    pub(crate) fn merged_width_with(&self, next: &Self) -> Option<usize> {
        (self.same_filter_domain(next)
            && self.y == next.y
            && self.height == next.height
            && self.x.checked_add(self.width) == Some(next.x))
        .then(|| self.width.checked_add(next.width))
        .flatten()
    }

    pub(crate) fn merged_height_with(&self, next: &Self) -> Option<usize> {
        (self.same_filter_domain(next)
            && self.x == next.x
            && self.width == next.width
            && self.y.checked_add(self.height) == Some(next.y))
        .then(|| self.height.checked_add(next.height))
        .flatten()
    }

    fn same_filter_domain(&self, next: &Self) -> bool {
        self.filter_domain_key() == next.filter_domain_key()
    }

    pub(crate) fn vertical_merge_key(&self) -> ([usize; 14], usize, usize) {
        (self.filter_domain_key(), self.x, self.width)
    }

    fn filter_domain_key(&self) -> [usize; 14] {
        [
            self.restoration_type as usize,
            self.plane,
            self.unit_row,
            self.unit_col,
            self.unit_filter_index.unwrap_or(usize::MAX),
            self.tile_mi_row_start,
            self.tile_mi_row_end,
            self.tile_mi_col_end,
            self.luma_start_x,
            self.luma_end_x,
            self.luma_start_y,
            self.luma_end_y,
            self.luma_stripe_start_y,
            self.luma_stripe_end_y,
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WienerNsLrUnitFilter {
    pub(crate) plane: usize,
    pub(crate) unit_row: usize,
    pub(crate) unit_col: usize,
    pub(crate) coeff_count: usize,
    pub(crate) coeffs: [i16; WIENER_NS_CHROMA_COEFFS],
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WienerNsLrUnitActivity {
    pub(super) active_source_blocks: Vec<WienerNsLrSourceBlock>,
    pub(super) unit_filters: Vec<WienerNsLrUnitFilter>,
    pub(super) unit_filter_state: WienerNsUnitFilterState,
}

impl WienerNsLrUnitActivity {
    fn record_source_block(
        &mut self,
        block: WienerNsLrSourceBlock,
        limits: DecodeLimits,
    ) -> Result<(), TilePartitionTraversalError> {
        if let Some(last) = self.active_source_blocks.last_mut() {
            if let Some(width) = last.merged_width_with(&block) {
                last.width = width;
                return Ok(());
            }
            if let Some(height) = last.merged_height_with(&block) {
                last.height = height;
                return Ok(());
            }
        }
        let next_len = checked_add(
            "lr_active_source_blocks",
            self.active_source_blocks.len(),
            1,
        )?;
        limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, next_len as u64)?;
        self.active_source_blocks.try_reserve(1)?;
        self.active_source_blocks.push(block);
        Ok(())
    }

    pub(super) fn record_unit_filter(
        &mut self,
        filter: WienerNsLrUnitFilter,
        limits: DecodeLimits,
    ) -> Result<(), TilePartitionTraversalError> {
        let next_len = checked_add("lr_unit_filters", self.unit_filters.len(), 1)?;
        limits.ensure_allocation_len(DecodeLimitName::MaxLumaSamplesPerFrame, next_len as u64)?;
        self.unit_filters.try_reserve(1)?;
        self.unit_filters.push(filter);
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(super) struct LrSourceBlockDerivation {
    pub(super) restoration_type: LrUnitRestorationType,
    pub(super) plane: usize,
    pub(super) unit_size: usize,
    pub(super) unit_row: usize,
    pub(super) unit_col: usize,
    pub(super) unit_filter_index: Option<usize>,
    pub(super) frame: TilePartitionFrameFacts,
    pub(super) tile_bounds: TilePartitionBounds,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
}

pub(super) fn record_active_wiener_ns_source_blocks_for_unit(
    input: LrSourceBlockDerivation,
    limits: DecodeLimits,
    lr_activity: &mut WienerNsLrUnitActivity,
) -> Result<(), TilePartitionTraversalError> {
    let geometry = lr_unit_geometry(input)?;
    let tile = input.tile_bounds;
    let row_axis = UnitMiAxis {
        offset: geometry.lr_row_offset,
        units: geometry.unit_rows,
        unit_size: input.unit_size,
        sub: input.sub_y,
        bias: 8,
        mi_count: geometry.mi_rows,
    };
    let col_axis = UnitMiAxis {
        offset: geometry.lr_col_offset,
        units: geometry.unit_cols,
        unit_size: input.unit_size,
        sub: input.sub_x,
        bias: 0,
        mi_count: geometry.mi_cols,
    };
    let (Some((row_start, row_end)), Some((col_start, col_end))) = (
        row_axis.span(input.unit_row)?,
        col_axis.span(input.unit_col)?,
    ) else {
        return Ok(());
    };
    let cols = col_end - col_start;
    for row in tile.mi_row_start + row_start..tile.mi_row_start + row_end {
        let mut block = lr_source_block_for(input, row, tile.mi_col_start + col_start)?;
        block.width = checked_mul("lr_source_run_width", block.width, cols)?;
        lr_activity.record_source_block(block, limits)?;
    }
    Ok(())
}

/// One axis of the monotone map from local MI index `mi` to restoration unit
/// `offset + min(((MI_SIZE * mi + bias) >> sub) / unit_size, units - 1)`.
struct UnitMiAxis {
    offset: usize,
    units: usize,
    unit_size: usize,
    sub: usize,
    bias: usize,
    mi_count: usize,
}

impl UnitMiAxis {
    /// Returns the local MI range `[start, end)` that maps to `unit`.
    fn span(&self, unit: usize) -> Result<Option<(usize, usize)>, TilePartitionTraversalError> {
        let Some(local) = unit.checked_sub(self.offset).filter(|&u| u < self.units) else {
            return Ok(None);
        };
        let first_mi = |local: usize| -> Result<usize, TilePartitionTraversalError> {
            let sample = checked_mul("lr_source_unit_sample", local, self.unit_size)?;
            let sample = checked_shl("lr_source_unit_sample", sample, self.sub)?;
            Ok(sample
                .saturating_sub(self.bias)
                .div_ceil(MI_SIZE)
                .min(self.mi_count))
        };
        let start = first_mi(local)?;
        let end = if local + 1 == self.units {
            self.mi_count
        } else {
            first_mi(local + 1)?
        };
        Ok((start < end).then_some((start, end)))
    }
}

#[derive(Clone, Copy)]
struct LrUnitGeometry {
    unit_rows: usize,
    unit_cols: usize,
    lr_row_offset: usize,
    lr_col_offset: usize,
    mi_rows: usize,
    mi_cols: usize,
}

fn lr_unit_geometry(
    input: LrSourceBlockDerivation,
) -> Result<LrUnitGeometry, TilePartitionTraversalError> {
    let mi_cols = checked_sub(
        "lr_source_mi_cols",
        input.tile_bounds.mi_col_end,
        input.tile_bounds.mi_col_start,
    )?;
    let mi_rows = checked_sub(
        "lr_source_mi_rows",
        input.tile_bounds.mi_row_end,
        input.tile_bounds.mi_row_start,
    )?;
    let frame_cols = checked_mul_shifted("lr_source_frame_cols", mi_cols, MI_SIZE, input.sub_x)?;
    let frame_rows = checked_mul_shifted("lr_source_frame_rows", mi_rows, MI_SIZE, input.sub_y)?;
    let unit_rows = count_units_in_frame(input.unit_size, frame_rows)?;
    let unit_cols = count_units_in_frame(input.unit_size, frame_cols)?;
    let lr_row_offset = checked_mul_shifted(
        "lr_source_row_offset",
        input.tile_bounds.mi_row_start,
        MI_SIZE,
        input.sub_y,
    )? / input.unit_size;
    let lr_col_offset = checked_mul_shifted(
        "lr_source_col_offset",
        input.tile_bounds.mi_col_start,
        MI_SIZE,
        input.sub_x,
    )? / input.unit_size;
    Ok(LrUnitGeometry {
        unit_rows,
        unit_cols,
        lr_row_offset,
        lr_col_offset,
        mi_rows,
        mi_cols,
    })
}

fn lr_source_block_for(
    input: LrSourceBlockDerivation,
    row: usize,
    col: usize,
) -> Result<WienerNsLrSourceBlock, TilePartitionTraversalError> {
    let x = checked_mul_shifted("lr_source_x", col, MI_SIZE, input.sub_x)?;
    let y = checked_mul_shifted("lr_source_y", row, MI_SIZE, input.sub_y)?;
    let width = MI_SIZE >> input.sub_x;
    let height = MI_SIZE >> input.sub_y;
    let (luma_start_x_mi, luma_end_x_mi, luma_start_y_mi, luma_end_y_mi) =
        if input.frame.disable_loopfilters_across_tiles {
            (
                input.tile_bounds.mi_col_start,
                input.tile_bounds.mi_col_end,
                input.tile_bounds.mi_row_start,
                input.tile_bounds.mi_row_end,
            )
        } else {
            (0, input.frame.mi_cols, 0, input.frame.mi_rows)
        };
    let luma_start_x = checked_mul("lr_luma_start_x", luma_start_x_mi, MI_SIZE)?;
    let luma_start_y = checked_mul("lr_luma_start_y", luma_start_y_mi, MI_SIZE)?;
    let luma_end_x = checked_sub(
        "lr_luma_end_x",
        checked_mul("lr_luma_end_x", luma_end_x_mi, MI_SIZE)?,
        1,
    )?;
    let luma_end_y = checked_sub(
        "lr_luma_end_y",
        checked_mul("lr_luma_end_y", luma_end_y_mi, MI_SIZE)?,
        1,
    )?;
    let local_row = checked_sub("lr_source_local_row", row, input.tile_bounds.mi_row_start)?;
    let luma_y = checked_mul("lr_source_luma_y", local_row, MI_SIZE)?;
    let stripe_num = checked_add("lr_source_stripe_num", luma_y, 8)? / 64;
    let stripe_base = checked_add(
        "lr_source_stripe_base",
        checked_mul(
            "lr_source_stripe_base",
            input.tile_bounds.mi_row_start,
            MI_SIZE,
        )?,
        checked_mul("lr_source_stripe_base", stripe_num, 64)?,
    )?;
    let luma_stripe_start_y = stripe_base
        .checked_sub(8)
        .map_or(luma_start_y, |start| luma_start_y.max(start));
    let luma_stripe_end_y = luma_end_y.min(checked_add("lr_source_stripe_end_y", stripe_base, 55)?);

    Ok(WienerNsLrSourceBlock {
        restoration_type: input.restoration_type,
        plane: input.plane,
        unit_row: input.unit_row,
        unit_col: input.unit_col,
        unit_filter_index: input.unit_filter_index,
        tile_mi_row_start: input.tile_bounds.mi_row_start,
        tile_mi_row_end: input.tile_bounds.mi_row_end,
        tile_mi_col_end: input.tile_bounds.mi_col_end,
        x,
        y,
        width,
        height,
        luma_start_x,
        luma_end_x,
        luma_start_y,
        luma_end_y,
        luma_stripe_start_y,
        luma_stripe_end_y,
    })
}

pub(super) fn count_units_in_frame(
    unit_size: usize,
    frame_size: usize,
) -> Result<usize, TilePartitionTraversalError> {
    Ok(checked_add("lr_count_units", frame_size, unit_size >> 1)? / unit_size)
        .map(|count| count.max(1))
}

pub(super) fn ceil_unit_index(
    value: usize,
    unit_size: usize,
) -> Result<usize, TilePartitionTraversalError> {
    let adjusted = checked_add("lr_unit_ceil", value, unit_size.saturating_sub(1))?;
    Ok(adjusted / unit_size)
}

#[cfg(test)]
mod tests {
    use super::{MI_SIZE, UnitMiAxis};

    #[test]
    fn unit_mi_span_inverts_the_mi_to_unit_map() {
        for unit_size in [16, 18, 32, 64, 128, 256, 512] {
            for (sub, bias, offset) in [(0, 0, 0), (1, 0, 3), (0, 8, 2), (1, 8, 0)] {
                for mi_count in (1..80).chain([270, 480]) {
                    let samples = (MI_SIZE * mi_count) >> sub;
                    let units = ((samples + (unit_size >> 1)) / unit_size).max(1);
                    let axis = UnitMiAxis {
                        offset,
                        units,
                        unit_size,
                        sub,
                        bias,
                        mi_count,
                    };
                    for unit in 0..offset + units + 2 {
                        let mapped: Vec<usize> = (0..mi_count)
                            .filter(|&mi| {
                                let local = ((MI_SIZE * mi + bias) >> sub) / unit_size;
                                offset + local.min(units - 1) == unit
                            })
                            .collect();
                        let expected = mapped.first().zip(mapped.last()).map(|(&s, &e)| (s, e + 1));
                        assert_eq!(axis.span(unit).ok().flatten(), expected);
                    }
                }
            }
        }
    }
}
