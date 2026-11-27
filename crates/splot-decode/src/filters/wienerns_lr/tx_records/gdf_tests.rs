// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

use super::*;
use crate::bitstream::tile_payload::{FrameCdfSubset, encode_symbol_sequence};

#[test]
fn inactive_state_has_no_grid() {
    let state = GdfState::inactive();

    assert!(!state.active);
    assert!(state.into_grid().unwrap().is_none());
}

#[test]
fn use_gdf_symbols_round_trip() {
    let payload =
        encode_symbol_sequence(&[(TileCdfSelector::UseGdf, 1), (TileCdfSelector::UseGdf, 0)]);
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    assert_eq!(
        read_use_gdf(&mut cdfs, &mut symbols, ByteOffset::new(0)).unwrap(),
        1
    );
    assert_eq!(
        read_use_gdf(&mut cdfs, &mut symbols, ByteOffset::new(0)).unwrap(),
        0
    );
    assert_eq!(symbols.symbol_count(), 2);
}

#[test]
fn unvisited_gdf_unit_fails_closed() {
    let state = GdfState {
        active: true,
        block_size: 64,
        sb_size4: 16,
        sb_per_gdf: 1,
        window: UnitWindow::frame(1, 1),
        values: vec![2],
    };

    assert!(state.into_grid().is_err());
}

#[test]
fn gdf_tile_merge_copies_only_the_owned_region() {
    let mut frame = GdfState {
        active: true,
        block_size: 64,
        sb_size4: 16,
        sb_per_gdf: 1,
        window: UnitWindow::frame(1, 2),
        values: vec![2; 2],
    };
    let mut left = frame.for_tile(0..16, 0..16, None).unwrap();
    let mut right = frame.for_tile(0..16, 16..32, None).unwrap();
    left.values = vec![0];
    right.values = vec![0];

    frame.merge_tile(&left, 0..16, 0..16).unwrap();
    frame.merge_tile(&right, 0..16, 16..32).unwrap();

    assert_eq!(frame.values, [0, 0]);
}

