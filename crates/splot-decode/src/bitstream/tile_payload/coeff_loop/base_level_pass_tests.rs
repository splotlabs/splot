// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn level_masks_list_nonzero_scan_indices_in_walk_order() {
    let mut masks = CoeffLevelMasks::default();
    let levels = [
        (1023, 2, 6),
        (700, 0, 3),
        (64, 1, 2),
        (63, 3, 0),
        (5, 1, 7),
        (0, 0, 1),
    ];
    for (scan_index, level, tcq_state) in levels {
        masks.record(scan_index, level, (tcq_state >> 1) & 1);
    }

    let visited: Vec<usize> = masks.nonzero_scan_indices(1024, false).collect();
    assert_eq!(visited, [1023, 64, 63, 5]);
    let with_dc: Vec<usize> = masks.nonzero_scan_indices(1024, true).collect();
    assert_eq!(with_dc, [1023, 64, 63, 5, 0]);
    let short: Vec<usize> = masks.nonzero_scan_indices(64, true).collect();
    assert_eq!(short, [63, 5, 0]);

    for (scan_index, _, tcq_state) in levels {
        assert_eq!(masks.tcq_q0(scan_index), tcq_state & 2 != 0, "{scan_index}");
    }
}

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
