// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use splot_core::span::ByteOffset;
use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};
use splot_core::tables::cdf::{
    DEFAULT_COEFF_BASE_CDF, DEFAULT_COEFF_BASE_EOB_CDF, DEFAULT_COEFF_BASE_EOB_UV_CDF,
    DEFAULT_COEFF_BASE_LF_CDF, DEFAULT_COEFF_BASE_LF_EOB_CDF, DEFAULT_COEFF_BASE_LF_EOB_UV_CDF,
    DEFAULT_COEFF_BASE_LF_UV_CDF, DEFAULT_COEFF_BASE_PH_CDF, DEFAULT_COEFF_BASE_UV_CDF,
    DEFAULT_COEFF_BR_CDF, DEFAULT_COEFF_BR_LF_CDF, DEFAULT_COEFF_BR_UV_CDF, DEFAULT_DC_SIGN_CDF,
    DEFAULT_EOB_EXTRA_CDF, DEFAULT_EOB_PT_16_CDF, DEFAULT_EOB_PT_32_CDF, DEFAULT_EOB_PT_64_CDF,
    DEFAULT_EOB_PT_128_CDF, DEFAULT_EOB_PT_256_CDF, DEFAULT_EOB_PT_512_CDF,
    DEFAULT_EOB_PT_1024_CDF, DEFAULT_TXB_SKIP_CDF, DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF,
    DEFAULT_V_TXB_SKIP_CDF, DEFAULT_Y_MODE_INDEX_CDF, DEFAULT_Y_MODE_SET_CDF,
};

fn coeff(selector: CoeffCdfSelector) -> TileCdfSelector {
    TileCdfSelector::Coeff(selector)
}

#[test]
fn frame_cdf_subset_copies_generated_defaults_without_aliasing() {
    let frame = FrameCdfSubset::from_defaults();
    assert_eq!(frame.rows().do_split(), &DEFAULT_DO_SPLIT_CDF);
    assert_eq!(
        frame.rows().do_ext_partition(),
        &DEFAULT_DO_EXT_PARTITION_CDF
    );
    assert_eq!(frame.rows().do_square_split(), &DEFAULT_DO_SQUARE_SPLIT_CDF);
    assert_eq!(frame.rows().rect_type(), &DEFAULT_RECT_TYPE_CDF);
    assert_eq!(
        frame.rows().do_uneven_4way_partition(),
        &DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF
    );
    assert_eq!(frame.rows().y_mode_set(), &DEFAULT_Y_MODE_SET_CDF);
    assert_eq!(frame.rows().y_mode_index(), &DEFAULT_Y_MODE_INDEX_CDF);
    assert_eq!(frame.rows().txb_skip(), &DEFAULT_TXB_SKIP_CDF);
    assert_eq!(
        frame.rows().uv_mode_cfl_not_allowed(),
        &DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF
    );
    assert_eq!(frame.rows().v_txb_skip(), &DEFAULT_V_TXB_SKIP_CDF);
    assert_eq!(frame.rows().eob_extra(), &DEFAULT_EOB_EXTRA_CDF);

    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0][0] = 1234;
    tile.rows_mut().do_ext_partition[0][4][0] = 5678;
    tile.rows_mut().rect_type[1][63][0] = 3456;
    tile.rows_mut().do_uneven_4way_partition[0][8][0] = 9012;

    assert_eq!(frame.rows().do_split()[0][0], DEFAULT_DO_SPLIT_CDF[0][0]);
    assert_eq!(
        frame.rows().do_ext_partition()[0][4],
        DEFAULT_DO_EXT_PARTITION_CDF[0][4]
    );
    assert_eq!(
        frame.rows().do_uneven_4way_partition()[0][8],
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][8]
    );
    assert_eq!(
        frame.rows().rect_type()[1][63],
        DEFAULT_RECT_TYPE_CDF[1][63]
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0
        })
        .unwrap(),
        DEFAULT_DO_SPLIT_CDF[0][0].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoExtPartition {
            plane_start: 0,
            ctx: 4
        })
        .unwrap(),
        DEFAULT_DO_EXT_PARTITION_CDF[0][4].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::RectType {
            plane_start: 1,
            ctx: 63
        })
        .unwrap(),
        DEFAULT_RECT_TYPE_CDF[1][63].as_slice()
    );
    assert_ne!(
        tile.row(TileCdfSelector::DoUneven4WayPartition {
            plane_start: 0,
            ctx: 8
        })
        .unwrap(),
        DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[0][8].as_slice()
    );
}