#[test]
fn multi_block_bru_leaf_gdf_and_ccso_reads_stay_entropy_synchronized() {
    use crate::bitstream::tile_payload::{
        DecodeBlockFrontier, DecodeTileWorkUnit, GeneralIntraLeafMode,
        GeneralIntraMultiblockCursor, LeafDecoder, LeafNeighbours, TileTraversalStorage,
        make_test_work_unit_at,
    };
    use crate::filters::wienerns_lr::tx_records::ccso::CcsoState;
    use crate::prediction::inter::test_support::fixture_sequence_and_key_core;
    use splot_core::symbol::{CdfUpdateMode, Symbol};
    use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

    struct SyncDecoder {
        gdf_state: GdfState,
        ccso_state: CcsoState,
        visited: Vec<bool>,
    }

    impl<'payload> LeafDecoder<'payload> for SyncDecoder {
        type Command = ();
        type Error = crate::DecodeError;

        fn decode_leaf<const ACTIVE: bool>(
            &mut self,
            work_unit: &mut DecodeTileWorkUnit,
            symbols: &mut SymbolDecoder<'payload>,
            frontier: &DecodeBlockFrontier,
            _neighbours: LeafNeighbours<'_>,
        ) -> crate::Result<(GeneralIntraLeafMode, ())> {
            self.visited.push(ACTIVE);
            self.gdf_state
                .read_for_block(work_unit, symbols, frontier, ByteOffset::new(0))?;
            self.ccso_state
                .read_for_block(work_unit, symbols, frontier, ByteOffset::new(0))?;
            Ok((GeneralIntraLeafMode::chroma(0), ()))
        }
    }

    let fixture = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/\
         syn-gdf-per-block-intra-128x128-q120.ivf"
    );
    let (sequence, mut core) = fixture_sequence_and_key_core(fixture);
    core.frame_is_intra = Some(false);
    let mut inter = splot_core::headers::frame::InterControl::default();
    inter.use_bru = Some(true);
    inter.bru_inactive = Some(false);
    core.inter = Some(inter);
    let mut gdf = core.gdf_params.unwrap();
    gdf.gdf_frame_enable = true;
    gdf.gdf_per_block = Some(true);
    gdf.gdf_pic_qc_idx = Some(0);
    gdf.gdf_pic_scale_idx = Some(0);
    core.gdf_params = Some(gdf);
    if let Some(lr) = core.lr_params.as_mut() {
        lr.uses_lr = false;
    }
    if let Some(tile) = core.tile_info.as_mut() {
        tile.tile_cols = 1;
        tile.tile_rows = 1;
        tile.mi_col_starts = splot_core::tile::TileStarts::from_iter_checked([0, 32]).unwrap();
        tile.mi_row_starts = splot_core::tile::TileStarts::from_iter_checked([0, 32]).unwrap();
        tile.context_update_tile_id = 0;
    }

    let mut enc_cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    );
    let mut write_sym = |selector: TileCdfSelector, value: u8| {
        enc_cdfs
            .with_row_mut(selector, |row| {
                encoder.write_symbol_u16(row, Symbol::new(value))
            })
            .unwrap()
            .unwrap();
    };
    write_sym(TileCdfSelector::BruMode, 0);
    write_sym(TileCdfSelector::UseGdf, 1);
    write_sym(TileCdfSelector::CcsoBlk { plane: 0, ctx: 0 }, 1);
    write_sym(TileCdfSelector::BruMode, 2);
    write_sym(
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 12,
        },
        0,
    );
    write_sym(TileCdfSelector::UseGdf, 0);
    write_sym(TileCdfSelector::CcsoBlk { plane: 0, ctx: 2 }, 0);
    write_sym(TileCdfSelector::BruMode, 1);
    write_sym(TileCdfSelector::UseGdf, 1);
    write_sym(TileCdfSelector::CcsoBlk { plane: 0, ctx: 0 }, 1);
    write_sym(TileCdfSelector::BruMode, 2);
    write_sym(
        TileCdfSelector::DoSplit {
            plane_start: 0,
            ctx: 12,
        },
        0,
    );
    write_sym(TileCdfSelector::UseGdf, 1);
    write_sym(TileCdfSelector::CcsoBlk { plane: 0, ctx: 2 }, 1);
    let payload = encoder.finish().unwrap().into_bytes();

    let mut work_unit = make_test_work_unit_at(&payload, CdfUpdateMode::Enabled, 0..32, 0..32)
        .with_bru_tile_active(true);
    let initial_use_gdf_cdf = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .with_row_mut(TileCdfSelector::UseGdf, |row| [row[0], row[1]])
        .unwrap();

    let gdf_state = GdfState::new(32, 32, &sequence, &core)
        .unwrap()
        .for_tile(0..32, 0..32, None)
        .unwrap();
    let ccso_state = CcsoState::active(
        4,
        [true, false, false],
        [false; super::super::CCSO_PLANES],
        (2, 2, 4),
        std::array::from_fn(|_| Vec::new()),
    )
    .unwrap();

    let mut cursor = GeneralIntraMultiblockCursor::new(
        &work_unit,
        &payload,
        &sequence,
        &core,
        crate::DecodeLimits::DEFAULT,
        TileTraversalStorage::default(),
    )
    .unwrap();
    let mut decoder = SyncDecoder {
        gdf_state,
        ccso_state,
        visited: Vec::new(),
    };
    let mut on_pub = |_, ()| {};

    assert_eq!(
        cursor
            .decode_next_superblock(&mut work_unit, &mut decoder, &mut on_pub)
            .unwrap(),
        Some([0, 0])
    );
    let after_first_sb_cdf = work_unit
        .cdf_mut()
        .tile_cdfs_mut()
        .with_row_mut(TileCdfSelector::UseGdf, |row| [row[0], row[1]])
        .unwrap();
    assert_ne!(after_first_sb_cdf, initial_use_gdf_cdf);

    assert_eq!(
        cursor
            .decode_next_superblock(&mut work_unit, &mut decoder, &mut on_pub)
            .unwrap(),
        Some([0, 16])
    );
    assert_eq!(
        cursor
            .decode_next_superblock(&mut work_unit, &mut decoder, &mut on_pub)
            .unwrap(),
        Some([16, 0])
    );
    assert_eq!(
        cursor
            .decode_next_superblock(&mut work_unit, &mut decoder, &mut on_pub)
            .unwrap(),
        Some([16, 16])
    );
    assert_eq!(
        cursor
            .decode_next_superblock(&mut work_unit, &mut decoder, &mut on_pub)
            .unwrap(),
        None
    );

    assert_eq!(decoder.visited, [false, true, false, true]);
    assert_eq!(
        decoder.gdf_state.into_grid().unwrap().unwrap(),
        crate::filters::gdf::GdfBlockGrid::new(64, 2, 2, vec![1, 0, 1, 1]).unwrap()
    );
    assert_eq!(decoder.ccso_state.block_value(0, 0, 0), 1);
    assert_eq!(decoder.ccso_state.block_value(0, 0, 1), 0);
    assert_eq!(decoder.ccso_state.block_value(0, 1, 0), 1);
    assert_eq!(decoder.ccso_state.block_value(0, 1, 1), 1);
    let output = cursor.into_output();
    assert_eq!(output.symbols.symbol_count(), 14);
    output.symbols.exit_symbol().unwrap();
}
