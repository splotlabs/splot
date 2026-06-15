// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use splot_core::span::ByteOffset;
use splot_core::symbol::{SymbolDecoder, SymbolDecoderConfig};

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
            .with_row_mut(selector, |row| symbol.read_symbol(row))
            .unwrap()
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
            .with_row_mut(selector, |row| symbol.read_symbol(row))
            .unwrap()
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

    let mut saved = SavedCdfSubset::from_frame(&frame);
    saved.apply_tile(
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
    saved.apply_tile(
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
}
