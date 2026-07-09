// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

use crate::bitstream::tile_payload::{encode_symbol_sequence, make_test_work_unit};

use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};

const BLOCK_16X8: usize = 5;

fn tx_size_for(width: usize, height: usize) -> usize {
    TX_WIDTH
        .iter()
        .zip(TX_HEIGHT.iter())
        .position(|(&w, &h)| w == width as i32 && h == height as i32)
        .expect("tx size")
}

#[test]
fn luma_tx_type_map_scales_chroma_coordinates_with_mi_floor() {
    let offset = ByteOffset::new(0);
    let mut map = InterLumaTxTypeMap::new(9, 4, 8, 8, offset).unwrap();
    map.update(9, 4, tx_size_for(8, 4), V_DCT, offset).unwrap();

    assert_eq!(map.chroma_inter_tx_type(9, 4, 4, 2, true, true), V_DCT);
    assert_eq!(map.chroma_inter_tx_type(9, 4, 5, 3, true, true), DCT_DCT);
}

#[test]
fn luma_tx_type_map_updates_on_16x16_units() {
    let offset = ByteOffset::new(0);
    let mut map = InterLumaTxTypeMap::new(0, 0, 8, 8, offset).unwrap();
    map.update(0, 0, tx_size_for(32, 16), V_DCT, offset)
        .unwrap();

    assert_eq!(map.values[map.index(0, 0).unwrap()], V_DCT);
    assert_eq!(map.values[map.index(0, 4).unwrap()], V_DCT);
    assert_eq!(map.values[map.index(0, 7).unwrap()], DCT_DCT);
}

#[test]
fn chroma_group_start_waits_for_subsampled_luma_chunks() {
    assert_eq!(
        completed_chroma_group_start(0, 0, 2, 2, false, false),
        Some((0, 0))
    );
    assert_eq!(completed_chroma_group_start(0, 0, 2, 2, true, false), None);
    assert_eq!(
        completed_chroma_group_start(1, 0, 2, 2, true, false),
        Some((0, 0))
    );
    assert_eq!(
        completed_chroma_group_start(1, 1, 2, 2, true, true),
        Some((0, 0))
    );
    assert_eq!(
        completed_chroma_group_start(0, 1, 1, 2, true, true),
        Some((0, 0))
    );
}

#[test]
fn lossless_inter_residual_tx_size_reads_selector() {
    let offset = ByteOffset::new(0);
    let size_group =
        usize::try_from(splot_core::tables::conversion::SIZE_GROUP[BLOCK_16X8]).unwrap();
    let payload = encode_symbol_sequence(&[(
        TileCdfSelector::LosslessTxSize {
            size_group,
            is_inter: 1,
        },
        1,
    )]);
    let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Disabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        offset,
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap();

    assert_eq!(
        inter_residual_tx_size(
            &mut work_unit,
            &mut symbols,
            BLOCK_16X8,
            true,
            InterResidualLumaTxSizeMode::Inter,
            offset,
        )
        .unwrap(),
        tx_size_for(16, 8)
    );
    assert_eq!(symbols.symbol_count(), 1);
}