#[test]
fn selector_returns_rows_and_bounds_errors() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let row = tile
        .row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_SPLIT_CDF[0][0].as_slice());
    assert_eq!(row.len(), CDF_ROW_LEN);

    let row = tile
        .row(TileCdfSelector::DoExtPartition {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_EXT_PARTITION_CDF[1][63].as_slice());

    let row = tile
        .row(TileCdfSelector::DoUneven4WayPartition {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_DO_UNEVEN_4WAY_PARTITION_CDF[1][63].as_slice());

    let row = tile
        .row(TileCdfSelector::RectType {
            plane_start: 1,
            ctx: 63,
        })
        .unwrap();
    assert_eq!(row, DEFAULT_RECT_TYPE_CDF[1][63].as_slice());

    let err = tile
        .with_row_mut(
            TileCdfSelector::DoSquareSplit {
                plane_start: 1,
                ctx: 0,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSquareSplit,
            index_name: "plane_start",
            actual: 1,
            max_exclusive: 1,
        }
    );

    let err = tile
        .row(TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 64,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoSplit,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );

    let err = tile
        .row(TileCdfSelector::DoExtPartition {
            plane_start: 2,
            ctx: 0,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoExtPartition,
            index_name: "plane_start",
            actual: 2,
            max_exclusive: 2,
        }
    );

    let err = tile
        .with_row_mut(
            TileCdfSelector::RectType {
                plane_start: 2,
                ctx: 0,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::RectType,
            index_name: "plane_start",
            actual: 2,
            max_exclusive: 2,
        }
    );

    let err = tile
        .row(TileCdfSelector::RectType {
            plane_start: 0,
            ctx: 64,
        })
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::RectType,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );

    let err = tile
        .with_row_mut(
            TileCdfSelector::DoUneven4WayPartition {
                plane_start: 0,
                ctx: 64,
            },
            |_| (),
        )
        .unwrap_err();
    assert_eq!(
        err,
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DoUneven4WayPartition,
            index_name: "ctx",
            actual: 64,
            max_exclusive: 64,
        }
    );
}

#[test]
fn selected_row_hands_off_to_symbol_decoder_update_modes() {
    let frame = FrameCdfSubset::from_defaults();
    let selectors = [
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::DoExtPartition {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoSquareSplit {
            plane_start: 0,
            ctx: 0,
        },
        TileCdfSelector::RectType {
            plane_start: 0,
            ctx: 4,
        },
        TileCdfSelector::DoUneven4WayPartition {
            plane_start: 0,
            ctx: 8,
        },
    ];
    let payload = [0x80, 0x00];

    for selector in selectors {
        let mut enabled = frame.tile_copy();
        let before = enabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap();
        enabled
            .read_partition_entry_symbol(selector, &mut symbol)
            .unwrap();
        assert_ne!(enabled.row(selector).unwrap(), before.as_slice());

        let mut disabled = frame.tile_copy();
        let before = disabled.row(selector).unwrap().to_vec();
        let mut symbol = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
        )
        .unwrap();
        disabled
            .read_partition_entry_symbol(selector, &mut symbol)
            .unwrap();
        assert_eq!(disabled.row(selector).unwrap(), before.as_slice());
    }
}

