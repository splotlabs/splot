// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
    let pos = row.saturating_mul(8).saturating_add(col);
    CoeffScanEntry::new(scan_index, pos, row, col)
}

fn config(plane: usize, tx_class: TransformClass, is_hidden: bool) -> CoeffMaxLevelConfig {
    CoeffMaxLevelConfig {
        plane,
        tx_class,
        is_hidden,
    }
}

#[test]
fn coefficient_max_level_derives_low_frequency_limits() {
    let cases = [
        (0, TransformClass::TwoD, entry(3, 0, 3), true, 8),
        (0, TransformClass::TwoD, entry(3, 2, 2), false, 6),
        (1, TransformClass::TwoD, entry(3, 0, 0), true, 5),
        (1, TransformClass::TwoD, entry(3, 0, 1), false, 6),
        (0, TransformClass::Horizontal, entry(3, 7, 1), true, 8),
        (0, TransformClass::Horizontal, entry(3, 0, 2), false, 6),
        (2, TransformClass::Horizontal, entry(3, 7, 0), true, 5),
        (2, TransformClass::Horizontal, entry(3, 0, 1), false, 6),
        (0, TransformClass::Vertical, entry(3, 1, 7), true, 8),
        (0, TransformClass::Vertical, entry(3, 2, 0), false, 6),
        (2, TransformClass::Vertical, entry(3, 0, 7), true, 5),
        (2, TransformClass::Vertical, entry(3, 1, 0), false, 6),
    ];

    for (plane, tx_class, entry, is_low_frequency, max_level) in cases {
        assert_eq!(
            coeff_is_low_frequency(entry, plane, tx_class),
            is_low_frequency
        );
        assert_eq!(
            derive_coeff_max_level(entry, config(plane, tx_class, false)),
            max_level
        );
    }
}

#[test]
fn coefficient_max_level_hidden_final_scan_entry_overrides_limit() {
    let hidden = derive_coeff_max_level(entry(0, 31, 31), config(0, TransformClass::TwoD, true));
    assert_eq!(hidden, 3);

    let not_final = derive_coeff_max_level(entry(1, 31, 31), config(0, TransformClass::TwoD, true));
    assert_eq!(not_final, 6);
}

#[test]
fn coefficient_max_level_pathological_coordinates_are_total() {
    let derived = derive_coeff_max_level(
        CoeffScanEntry::new(7, usize::MAX, usize::MAX, usize::MAX),
        config(0, TransformClass::TwoD, false),
    );

    assert!(!coeff_is_low_frequency(
        CoeffScanEntry::new(7, usize::MAX, usize::MAX, usize::MAX),
        0,
        TransformClass::TwoD
    ));
    assert_eq!(derived, 6);
}
