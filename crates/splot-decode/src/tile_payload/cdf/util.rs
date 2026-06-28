// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CDF selection-bounds validation and per-tile CDF averaging helpers, split out
//! of `super` to keep the selection-boundary source under the §5 hard cap.

use super::{
    CDF_PROB_SCALE, DO_SPLIT_PLANE_CONTEXTS, DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS, TileCdfArray,
    TileCdfError,
};

pub(super) fn checked_plane(
    array: TileCdfArray,
    plane_start: usize,
) -> Result<usize, TileCdfError> {
    checked_plane_within(array, plane_start, DO_SPLIT_PLANE_CONTEXTS)
}

/// § 8.3.2 fixes `do_square_split` `PlaneStart` at 0 (the chroma partition is
/// forced for the large block sizes where it is read), so only plane 0 is valid
/// for that selector — tighter than the shared 2-plane partition CDF array bound.
pub(super) fn checked_square_split_plane(plane_start: usize) -> Result<usize, TileCdfError> {
    checked_plane_within(
        TileCdfArray::DoSquareSplit,
        plane_start,
        DO_SQUARE_SPLIT_VALID_PLANE_CONTEXTS,
    )
}

pub(super) fn checked_plane_within(
    array: TileCdfArray,
    plane_start: usize,
    max_exclusive: usize,
) -> Result<usize, TileCdfError> {
    if plane_start >= max_exclusive {
        return Err(TileCdfError::SelectorOutOfRange {
            array,
            index_name: "plane_start",
            actual: plane_start,
            max_exclusive,
        });
    }
    Ok(plane_start)
}

pub(super) fn checked_context(
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

pub(super) const fn tx_partition_type_array(reduced: bool) -> TileCdfArray {
    if reduced {
        TileCdfArray::TxPartitionTypeReduced
    } else {
        TileCdfArray::TxPartitionType
    }
}

pub(in crate::tile_payload::cdf) fn avg_cdf_row<const N: usize>(
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

pub(in crate::tile_payload::cdf) fn scale_cdf_count<const N: usize>(cdf: &mut [i32; N]) {
    cdf[N - 1] = cdf[N - 1].saturating_mul(3) >> 2;
}

pub(super) const fn floor_log2(value: u32) -> u32 {
    u32::BITS - 1 - value.leading_zeros()
}