#[test]
fn cdf_save_policy_matches_spec() {
    let single = tile_cdf_save_policy(TileCdfPolicyInput::new(1, 1, false, false, 0), 0).unwrap();
    assert_eq!(single.num_log2(), 0);
    assert!(single.copy_cdf());
    assert!(!single.avg_cdf());

    let avg = tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, true, true, 0), 2).unwrap();
    assert_eq!(avg.num_log2(), 2);
    assert!(avg.avg_cdf());
    assert!(!avg.copy_cdf());

    let not_averaged =
        tile_cdf_save_policy(TileCdfPolicyInput::new(16, 1, true, true, 0), 8).unwrap();
    assert_eq!(not_averaged.num_log2(), 3);
    assert!(!not_averaged.avg_cdf());

    let context = tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 3), 3).unwrap();
    assert!(context.copy_cdf());

    assert!(matches!(
        tile_cdf_save_policy(TileCdfPolicyInput::new(u32::MAX, 2, false, false, 0), 0),
        Err(TileCdfError::TileCountOverflow { .. })
    ));
    assert!(matches!(
        tile_cdf_save_policy(TileCdfPolicyInput::new(2, 2, false, false, 4), 0),
        Err(TileCdfError::ContextUpdateTileOutOfRange { .. })
    ));
}

#[test]
fn saved_copy_and_average_are_exact_for_supported_subset() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0] = [20_000, 7, 4];
    tile.rows_mut().do_ext_partition[0][4] = [22_000, 5, 8];
    tile.rows_mut().do_square_split[0][0] = [21_000, 6, 2];
    tile.rows_mut().rect_type[1][63] = [24_000, 3, 16];
    tile.rows_mut().do_uneven_4way_partition[0][8] = [23_000, 4, 12];
    tile.rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 8];
    tile.rows_mut().block.y_mode_index[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.txb_skip[2][0][0][0] = [25_000, 13, 20];
    tile.rows_mut().block.uv_mode_cfl_not_allowed[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.v_txb_skip[1][3] = [26_000, 14, 24];
    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [20_000, 21_000, 22_000, 9, 8];

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );
    assert_eq!(saved.rows(), tile.rows());

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 2,
            copy_cdf: false,
            avg_cdf: true,
        },
    );
    assert_eq!(saved.rows().do_split()[0][0], [29_576, 7, 1]);
    assert_eq!(saved.rows().do_ext_partition()[0][4], [30_076, 5, 2]);
    assert_eq!(saved.rows().do_square_split()[0][0], [29_826, 6, 0]);
    assert_eq!(saved.rows().rect_type()[1][63], [30_576, 3, 4]);
    assert_eq!(
        saved.rows().do_uneven_4way_partition()[0][8],
        [30_326, 4, 3]
    );
    assert_eq!(saved.rows().y_mode_set(), &[29_576, 29_826, 30_076, 9, 2]);
    assert_eq!(
        saved.rows().y_mode_index()[0],
        [
            29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 31_076, 11, 3
        ]
    );
    assert_eq!(saved.rows().txb_skip()[2][0][0][0], [30_826, 13, 5]);
    assert_eq!(
        saved.rows().uv_mode_cfl_not_allowed()[0],
        [
            29_576, 29_826, 30_076, 30_326, 30_576, 30_826, 31_076, 11, 3
        ]
    );
    assert_eq!(saved.rows().v_txb_skip()[1][3], [31_076, 14, 6]);
    assert_eq!(
        saved.rows().block.coeff.coeff_base[1][2][3][1],
        [29_576, 29_826, 30_076, 9, 2]
    );
}

#[test]
fn disabled_cdf_update_keeps_saved_subset_at_initial_rows() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbol = SymbolDecoder::with_base_and_config(
        &[0x80, 0x00],
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    tile.read_partition_entry_symbol(
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 0,
        },
        &mut symbol,
    )
    .unwrap();

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );

    assert_eq!(tile.rows(), frame.rows());
    assert_eq!(saved.rows(), frame.rows());
}

