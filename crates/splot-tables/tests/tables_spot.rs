// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Spot checks for the shared generated AV2 § 9 tables in `splot-tables` (the
//! § 9.6/§ 9.7 transform kernels and the § 9.4 quantizer matrix; feature
//! `AV2-9-ADDITIONAL-TABLES`).
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

#[test]
fn quantizer_qm_offset_matches_mirror() {
    // docs/spec/av2/1.0.0/09-additional-tables/09-04-quantizer-matrix-tables.md
    // lines 90-97 (Qm_Offset[ TX_SIZES_ALL ]).
    let t = &tables::quantizer::QM_OFFSET;
    assert_eq!(t.len(), 25);
    // line 92: "0, 16, 80, 336, 336, 1360, 1392, 1424,".
    assert_eq!(&t[..8], &[0, 16, 80, 336, 336, 1360, 1392, 1424]);
    // line 95: "3472," (the final element).
    assert_eq!(t[24], 3472);
}

#[test]
fn quantizer_matrix_luma_4x4_matches_mirror() {
    // 09-04-quantizer-matrix-tables.md line 107, the first (lvl 0) luma matrix,
    // "Size 4x4" run: "32, 43, 73, 97, 43, 67, 94, 110, 73, 94, 137, 150, 97,
    // 110, 150, 200, ...".
    let m = &tables::quantizer::QUANTIZER_MATRIX; // [15][2][3600]
    assert_eq!(m.len(), 15);
    assert_eq!(m[0].len(), 2);
    assert_eq!(m[0][0].len(), 3600);
    assert_eq!(
        &m[0][0][..16],
        &[
            32, 43, 73, 97, 43, 67, 94, 110, 73, 94, 137, 150, 97, 110, 150, 200
        ]
    );
}
