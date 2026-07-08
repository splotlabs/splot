// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn entry(scan_index: usize, row: usize, col: usize) -> CoeffScanEntry {
    let pos = row.saturating_mul(8).saturating_add(col);
    CoeffScanEntry::for_test(scan_index, pos, row, col)
}

fn config(plane: usize, tx_class: CoeffTransformClass, is_hidden: bool) -> CoeffMaxLevelConfig {
    CoeffMaxLevelConfig {
        plane,
        tx_class,
        is_hidden,
    }
}

fn plane_tx_config(
    plane: usize,
    plane_tx_type: usize,
    is_hidden: bool,
) -> CoeffMaxLevelPlaneTxTypeConfig {
    CoeffMaxLevelPlaneTxTypeConfig {
        plane,
        plane_tx_type,
        is_hidden,
    }
}

#[test]
fn coefficient_transform_class_derives_from_plane_tx_type() {
    for plane_tx_type in [10, 12, 14] {
        assert_eq!(
            CoeffTransformClass::from_plane_tx_type(plane_tx_type),
            CoeffTransformClass::Vertical
        );
    }
    for plane_tx_type in [11, 13, 15] {
        assert_eq!(
            CoeffTransformClass::from_plane_tx_type(plane_tx_type),
            CoeffTransformClass::Horizontal
        );
    }
    for plane_tx_type in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 16, usize::MAX] {
        assert_eq!(
            CoeffTransformClass::from_plane_tx_type(plane_tx_type),
            CoeffTransformClass::TwoD
        );
    }
}

#[test]
fn coefficient_max_level_derives_low_frequency_limits() {
    let cases = [
        (0, CoeffTransformClass::TwoD, entry(3, 0, 3), true, 8),
        (0, CoeffTransformClass::TwoD, entry(3, 2, 2), false, 6),
        (1, CoeffTransformClass::TwoD, entry(3, 0, 0), true, 5),
        (1, CoeffTransformClass::TwoD, entry(3, 0, 1), false, 6),
        (0, CoeffTransformClass::Horizontal, entry(3, 7, 1), true, 8),
        (0, CoeffTransformClass::Horizontal, entry(3, 0, 2), false, 6),
        (2, CoeffTransformClass::Horizontal, entry(3, 7, 0), true, 5),
        (2, CoeffTransformClass::Horizontal, entry(3, 0, 1), false, 6),
        (0, CoeffTransformClass::Vertical, entry(3, 1, 7), true, 8),
        (0, CoeffTransformClass::Vertical, entry(3, 2, 0), false, 6),
        (2, CoeffTransformClass::Vertical, entry(3, 0, 7), true, 5),
        (2, CoeffTransformClass::Vertical, entry(3, 1, 0), false, 6),
    ];

    for (plane, tx_class, entry, is_low_frequency, max_level) in cases {
        let derived = derive_coeff_max_level(entry, config(plane, tx_class, false));
        assert_eq!(derived.entry, entry);
        assert_eq!(derived.is_low_frequency, is_low_frequency);
        assert_eq!(derived.max_level, max_level);
    }
}

#[test]
fn coefficient_max_level_hidden_final_scan_entry_overrides_limit() {
    let hidden =
        derive_coeff_max_level(entry(0, 31, 31), config(0, CoeffTransformClass::TwoD, true));
    assert!(!hidden.is_low_frequency);
    assert_eq!(hidden.max_level, 3);

    let not_final =
        derive_coeff_max_level(entry(1, 31, 31), config(0, CoeffTransformClass::TwoD, true));
    assert_eq!(not_final.max_level, 6);
}

#[test]
fn coefficient_max_level_builds_quant_pass_inputs_in_walk_order() -> Result<(), CoeffMaxLevelError>
{
    let entries = vec![entry(2, 0, 3), entry(1, 4, 4), entry(0, 0, 0)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(entries.clone());

    let levels =
        derive_nonzero_coeff_max_levels(&walk, config(0, CoeffTransformClass::TwoD, true))?;
    let inputs = max_levels_to_quant_pass_inputs(&levels)?;

    assert_eq!(levels.len(), 3);
    assert_eq!(inputs.len(), 3);
    assert_eq!(inputs[0].entry, entries[0]);
    assert_eq!(inputs[0].max_level, 8);
    assert_eq!(inputs[1].entry, entries[1]);
    assert_eq!(inputs[1].max_level, 6);
    assert_eq!(inputs[2].entry, entries[2]);
    assert_eq!(inputs[2].max_level, 3);

    Ok(())
}

#[test]
fn coefficient_max_level_plane_tx_type_handoff_matches_direct_config()
-> Result<(), CoeffMaxLevelError> {
    let entries = vec![
        entry(3, 0, 3),
        entry(2, 7, 1),
        entry(1, 1, 7),
        entry(0, 31, 31),
    ];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(entries);
    let cases = [
        (0, 0, CoeffTransformClass::TwoD),
        (0, 10, CoeffTransformClass::Vertical),
        (2, 11, CoeffTransformClass::Horizontal),
        (2, usize::MAX, CoeffTransformClass::TwoD),
    ];

    for (plane, plane_tx_type, tx_class) in cases {
        let direct = derive_nonzero_coeff_max_levels(&walk, config(plane, tx_class, true))?;
        let derived = derive_nonzero_coeff_max_levels_from_plane_tx_type(
            &walk,
            plane_tx_config(plane, plane_tx_type, true),
        )?;
        assert_eq!(derived, direct, "PlaneTxType {plane_tx_type}");
    }

    Ok(())
}

#[test]
fn coefficient_max_level_pathological_coordinates_are_total() {
    let derived = derive_coeff_max_level(
        CoeffScanEntry::for_test(7, usize::MAX, usize::MAX, usize::MAX),
        config(0, CoeffTransformClass::TwoD, false),
    );

    assert!(!derived.is_low_frequency);
    assert_eq!(derived.max_level, 6);
}
