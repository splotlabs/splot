// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDF selection-bounds validation and row lifecycle helpers.

use super::{
    CDF_PROB_SCALE, DO_SPLIT_PLANE_CONTEXTS, DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS, TileCdfArray,
    TileCdfError,
};

pub(crate) fn checked_plane(
    array: TileCdfArray,
    plane_start: usize,
) -> Result<usize, TileCdfError> {
    checked_context(array, "plane_start", plane_start, DO_SPLIT_PLANE_CONTEXTS)
}

pub(crate) fn checked_square_split_plane(plane_start: usize) -> Result<usize, TileCdfError> {
    checked_context(
        TileCdfArray::DoSquareSplit,
        "plane_start",
        plane_start,
        DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS,
    )
}

pub(crate) fn checked_context(
    array: TileCdfArray,
    index_name: &'static str,
    actual: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    if actual >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name,
            actual,
            max_exclusive,
        });
    }
    Ok(actual)
}

pub(crate) const fn tx_partition_type_array(reduced: bool) -> TileCdfArray {
    if reduced {
        TileCdfArray::TxPartitionTypeReduced
    } else {
        TileCdfArray::TxPartitionType
    }
}

pub(in crate::bitstream::tile_payload::cdf) fn avg_cdf_row<const N: usize>(
    cdf: &mut [i32; N],
    tile_cdf: &[i32; N],
    tile_num: u32,
    num_log2: u8,
) {
    if tile_num == 0 {
        for value in &mut cdf[..N - 2] {
            *value = CDF_PROB_SCALE;
        }
        cdf[N - 2] = tile_cdf[N - 2];
        cdf[N - 1] = 0;
    }
    let shift = u32::from(num_log2);
    for i in 0..N - 2 {
        cdf[i] -= (CDF_PROB_SCALE - tile_cdf[i]) >> shift;
    }
    cdf[N - 1] += tile_cdf[N - 1] >> shift;
}

pub(in crate::bitstream::tile_payload::cdf) fn blend_cdf_row<const N: usize>(
    cdf: &mut [i32; N],
    saved_cdf: &[i32; N],
) {
    for i in 0..N - 2 {
        cdf[i] = CDF_PROB_SCALE
            - (((CDF_PROB_SCALE - saved_cdf[i]) + 7 * (CDF_PROB_SCALE - cdf[i]) + 4) >> 3);
    }
    cdf[N - 1] = (saved_cdf[N - 1] + 7 * cdf[N - 1] + 4) >> 3;
}

pub(in crate::bitstream::tile_payload::cdf) fn scale_cdf_count<const N: usize>(cdf: &mut [i32; N]) {
    cdf[N - 1] = cdf[N - 1].saturating_mul(3) >> 2;
}

pub(in crate::bitstream::tile_payload::cdf) fn avg_cdf_rows<'a, 'b, const N: usize>(
    frame: impl Iterator<Item = &'a mut [i32; N]>,
    tile: impl Iterator<Item = &'b [i32; N]>,
    tile_num: u32,
    num_log2: u8,
) {
    for (frame_row, tile_row) in frame.zip(tile) {
        avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
    }
}

pub(in crate::bitstream::tile_payload::cdf) fn blend_cdf_rows<'a, 'b, const N: usize>(
    rows: impl Iterator<Item = &'a mut [i32; N]>,
    saved: impl Iterator<Item = &'b [i32; N]>,
) {
    for (row, saved_row) in rows.zip(saved) {
        blend_cdf_row(row, saved_row);
    }
}

pub(in crate::bitstream::tile_payload::cdf) fn scale_cdf_rows<'a, const N: usize>(
    rows: impl Iterator<Item = &'a mut [i32; N]>,
) {
    for row in rows {
        scale_cdf_count(row);
    }
}

pub(crate) const fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}
