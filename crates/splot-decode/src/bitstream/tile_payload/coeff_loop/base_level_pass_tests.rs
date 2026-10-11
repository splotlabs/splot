// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

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
        masks.record(scan_index, level, tcq_state);
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