#[test]
fn frame_end_update_copies_saved_rows_and_scales_counts() {
    let mut frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().do_split[0][0] = [20_000, 7, 20];
    tile.rows_mut().do_ext_partition[0][4] = [22_000, 5, 8];
    tile.rows_mut().do_square_split[0][0] = [21_000, 6, 2];
    tile.rows_mut().rect_type[1][63] = [24_000, 3, 16];
    tile.rows_mut().do_uneven_4way_partition[0][8] = [23_000, 4, 12];
    tile.rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 20];
    tile.rows_mut().block.y_mode_index[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12,
    ];
    tile.rows_mut().block.txb_skip[2][0][0][0] = [25_000, 13, 20];
    tile.rows_mut().block.uv_mode_cfl_not_allowed[0] = [
        20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 16,
    ];
    tile.rows_mut().block.v_txb_skip[1][3] = [26_000, 14, 24];
    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [20_000, 21_000, 22_000, 9, 20];

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_completed_tile(
        0,
        &tile,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
    );
    frame.frame_end_update_from_saved(&saved);

    assert_eq!(frame.rows().do_split()[0][0], [20_000, 7, 15]);
    assert_eq!(frame.rows().do_ext_partition()[0][4], [22_000, 5, 6]);
    assert_eq!(frame.rows().do_square_split()[0][0], [21_000, 6, 1]);
    assert_eq!(frame.rows().rect_type()[1][63], [24_000, 3, 12]);
    assert_eq!(
        frame.rows().do_uneven_4way_partition()[0][8],
        [23_000, 4, 9]
    );
    assert_eq!(frame.rows().y_mode_set(), &[20_000, 21_000, 22_000, 9, 15]);
    assert_eq!(
        frame.rows().y_mode_index()[0],
        [
            20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 9
        ]
    );
    assert_eq!(frame.rows().txb_skip()[2][0][0][0], [25_000, 13, 15]);
    assert_eq!(
        frame.rows().uv_mode_cfl_not_allowed()[0],
        [
            20_000, 21_000, 22_000, 23_000, 24_000, 25_000, 26_000, 11, 12
        ]
    );
    assert_eq!(frame.rows().v_txb_skip()[1][3], [26_000, 14, 18]);
    assert_eq!(
        frame.rows().block.coeff.coeff_base[1][2][3][1],
        [20_000, 21_000, 22_000, 9, 15]
    );
}

#[test]
fn work_unit_boundary_applies_saved_and_frame_updates_transactionally() {
    let expected_frame = FrameCdfSubset::from_defaults();
    let mut boundary = TileCdfWorkUnitBoundary::new(
        CdfUpdateMode::Enabled,
        TileCdfSavePolicy {
            num_log2: 0,
            copy_cdf: true,
            avg_cdf: false,
        },
        FrameCdfSubset::from_defaults(),
    );
    boundary.tile_cdfs_mut().rows_mut().do_split[0][0] = [20_000, 7, 20];
    boundary.tile_cdfs_mut().rows_mut().block.y_mode_set = [20_000, 21_000, 22_000, 9, 20];

    assert_eq!(boundary.saved_cdfs().rows(), expected_frame.rows());
    assert_eq!(boundary.frame_cdfs().rows(), expected_frame.rows());

    boundary.apply_completed_tile_to_saved(0);
    assert_eq!(
        boundary.saved_cdfs().rows().do_split()[0][0],
        [20_000, 7, 20]
    );
    assert_eq!(
        boundary.saved_cdfs().rows().y_mode_set(),
        &[20_000, 21_000, 22_000, 9, 20]
    );
    assert_eq!(boundary.frame_cdfs().rows(), expected_frame.rows());

    boundary.frame_end_update_cdf_subset();
    assert_eq!(
        boundary.frame_cdfs().rows().do_split()[0][0],
        [20_000, 7, 15]
    );
    assert_eq!(
        boundary.frame_cdfs().rows().y_mode_set(),
        &[20_000, 21_000, 22_000, 9, 15]
    );
}

#[test]
fn eob_extra_selector_returns_rows_and_bounds_error() {
    // AV2 § 8.3.2: TileEobExtraCdf is selected directly by coeff_cdf_q_ctx with
    // no per-symbol context. Each q-context returns its Default_Eob_Extra_Cdf row.
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    for (q, expected) in DEFAULT_EOB_EXTRA_CDF.iter().enumerate() {
        let row = tile
            .row(TileCdfSelector::EobExtra { coeff_cdf_q_ctx: q })
            .unwrap();
        assert_eq!(row, expected.as_slice(), "eob_extra q-ctx {q}");
    }
    // A coeff_cdf_q_ctx at the bound is a typed SelectorOutOfRange naming the array.
    assert!(matches!(
        tile.row(TileCdfSelector::EobExtra { coeff_cdf_q_ctx: 4 }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::EobExtra,
            index_name: "coeff_cdf_q_ctx",
            actual: 4,
            max_exclusive: 4,
        })
    ));
}

