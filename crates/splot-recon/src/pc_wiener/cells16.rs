// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Source chunks and the 16-lane PC-Wiener row group on [`lr_madd`] tap
//! pairs.

use super::{
    PC_WIENER_BLOCK_SIZE, PC_WIENER_CONFIG, PC_WIENER_FILTER_TAP_RADIUS, PC_WIENER_FILTERS,
    PC_WIENER_FULL_CLASSES, PC_WIENER_PREC_BITS, PC_WIENER_WINDOW_ROWS,
};
use crate::lr_madd::{self, MaddPattern, MaddWeights, TapPairs};
use crate::wienerns_filter::LumaSimdSource;

/// The § 7.20.4 taps for [`lr_madd`].
pub(super) struct PcTaps;

impl TapPairs for PcTaps {
    #[cfg(target_arch = "x86_64")]
    lr_madd::madd_avx2!(PcTaps, 7, 22);
    const RADIUS: usize = PC_WIENER_FILTER_TAP_RADIUS;
    const TAPS: &'static [(isize, isize)] = &PC_WIENER_CONFIG;
}

/// `pmaddwd` pairs of one class: taps `2k` and `2k + 1`, then tap 12 folded
/// with the `1 << PC_WIENER_PREC_BITS` center unit and the rounding offset.
pub(super) type ClassPairs = [MaddPattern; 7];

/// [`ClassPairs`] of every class of every § 9.8 `Pc_Wiener_Filters` set; each
/// tap is narrowed to `i16`, which holds the `-208..=127` table range.
pub(super) static PC_WIENER_PAIRS: std::sync::LazyLock<Vec<[ClassPairs; PC_WIENER_FULL_CLASSES]>> =
    std::sync::LazyLock::new(|| {
        PC_WIENER_FILTERS
            .iter()
            .map(|set| {
                set.map(|coeffs| {
                    core::array::from_fn(|pair| {
                        let [low, high] = if pair == 6 {
                            [
                                coeffs[12] + (1 << PC_WIENER_PREC_BITS),
                                1 << (PC_WIENER_PREC_BITS - 1),
                            ]
                        } else {
                            [coeffs[2 * pair], coeffs[2 * pair + 1]]
                        };
                        let [low, high] = [low as i16, high as i16];
                        [low, high, low, high, low, high, low, high]
                    })
                })
            })
            .collect()
    });

/// The `REACH` columns from `col` of each of `ROWS` padded source rows.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn chunk_rows<'a, T, const REACH: usize, const ROWS: usize>(
    rows: &[&'a [T]; ROWS],
    col: usize,
) -> Option<[&'a [T; REACH]; ROWS]> {
    let first = rows.first()?.get(col..)?.first_chunk::<REACH>()?;
    let mut chunks = [first; ROWS];
    for (slot, row) in chunks.iter_mut().zip(rows) {
        *slot = row.get(col..)?.first_chunk::<REACH>()?;
    }
    Some(chunks)
}

/// Writes the weights of the four cells `classes` in the kernel's
/// arrangement.
#[inline]
pub(super) fn group_weights<T: LumaSimdSource>(
    weights: &mut [MaddWeights; 7],
    pairs: &[ClassPairs; PC_WIENER_FULL_CLASSES],
    classes: [u8; 4],
) {
    let cells = classes.map(|class| &pairs[usize::from(class) % PC_WIENER_FULL_CLASSES]);
    for (pair, weights) in weights.iter_mut().enumerate() {
        lr_madd::arrange_into::<T>(
            weights,
            [
                &cells[0][pair],
                &cells[1][pair],
                &cells[2][pair],
                &cells[3][pair],
            ],
        );
    }
}

/// The first `rows` rows of a 4x4-cell row group of one 16-column group.
#[allow(clippy::inline_always)]
#[inline(always)]
pub(super) fn filter_pc_wiener_group16<T: LumaSimdSource>(
    output: &mut [u16],
    (stride, rows): (usize, usize),
    lanes: &[&[T; 22]; PC_WIENER_BLOCK_SIZE + 2 * PC_WIENER_FILTER_TAP_RADIUS],
    weights: &[MaddWeights; 7],
    max_sample: u16,
) {
    for k in 0..rows.min(PC_WIENER_BLOCK_SIZE) {
        let Some(window) = lanes[k..].first_chunk::<PC_WIENER_WINDOW_ROWS>() else {
            return;
        };
        let values =
            lr_madd::filter16::<PcTaps, T, PC_WIENER_WINDOW_ROWS, 22>(window, weights, max_sample);
        if let Some(slot) = output
            .get_mut(k * stride..)
            .and_then(<[u16]>::first_chunk_mut::<16>)
        {
            *slot = values;
        }
    }
}
