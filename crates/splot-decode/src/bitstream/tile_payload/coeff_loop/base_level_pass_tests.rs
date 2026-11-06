// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn nonzero_entries_round_trip_their_fields() {
    for (pos, scan_index, level, tcq_q0, is_lf) in [
        (0, 0, 1, 0, true),
        (1023, 1023, 255, 1, false),
        (37, 600, 8, 1, true),
    ] {
        let entry =
            NonzeroEntry::from_raw(NonzeroEntry::new(pos, scan_index, level, tcq_q0, is_lf).0);
        assert_eq!(entry.pos(), pos);
        assert_eq!(entry.scan_index(), scan_index);
        assert_eq!(entry.level(), level);
        assert_eq!(entry.tcq_q0(), tcq_q0 == 1);
        assert_eq!(entry.is_lf(), is_lf);
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
