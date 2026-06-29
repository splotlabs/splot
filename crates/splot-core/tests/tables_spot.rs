// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Spot checks for the generated AV2 § 9 tables (feature `AV2-9-ADDITIONAL-TABLES`).
//!
//! These assert a handful of values from the `cargo xtask gen-tables` output
//! against the *other* rendering of the same spec — the committed mirror's § 9
//! Markdown text — so the cross-check is independent of the generator's own
//! parsing of `all_tables.h`. Every assertion cites the mirror file and line
//! range it was read from.

use splot_core::tables;

#[test]
fn conversion_mi_width_log2_matches_mirror() {
    let t = &tables::conversion::MI_WIDTH_LOG2;
    assert_eq!(t.len(), 29);
    assert_eq!(&t[..11], &[0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3]);
    assert_eq!(&t[25..], &[0, 3, 1, 4]);
}

#[test]
fn conversion_partition_size_tables_match_mirror() {
    let subsize = &tables::conversion::PARTITION_SUBSIZE;
    assert_eq!(subsize.len(), 10);
    assert_eq!(subsize[0].len(), 29);
    assert_eq!(
        subsize[0],
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28,
        ]
    );
    assert_eq!(subsize[1][0], 29); // BLOCK_4X4 -> BLOCK_INVALID.
    assert_eq!(subsize[1][1], 0); // BLOCK_4X8 -> BLOCK_4X4.
    assert_eq!(subsize[1][5], 20); // BLOCK_16X8 -> BLOCK_16X4.
    assert_eq!(subsize[2][2], 0); // BLOCK_8X4 -> BLOCK_4X4.
    assert_eq!(subsize[2][13], 29); // BLOCK_64X128 -> BLOCK_INVALID.
    assert_eq!(subsize[9][15], 12); // BLOCK_128X128 -> BLOCK_64X64.
    assert_eq!(subsize[9][18], 15); // BLOCK_256X256 -> BLOCK_128X128.

    let midsize = &tables::conversion::H_PARTITION_MIDSIZE;
    assert_eq!(midsize.len(), 29);
    assert_eq!(midsize[0], 29); // BLOCK_4X4 -> BLOCK_INVALID.
    assert_eq!(midsize[4], 1); // BLOCK_8X16 -> BLOCK_4X8.
    assert_eq!(midsize[12], 9); // BLOCK_64X64 -> BLOCK_32X32.
    assert_eq!(midsize[23], 21); // BLOCK_16X64 -> BLOCK_8X32.
    assert_eq!(midsize[28], 29); // BLOCK_64X8 -> BLOCK_INVALID.
}

#[test]
fn conversion_tx_size_symbolic_tables_match_mirror() {
    assert_eq!(tables::conversion::TX_SIZE_SQR.len(), 25);
    assert_eq!(tables::conversion::TX_SIZE_SQR_UP.len(), 25);
    assert_eq!(tables::conversion::ADJUSTED_TX_SIZE.len(), 25);

    assert_eq!(&tables::conversion::TX_SIZE_SQR[..5], &[0, 1, 2, 3, 4]);
    assert_eq!(tables::conversion::TX_SIZE_SQR[5], 0); // TX_4X8 -> TX_4X4.
    assert_eq!(tables::conversion::TX_SIZE_SQR[12], 3); // TX_64X32 -> TX_32X32.
    assert_eq!(tables::conversion::TX_SIZE_SQR[24], 0); // TX_64X4 -> TX_4X4.

    assert_eq!(&tables::conversion::TX_SIZE_SQR_UP[..5], &[0, 1, 2, 3, 4]);
    assert_eq!(tables::conversion::TX_SIZE_SQR_UP[5], 1); // TX_4X8 -> TX_8X8.
    assert_eq!(tables::conversion::TX_SIZE_SQR_UP[12], 4); // TX_64X32 -> TX_64X64.
    assert_eq!(tables::conversion::TX_SIZE_SQR_UP[24], 4); // TX_64X4 -> TX_64X64.

    assert_eq!(&tables::conversion::ADJUSTED_TX_SIZE[..5], &[0, 1, 2, 3, 3]);
    assert_eq!(tables::conversion::ADJUSTED_TX_SIZE[11], 3); // TX_32X64 -> TX_32X32.
    assert_eq!(tables::conversion::ADJUSTED_TX_SIZE[17], 9); // TX_16X64 -> TX_16X32.
    assert_eq!(tables::conversion::ADJUSTED_TX_SIZE[24], 20); // TX_64X4 -> TX_32X4.
}

#[test]
fn conversion_mode_to_txfm_matches_mirror() {
    let t = &tables::conversion::MODE_TO_TXFM;
    assert_eq!(t.len(), 14);
    assert_eq!(
        *t,
        [
            0, // DCT_DCT, DC_PRED.
            1, // ADST_DCT, V_PRED.
            2, // DCT_ADST, H_PRED.
            0, // DCT_DCT, D45_PRED.
            3, // ADST_ADST, D135_PRED.
            1, // ADST_DCT, D113_PRED.
            2, // DCT_ADST, D157_PRED.
            2, // DCT_ADST, D203_PRED.
            1, // ADST_DCT, D67_PRED.
            3, // ADST_ADST, SMOOTH_PRED.
            1, // ADST_DCT, SMOOTH_V_PRED.
            2, // DCT_ADST, SMOOTH_H_PRED.
            3, // ADST_ADST, PAETH_PRED.
            0, // DCT_DCT, UV_CFL_PRED.
        ]
    );
}

#[test]
fn conversion_palette_color_hash_multipliers_matches_mirror() {
    assert_eq!(
        tables::conversion::PALETTE_COLOR_HASH_MULTIPLIERS,
        [1, 2, 2]
    );
}

#[test]
fn conversion_para_adjustment_list_matches_mirror() {
    let t = &tables::conversion::PARA_ADJUSTMENT_LIST;
    assert_eq!(t.len(), 125);
    assert_eq!(t[0], [0, 0, 0]); // line 910
    assert_eq!(t[6], [0, -1, -1]); // line 911
    assert_eq!(t[50], [-2, 0, 0]); // line 930
    assert_eq!(t[124], [1, 1, 1]); // line 946
}

#[test]
fn conversion_prob_inc_matches_mirror() {
    let t = &tables::conversion::PROB_INC;
    assert_eq!(t.len(), 7);
    assert_eq!(t[0], [8, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(t[2], [12, 8, 4, 0, 0, 0, 0, 0]);
    assert_eq!(t[6], [14, 12, 10, 8, 6, 4, 2, 0]);
}

#[test]
fn cdf_default_skip_cdf_matches_mirror() {
    let t = &tables::cdf::DEFAULT_SKIP_CDF;
    assert_eq!(t[0], [25865, 25, 0]);
    assert_eq!(t[1], [14316, 0, 0]);
    assert_eq!(t[2], [4598, 0, 0]);
}
