// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Hashes of 8x8 sample blocks at every position: a polynomial hash of each
//! eight-sample row run, then of eight row hashes down a column.

use std::simd::Simd;
use std::simd::num::SimdUint as _;

/// The block side the hashes cover.
pub(crate) const HASH_SIDE: usize = 8;
const LANES: usize = 8;
const ROW_BASE: u32 = 0x9E37_79B1;
const COLUMN_BASE: u32 = 0x85EB_CA77;

/// `base^i` for `i` in `0..HASH_SIDE`, wrapping.
const fn powers(base: u32) -> [u32; HASH_SIDE] {
    let mut out = [1u32; HASH_SIDE];
    let mut i = 1;
    while i < HASH_SIDE {
        out[i] = out[i - 1].wrapping_mul(base);
        i += 1;
    }
    out
}

const ROW_POWERS: [u32; HASH_SIDE] = powers(ROW_BASE);
const COLUMN_POWERS: [u32; HASH_SIDE] = powers(COLUMN_BASE);

/// Writes to `out[x]` the hash of `row[x..x + 8]` for every `x` that fits
/// both, wrapping: `Σ row[x + i] · ROW_BASE^i`.
#[cfg(test)]
pub(crate) fn row_hashes_scalar(row: &[u16], out: &mut [u32]) {
    for (x, out) in out
        .iter_mut()
        .enumerate()
        .take((row.len() + 1).saturating_sub(HASH_SIDE))
    {
        *out = row[x..x + HASH_SIDE]
            .iter()
            .zip(ROW_POWERS)
            .fold(0u32, |hash, (&sample, power)| {
                hash.wrapping_add(u32::from(sample).wrapping_mul(power))
            });
    }
}

/// [`row_hashes_scalar`] over eight positions per step.
pub(crate) fn row_hashes_simd(row: &[u16], out: &mut [u32]) {
    let count = (row.len() + 1).saturating_sub(HASH_SIDE).min(out.len());
    let mut x = 0;
    while x + LANES <= count {
        let mut hash = Simd::<u32, LANES>::splat(0);
        for (i, power) in ROW_POWERS.into_iter().enumerate() {
            let samples: Simd<u32, LANES> = Simd::<u16, LANES>::from_slice(&row[x + i..]).cast();
            hash += samples * Simd::splat(power);
        }
        hash.copy_to_slice(&mut out[x..x + LANES]);
        x += LANES;
    }
    for (x, out) in out.iter_mut().enumerate().take(count).skip(x) {
        *out = row[x..x + HASH_SIDE]
            .iter()
            .zip(ROW_POWERS)
            .fold(0u32, |hash, (&sample, power)| {
                hash.wrapping_add(u32::from(sample).wrapping_mul(power))
            });
    }
}

/// Writes to `out[x]` the block hash `Σ rows[j][x] · COLUMN_BASE^j` over
/// the eight row-hash runs `rows`, for every `x` all of them and `out` hold.
#[cfg(test)]
pub(crate) fn column_hashes_scalar(rows: [&[u32]; HASH_SIDE], out: &mut [u32]) {
    let count = rows.iter().map(|row| row.len()).min().unwrap_or(0);
    for (x, out) in out.iter_mut().enumerate().take(count) {
        *out = rows
            .iter()
            .zip(COLUMN_POWERS)
            .fold(0u32, |hash, (row, power)| {
                hash.wrapping_add(row[x].wrapping_mul(power))
            });
    }
}

/// [`column_hashes_scalar`] over eight positions per step.
pub(crate) fn column_hashes_simd(rows: [&[u32]; HASH_SIDE], out: &mut [u32]) {
    let count = rows
        .iter()
        .map(|row| row.len())
        .min()
        .unwrap_or(0)
        .min(out.len());
    let mut x = 0;
    while x + LANES <= count {
        let mut hash = Simd::<u32, LANES>::splat(0);
        for (row, power) in rows.iter().zip(COLUMN_POWERS) {
            hash += Simd::<u32, LANES>::from_slice(&row[x..]) * Simd::splat(power);
        }
        hash.copy_to_slice(&mut out[x..x + LANES]);
        x += LANES;
    }
    for (x, out) in out.iter_mut().enumerate().take(count).skip(x) {
        *out = rows
            .iter()
            .zip(COLUMN_POWERS)
            .fold(0u32, |hash, (row, power)| {
                hash.wrapping_add(row[x].wrapping_mul(power))
            });
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        /// Rows of odd lengths with repeats and full 10-bit values.
        #[test]
        fn row_hashes_simd_match_scalar(
            row in prop::collection::vec(prop_oneof![3 => 40u16..44, 1 => 0u16..1024], 0..60),
        ) {
            let (mut scalar, mut simd) = (vec![7; row.len()], vec![7; row.len()]);
            row_hashes_scalar(&row, &mut scalar);
            row_hashes_simd(&row, &mut simd);
            prop_assert_eq!(scalar, simd);
        }

        /// Eight row-hash runs of different lengths with wrapping values.
        #[test]
        fn column_hashes_simd_match_scalar(
            runs in prop::collection::vec(prop::collection::vec(any::<u32>(), 20..40), 8),
        ) {
            let rows: [&[u32]; HASH_SIDE] = core::array::from_fn(|j| runs[j].as_slice());
            let (mut scalar, mut simd) = (vec![3; 40], vec![3; 40]);
            column_hashes_scalar(rows, &mut scalar);
            column_hashes_simd(rows, &mut simd);
            prop_assert_eq!(scalar, simd);
        }
    }

    #[test]
    fn equal_blocks_hash_alike_and_a_changed_sample_does_not() {
        let rows = |changed: bool| -> Vec<u16> {
            (0..16 * 16)
                .map(|i| ((i % 16) * 37 % 11) as u16 + u16::from(changed && i == 3 * 16 + 4))
                .collect()
        };
        let plane = rows(false);
        let hash_at = |plane: &[u16], x: usize, y: usize| {
            let rows: Vec<Vec<u32>> = (y..y + 8)
                .map(|row| {
                    let mut out = vec![0; 16];
                    row_hashes_simd(&plane[row * 16..row * 16 + 16], &mut out);
                    out
                })
                .collect();
            let refs: [&[u32]; HASH_SIDE] = core::array::from_fn(|j| rows[j].as_slice());
            let mut out = vec![0; 9];
            column_hashes_simd(refs, &mut out);
            out[x]
        };
        assert_eq!(hash_at(&plane, 1, 0), hash_at(&plane, 1, 5));
        let changed = rows(true);
        assert_ne!(hash_at(&plane, 1, 0), hash_at(&changed, 1, 0));
    }
}