#[test]
fn eob_extra_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.eob_extra[2] = [12_345, 0, 7];
    // The tile copy is mutated; the frame's default row is untouched.
    assert_eq!(tile.rows().eob_extra()[2], [12_345, 0, 7]);
    assert_eq!(frame.rows().eob_extra()[2], DEFAULT_EOB_EXTRA_CDF[2]);
}

fn assert_eob_pt_bank<const N: usize>(
    tile: &TileCdfSubset,
    size: EobPtSize,
    expected: &[[[i32; N]; 3]; 4],
) {
    // AV2 §8.3.2 selects TileEobPt<size>Cdf[eobCtx] for the active q-context.
    for (q, expected_q) in expected.iter().enumerate() {
        for (c, expected_qc) in expected_q.iter().enumerate() {
            let row = tile
                .row(TileCdfSelector::EobPt {
                    size,
                    coeff_cdf_q_ctx: q,
                    eob_ctx: c,
                })
                .unwrap();
            assert_eq!(row, expected_qc.as_slice(), "eob_pt {size:?} q {q} ctx {c}");
        }
    }
}

#[test]
fn eob_pt_family_loads_defaults_and_selects_by_size_and_context() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert_eob_pt_bank(&tile, EobPtSize::Pt16, &DEFAULT_EOB_PT_16_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt32, &DEFAULT_EOB_PT_32_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt64, &DEFAULT_EOB_PT_64_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt128, &DEFAULT_EOB_PT_128_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt256, &DEFAULT_EOB_PT_256_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt512, &DEFAULT_EOB_PT_512_CDF);
    assert_eob_pt_bank(&tile, EobPtSize::Pt1024, &DEFAULT_EOB_PT_1024_CDF);
}

#[test]
fn eob_pt_selector_rejects_out_of_range_contexts() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert!(matches!(
        tile.row(TileCdfSelector::EobPt {
            size: EobPtSize::Pt16,
            coeff_cdf_q_ctx: 4,
            eob_ctx: 0
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::EobPt,
            index_name: "coeff_cdf_q_ctx",
            actual: 4,
            max_exclusive: 4,
        })
    ));
    assert!(matches!(
        tile.row(TileCdfSelector::EobPt {
            size: EobPtSize::Pt1024,
            coeff_cdf_q_ctx: 0,
            eob_ctx: 3
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::EobPt,
            index_name: "eob_ctx",
            actual: 3,
            max_exclusive: 3,
        })
    ));
}

#[test]
fn eob_pt_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.eob_pt_16[1][2] = [10, 20, 30, 40, 50, 7];
    assert_eq!(tile.rows().block.eob_pt_16[1][2], [10, 20, 30, 40, 50, 7]);
    assert_eq!(
        frame.rows().block.eob_pt_16[1][2],
        DEFAULT_EOB_PT_16_CDF[1][2]
    );
}

#[test]
fn dc_sign_loads_defaults_and_selects_by_all_indices() {
    // AV2 §8.3.2: dc_sign reads TileDcSignCdf[ptype][isHidden][ctx] for the
    // active q-context. Verify every [q][plane][group][ctx] cell round-trips.
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    for (q, q_rows) in DEFAULT_DC_SIGN_CDF.iter().enumerate() {
        for (p, p_rows) in q_rows.iter().enumerate() {
            for (g, g_rows) in p_rows.iter().enumerate() {
                for (c, expected) in g_rows.iter().enumerate() {
                    let row = tile
                        .row(TileCdfSelector::DcSign {
                            coeff_cdf_q_ctx: q,
                            plane_type: p,
                            group: g,
                            ctx: c,
                        })
                        .unwrap();
                    assert_eq!(row, expected.as_slice(), "dc_sign q{q} p{p} g{g} c{c}");
                }
            }
        }
    }
}

