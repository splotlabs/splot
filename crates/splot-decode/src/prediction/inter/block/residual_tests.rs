// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

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
