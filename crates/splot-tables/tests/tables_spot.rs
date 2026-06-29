// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Spot checks for the shared generated AV2 § 9 tables in `splot-tables` (the
//! § 9.6/§ 9.7 transform kernels, the § 9.4 quantizer matrix, and shared § 9.8
//! loop-restoration tables; feature
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
    let t = &tables::quantizer::QM_OFFSET;
    assert_eq!(t.len(), 25);
    assert_eq!(&t[..8], &[0, 16, 80, 336, 336, 1360, 1392, 1424]);
    assert_eq!(t[24], 3472);
}

#[test]
fn quantizer_matrix_luma_4x4_matches_mirror() {
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

#[test]
fn loop_restoration_pc_wiener_tables_match_mirror() {
    let lut = &tables::loop_restoration::PC_WIENER_LUT_TO_CLASS;
    assert_eq!(lut.len(), 4096);
    assert_eq!(
        &lut[..16],
        &[
            83, 154, 254, 125, 125, 125, 253, 253, 77, 200, 207, 30, 30, 239, 239, 239,
        ]
    );

    assert_eq!(
        tables::loop_restoration::PC_WIENER_FILTERS[0][0],
        [73, 127, -20, -30, -38, -29, 10, 7, -1, -3, 1, 7, -208]
    );
}