#[test]
fn dc_sign_selector_rejects_out_of_range_indices() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    // Each of the four index axes is bounds-checked and names the DcSign array.
    assert!(matches!(
        tile.row(TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 4,
            plane_type: 0,
            group: 0,
            ctx: 0
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "coeff_cdf_q_ctx",
            actual: 4,
            max_exclusive: 4,
        })
    ));
    assert!(matches!(
        tile.row(TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 2,
            group: 0,
            ctx: 0
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "plane_type",
            actual: 2,
            max_exclusive: 2,
        })
    ));
    assert!(matches!(
        tile.row(TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 2,
            ctx: 0
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "group",
            actual: 2,
            max_exclusive: 2,
        })
    ));
    assert!(matches!(
        tile.row(TileCdfSelector::DcSign {
            coeff_cdf_q_ctx: 0,
            plane_type: 0,
            group: 0,
            ctx: 3
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::DcSign,
            index_name: "ctx",
            actual: 3,
            max_exclusive: 3,
        })
    ));
}

#[test]
fn dc_sign_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    tile.rows_mut().block.dc_sign[2][1][0][1] = [111, 5, 9];
    assert_eq!(tile.rows().block.dc_sign[2][1][0][1], [111, 5, 9]);
    assert_eq!(
        frame.rows().block.dc_sign[2][1][0][1],
        DEFAULT_DC_SIGN_CDF[2][1][0][1]
    );
}

#[test]
fn txb_skip_plane_type_error_still_names_txb_skip() {
    // `checked_plane_type` is now parameterized with the owning array (shared by
    // txb_skip and dc_sign); an out-of-range txb_skip plane_type must still name
    // TxbSkip, not DcSign.
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    assert!(matches!(
        tile.row(TileCdfSelector::TxbSkip {
            coeff_cdf_q_ctx: 0,
            plane_type: 2,
            tx_size: 0,
            ctx: 0
        }),
        Err(TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::TxbSkip,
            index_name: "plane_type",
            actual: 2,
            max_exclusive: 2,
        })
    ));
}

