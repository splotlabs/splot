// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::super::max_level::coeff_is_low_frequency;
use super::*;

#[test]
fn low_frequency_positions_are_a_scan_prefix() {
    use splot_recon::{TransformClass, coefficient_scan_slice};
    fn check<const CLASS: usize, const LUMA: bool>(class: TransformClass) {
        let tx_class = [
            CoeffTransformClass::TwoD,
            CoeffTransformClass::Horizontal,
            CoeffTransformClass::Vertical,
        ][CLASS];
        for width_log2 in 2..=5u32 {
            for height in [4, 8, 16, 32] {
                let width = 1usize << width_log2;
                let scan = coefficient_scan_slice(width, height, class).unwrap();
                let end = lf_scan_end::<CLASS, LUMA>(width, height);
                for (scan_index, &pos) in scan.iter().enumerate() {
                    let pos = usize::from(pos);
                    let entry =
                        CoeffScanEntry::new(scan_index, pos, pos >> width_log2, pos % width);
                    let is_lf = coeff_is_low_frequency(entry, usize::from(!LUMA), tx_class);
                    assert_eq!(is_lf, scan_index < end, "{CLASS} {LUMA} {width}x{height}");
                }
            }
        }
    }
    check::<0, true>(TransformClass::TwoD);
    check::<0, false>(TransformClass::TwoD);
    check::<1, true>(TransformClass::Horizontal);
    check::<1, false>(TransformClass::Horizontal);
    check::<2, true>(TransformClass::Vertical);
    check::<2, false>(TransformClass::Vertical);
}

#[test]
fn grid_scans_hold_each_shape_scan_with_its_grid_index() {
    for (class, recon_class) in [
        (CoeffTransformClass::TwoD, TransformClass::TwoD),
        (CoeffTransformClass::Horizontal, TransformClass::Horizontal),
        (CoeffTransformClass::Vertical, TransformClass::Vertical),
    ] {
        for width in [4, 8, 16, 32] {
            for height in [4, 8, 16, 32] {
                let scan = splot_recon::coefficient_scan_slice(width, height, recon_class).unwrap();
                let grid = grid_scan(class, width, height, scan.len()).unwrap();
                for (&pos, &entry) in scan.iter().zip(grid) {
                    let pos = usize::from(pos);
                    assert_eq!(entry >> 16, pos as u32);
                    let expected = (pos / width) * LEVEL_STRIDE + pos % width;
                    assert_eq!(entry_grid(entry), expected);
                }
            }
        }
    }
}

#[test]
fn grid_scan_rejects_bad_shapes_and_eobs() {
    let class = CoeffTransformClass::TwoD;
    assert!(grid_scan(class, 12, 4, 1).is_err());
    assert!(grid_scan(class, 4, 64, 1).is_err());
    assert!(grid_scan(class, 4, 4, 0).is_err());
    assert!(grid_scan(class, 4, 4, 17).is_err());
    assert_eq!(grid_scan(class, 4, 4, 16).unwrap().len(), 16);
}
