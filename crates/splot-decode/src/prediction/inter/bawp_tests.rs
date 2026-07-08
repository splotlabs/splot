// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::bawp_template_counts;

#[test]
fn template_counts_follow_the_clamped_size_table() {
    for (case, expected) in [
        ((16, 16, true, true, true), (16, 16, 16, 16)),
        ((12, 16, true, true, true), (8, 16, 8, 8)),
        ((16, 12, true, true, true), (16, 8, 8, 8)),
        ((4, 4, true, true, true), (4, 4, 4, 4)),
        ((16, 4, true, true, true), (16, 4, 16, 0)),
        ((4, 16, true, true, true), (4, 16, 0, 16)),
        ((32, 8, true, true, false), (16, 8, 16, 0)),
        ((8, 32, true, false, true), (8, 16, 0, 16)),
        ((32, 32, false, true, true), (8, 8, 8, 8)),
        ((12, 12, false, true, true), (8, 8, 8, 8)),
        ((64, 64, true, false, false), (16, 16, 0, 0)),
    ] {
        let (bw, bh, luma, up, left) = case;
        assert_eq!(
            bawp_template_counts(bw, bh, luma, up, left),
            expected,
            "bw={bw} bh={bh} luma={luma} up={up} left={left}"
        );
    }
}