#[test]
fn coeff_base_rows_load_defaults_and_select_by_family() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();
    let cases: &[(TileCdfSelector, &[i32])] = &[
        (
            coeff(CoeffCdfSelector::Base {
                coeff_cdf_q_ctx: 1,
                tx_size: 2,
                ctx: 3,
                tcq_ctx: 1,
            }),
            DEFAULT_COEFF_BASE_CDF[1][2][3][1].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BasePh {
                coeff_cdf_q_ctx: 2,
                ctx: 4,
            }),
            DEFAULT_COEFF_BASE_PH_CDF[2][4].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseUv {
                coeff_cdf_q_ctx: 2,
                ctx: 11,
            }),
            DEFAULT_COEFF_BASE_UV_CDF[2][11].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLf {
                coeff_cdf_q_ctx: 3,
                tx_size: 4,
                ctx: 32,
                tcq_ctx: 0,
            }),
            DEFAULT_COEFF_BASE_LF_CDF[3][4][32][0].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfUv {
                coeff_cdf_q_ctx: 0,
                ctx: 11,
            }),
            DEFAULT_COEFF_BASE_LF_UV_CDF[0][11].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseEob {
                coeff_cdf_q_ctx: 1,
                tx_size: 2,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_EOB_CDF[1][2][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseEobUv {
                coeff_cdf_q_ctx: 2,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_EOB_UV_CDF[2][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfEob {
                coeff_cdf_q_ctx: 3,
                tx_size: 4,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_LF_EOB_CDF[3][4][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BaseLfEobUv {
                coeff_cdf_q_ctx: 1,
                ctx: 3,
            }),
            DEFAULT_COEFF_BASE_LF_EOB_UV_CDF[1][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::Br {
                coeff_cdf_q_ctx: 2,
                ctx: 6,
            }),
            DEFAULT_COEFF_BR_CDF[2][6].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BrUv {
                coeff_cdf_q_ctx: 3,
                ctx: 3,
            }),
            DEFAULT_COEFF_BR_UV_CDF[3][3].as_slice(),
        ),
        (
            coeff(CoeffCdfSelector::BrLf {
                coeff_cdf_q_ctx: 1,
                ctx: 13,
            }),
            DEFAULT_COEFF_BR_LF_CDF[1][13].as_slice(),
        ),
    ];

    for (selector, expected) in cases {
        assert_eq!(tile.row(*selector).unwrap(), *expected, "{selector:?}");
    }
}

#[test]
fn coeff_base_selectors_reject_out_of_range_axes() {
    let frame = FrameCdfSubset::from_defaults();
    let tile = frame.tile_copy();

    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::Base {
            coeff_cdf_q_ctx: 4,
            tx_size: 0,
            ctx: 0,
            tcq_ctx: 0,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBase,
            index_name: "coeff_cdf_q_ctx",
            actual: 4,
            max_exclusive: 4,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::BaseLfEob {
            coeff_cdf_q_ctx: 0,
            tx_size: 5,
            ctx: 0,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBaseLfEob,
            index_name: "tx_size",
            actual: 5,
            max_exclusive: 5,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::BasePh {
            coeff_cdf_q_ctx: 4,
            ctx: 0,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBasePh,
            index_name: "coeff_cdf_q_ctx",
            actual: 4,
            max_exclusive: 4,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::BasePh {
            coeff_cdf_q_ctx: 0,
            ctx: 5,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBasePh,
            index_name: "ctx",
            actual: 5,
            max_exclusive: 5,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::BaseUv {
            coeff_cdf_q_ctx: 0,
            ctx: 12,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBaseUv,
            index_name: "ctx",
            actual: 12,
            max_exclusive: 12,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::Base {
            coeff_cdf_q_ctx: 0,
            tx_size: 0,
            ctx: 0,
            tcq_ctx: 2,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBase,
            index_name: "tcq_ctx",
            actual: 2,
            max_exclusive: 2,
        }
    );
    assert_eq!(
        tile.row(coeff(CoeffCdfSelector::BrLf {
            coeff_cdf_q_ctx: 0,
            ctx: 14,
        }))
        .unwrap_err(),
        TileCdfError::SelectorOutOfRange {
            array: TileCdfArray::CoeffBrLf,
            index_name: "ctx",
            actual: 14,
            max_exclusive: 14,
        }
    );
}

#[test]
fn coeff_base_tile_copy_does_not_alias_the_frame() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();

    tile.rows_mut().block.coeff.coeff_base[1][2][3][1] = [12_000, 13_000, 14_000, 7, 9];
    tile.rows_mut().block.coeff.coeff_base_ph[2][4] = [12_000, 13_000, 14_000, 8, 10];
    tile.rows_mut().block.coeff.coeff_base_lf_uv[0][11] =
        [11_000, 12_000, 13_000, 14_000, 15_000, 5, 8];
    tile.rows_mut().block.coeff.coeff_br_lf[1][13] = [15_000, 16_000, 17_000, 6, 12];

    assert_eq!(
        frame.rows().block.coeff.coeff_base[1][2][3][1],
        DEFAULT_COEFF_BASE_CDF[1][2][3][1]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_ph[2][4],
        DEFAULT_COEFF_BASE_PH_CDF[2][4]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_base_lf_uv[0][11],
        DEFAULT_COEFF_BASE_LF_UV_CDF[0][11]
    );
    assert_eq!(
        frame.rows().block.coeff.coeff_br_lf[1][13],
        DEFAULT_COEFF_BR_LF_CDF[1][13]
    );
}

#[test]
fn coeff_base_row_hands_off_to_symbol_decoder_update_mode() {
    let frame = FrameCdfSubset::from_defaults();
    let selector = coeff(CoeffCdfSelector::BasePh {
        coeff_cdf_q_ctx: 1,
        ctx: 3,
    });
    let payload = [0x80, 0x00];
    let mut tile = frame.tile_copy();
    let before = tile.row(selector).unwrap().to_vec();
    let mut symbol = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap();
    let consumed_before = symbol.consumed_bits();

    tile.read_block_symbol_trace(selector, &mut symbol).unwrap();

    assert_ne!(tile.row(selector).unwrap(), before.as_slice());
    assert_ne!(symbol.consumed_bits(), consumed_before);
}
