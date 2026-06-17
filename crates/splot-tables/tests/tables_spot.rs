// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Spot checks for the shared generated AV2 § 9 transform-kernel tables
//! (feature `AV2-9-ADDITIONAL-TABLES`, crate `splot-tables`).
//!
//! These assert values from the `cargo xtask gen-tables` output against the
//! *other* rendering of the same spec — the committed mirror's § 9 Markdown text
//! — so the cross-check is independent of the generator's own parsing of
//! `all_tables.h`. Each assertion cites the mirror file and line range it was
//! read from. The matching cross-check for the in-`splot-core` § 9 tables lives
//! in `crates/splot-core/tests/tables_spot.rs`.

use splot_tables::tables;

#[test]
fn transform_1d_adst_kernel4_matches_mirror() {
    // docs/spec/av2/1.0.0/09-additional-tables/09-06-1d-transform-tables.md
    // lines 195-202 (Adst_Kernel4[4][4]).
    assert_eq!(
        tables::transform_1d::ADST_KERNEL4,
        [
            [18, 50, 75, 89],   // line 197
            [50, 89, 18, -75],  // line 198
            [75, 18, -89, 50],  // line 199
            [89, -75, 50, -18], // line 200
        ]
    );
}
