// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use splot_core::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::*;
use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::prediction::inter::read_mv::{
    MV_PRECISION_EIGHT_PEL, MV_PRECISION_FOUR_PEL, MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL,
    MV_PRECISION_QUARTER_PEL,
};

/// The largest difference component: `MV_UPP - 1` (17 signed bits).
const MAX_COMPONENT: u32 = (1 << 16) - 1;

/// A component of random bit length, so every shell class occurs, on the
/// lattice of `precision`.
fn component(state: &mut u64, precision: u8) -> i32 {
    let mut next = || {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    };
    let bits = (next() % 18) as u32;
    let step = 1u32 << (MV_PRECISION_EIGHTH_PEL - precision);
    let magnitude =
        ((next() as u32 & ((1 << bits) - 1)) & !(step - 1)).min(MAX_COMPONENT & !(step - 1));
    if next() & 1 == 0 {
        magnitude as i32
    } else {
        -(magnitude as i32)
    }
}

/// Random single and paired differences of every lattice precision, written
/// by the facade, read back by the decoder's `read_mv`.
#[test]
fn random_differences_round_trip_through_read_mv() {
    let mut state = 0x5EED_u64;
    for precision in [
        MV_PRECISION_EIGHT_PEL,
        MV_PRECISION_FOUR_PEL,
        MV_PRECISION_ONE_PEL,
        MV_PRECISION_HALF_PEL,
        MV_PRECISION_QUARTER_PEL,
        MV_PRECISION_EIGHTH_PEL,
    ] {
        let cases: Vec<Vec<MotionVector>> = (0..400)
            .map(|case| {
                (0..=(case % 2))
                    .map(|_| MotionVector {
                        row: component(&mut state, precision),
                        col: component(&mut state, precision),
                    })
                    .collect()
            })
            .collect();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let mut journal = CdfJournal::new(&cdfs);
        let mut encoder = SymbolEncoder::new();
        for mvds in &cases {
            write_mvds((&mut cdfs, &mut journal), precision, mvds, &mut encoder).unwrap();
        }
        let payload = encoder.finish().unwrap().into_bytes();
        let mut symbols = SymbolDecoder::with_base_and_config(
            &payload,
            ByteOffset::new(0),
            SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
        )
        .unwrap();
        let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
        let (config, offset) = (MvReadConfig::inter(precision), ByteOffset::new(0));
        let mut classes = BTreeSet::new();
        for mvds in &cases {
            let mut read = || {
                read_newmv_block_mvd_magnitude_with_config(&mut cdfs, &mut symbols, offset, config)
                    .unwrap()
            };
            let decoded: Vec<MotionVector> = if let [_] = mvds[..] {
                let magnitude = read();
                let mv = apply_inter_mvd_signs(magnitude, &mut symbols, offset, config, false, 1);
                vec![mv.unwrap().into()]
            } else {
                let (first, second) = (read(), read());
                let (first, second) = apply_inter_mvd_sign_pair(
                    first,
                    second,
                    &mut symbols,
                    offset,
                    config,
                    false,
                    1,
                )
                .unwrap();
                vec![first.into(), second.into()]
            };
            assert_eq!(&decoded, mvds, "precision {precision}");
            classes.extend(mvds.iter().map(|&mvd| shell(mvd.into(), precision).0));
        }
        let max = shell(
            Mv {
                row: MAX_COMPONENT as i32,
                col: MAX_COMPONENT as i32,
            },
            precision,
        )
        .0;
        assert_eq!(classes, (0..=max).collect(), "precision {precision}");
    }
}
