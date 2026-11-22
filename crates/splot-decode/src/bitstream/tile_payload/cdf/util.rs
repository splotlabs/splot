// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDF selection-bounds validation and row lifecycle helpers.

use splot_core::symbol::CDF_COUNT_SHIFT;

use super::layout::{RowLenBounds, rows_of_len, rows_of_len_mut};
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
    cdf: &mut [u16; N],
    tile_cdf: &[u16; N],
    tile_num: u32,
    num_log2: u8,
) {
    if tile_num == 0 {
        for value in &mut cdf[..N - 2] {
            *value = CDF_PROB_SCALE as u16;
        }
        cdf[N - 2] = tile_cdf[N - 2] % COUNT_STEP;
    }
    let shift = u32::from(num_log2);
    for i in 0..N - 2 {
        cdf[i] -= ((CDF_PROB_SCALE - u32::from(tile_cdf[i])) >> shift) as u16;
    }
    cdf[N - 2] += (tile_cdf[N - 2] >> CDF_COUNT_SHIFT >> shift) * COUNT_STEP;
}

const COUNT_STEP: u16 = 1 << CDF_COUNT_SHIFT;

/// Sets the use count of a row's adaptation state.
fn set_count<const N: usize>(cdf: &mut [u16; N], count: u16) {
    cdf[N - 2] = cdf[N - 2] % COUNT_STEP + count * COUNT_STEP;
}

pub(in crate::bitstream::tile_payload::cdf) fn blend_cdf_row<const N: usize>(
    cdf: &mut [u16; N],
    saved_cdf: &[u16; N],
) {
    for i in 0..N - 2 {
        let saved = u32::from(saved_cdf[i]);
        let current = u32::from(cdf[i]);
        cdf[i] = (CDF_PROB_SCALE
            - (((CDF_PROB_SCALE - saved) + 7 * (CDF_PROB_SCALE - current) + 4) >> 3))
            as u16;
    }
    let count = |row: &[u16; N]| u32::from(row[N - 2] >> CDF_COUNT_SHIFT);
    set_count(cdf, ((count(saved_cdf) + 7 * count(cdf) + 4) >> 3) as u16);
}

pub(in crate::bitstream::tile_payload::cdf) fn scale_cdf_count<const N: usize>(cdf: &mut [u16; N]) {
    set_count(cdf, ((cdf[N - 2] >> CDF_COUNT_SHIFT) * 3) >> 2);
}

pub(in crate::bitstream::tile_payload::cdf) fn avg_cdf_rows<const N: usize>(
    frame: &mut [[u16; N]],
    tile: &[[u16; N]],
    tile_num: u32,
    num_log2: u8,
) {
    for (frame_row, tile_row) in frame.iter_mut().zip(tile) {
        avg_cdf_row(frame_row, tile_row, tile_num, num_log2);
    }
}

pub(in crate::bitstream::tile_payload::cdf) fn blend_cdf_rows<const N: usize>(
    rows: &mut [[u16; N]],
    saved: &[[u16; N]],
) {
    for (row, saved_row) in rows.iter_mut().zip(saved) {
        blend_cdf_row(row, saved_row);
    }
}

pub(in crate::bitstream::tile_payload::cdf) fn scale_cdf_rows<const N: usize>(
    rows: &mut [[u16; N]],
) {
    for row in rows {
        scale_cdf_count(row);
    }
}

/// [`avg_cdf_rows`] over every row of two stores of one layout.
pub(in crate::bitstream::tile_payload::cdf) fn avg_cdf_store(
    frame: &mut [u16],
    tile: &[u16],
    bounds: &RowLenBounds,
    tile_num: u32,
    num_log2: u8,
) {
    macro_rules! avg {
        ($len:literal) => {
            avg_cdf_rows::<$len>(
                rows_of_len_mut(frame, bounds),
                rows_of_len(tile, bounds),
                tile_num,
                num_log2,
            );
        };
    }
    for_each_cdf_row_len!(avg);
}

/// [`blend_cdf_rows`] over every row of two stores of one layout.
pub(in crate::bitstream::tile_payload::cdf) fn blend_cdf_store(
    store: &mut [u16],
    saved: &[u16],
    bounds: &RowLenBounds,
) {
    macro_rules! blend {
        ($len:literal) => {
            blend_cdf_rows::<$len>(rows_of_len_mut(store, bounds), rows_of_len(saved, bounds));
        };
    }
    for_each_cdf_row_len!(blend);
}

/// [`scale_cdf_rows`] over every row of a store.
pub(in crate::bitstream::tile_payload::cdf) fn scale_cdf_store(
    store: &mut [u16],
    bounds: &RowLenBounds,
) {
    macro_rules! scale {
        ($len:literal) => {
            scale_cdf_rows::<$len>(rows_of_len_mut(store, bounds));
        };
    }
    for_each_cdf_row_len!(scale);
}

/// [`cdf_rows_valid`] over every row of a store.
#[cfg(any(debug_assertions, fuzzing, test))]
pub(in crate::bitstream::tile_payload::cdf) fn cdf_store_valid(
    store: &[u16],
    bounds: &RowLenBounds,
) -> bool {
    let mut valid = true;
    macro_rules! check {
        ($len:literal) => {
            valid &= cdf_rows_valid::<$len>(rows_of_len(store, bounds));
        };
    }
    for_each_cdf_row_len!(check);
    valid
}

/// Whether every row holds non-decreasing probabilities up to
/// `CDF_PROB_SCALE` and a use count up to 32: the § 8.2.6 invariants the
/// trusted symbol reads rely on.
#[cfg(any(debug_assertions, fuzzing, test))]
pub(in crate::bitstream::tile_payload::cdf) fn cdf_rows_valid<const N: usize>(
    rows: &[[u16; N]],
) -> bool {
    rows.iter().all(|row| {
        let probabilities = &row[..N - 2];
        probabilities.is_sorted()
            && probabilities
                .last()
                .is_none_or(|&last| u32::from(last) <= CDF_PROB_SCALE)
            && row[N - 2] >> CDF_COUNT_SHIFT <= 32
    })
}

pub(crate) const fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}
