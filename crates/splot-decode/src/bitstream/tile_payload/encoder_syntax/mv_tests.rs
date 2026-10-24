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
            let coding = MvdCoding {
                precision,
                use_amvd: false,
                derive_sign: false,
                derive_threshold: 1,
            };
            write_mvds((&mut cdfs, &mut journal), coding, mvds, &mut encoder).unwrap();
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

/// Random adaptive-MVD differences, and coarse differences whose last sign
/// the decoder derives from the parity of their magnitudes, round-trip
/// through the decoder's reads; a target whose last sign is the other
/// parity is refused.
#[test]
fn adaptive_and_derived_sign_differences_round_trip() {
    let mut state = 0xA5D_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let amvd = |v: u64| match v % 9 {
        0 => 0,
        i => AMVD_INDEX_TO_MVD[i as usize] * if v & 16 == 0 { 1 } else { -1 },
    };
    let mut cases = Vec::new();
    for _ in 0..300 {
        let mvd = MotionVector {
            row: amvd(next()),
            col: amvd(next() >> 8),
        };
        cases.push((true, MV_PRECISION_QUARTER_PEL, mvd));
    }
    let mut refused = 0;
    for _ in 0..300 {
        let mvd = MotionVector {
            row: component(&mut state, MV_PRECISION_ONE_PEL),
            col: component(&mut state, MV_PRECISION_ONE_PEL),
        };
        let magnitudes = (mvd.row.abs() >> 3) + (mvd.col.abs() >> 3);
        let last_negative = if mvd.col != 0 {
            mvd.col < 0
        } else {
            mvd.row < 0
        };
        if (mvd.row != 0 || mvd.col != 0) && (magnitudes & 1 == 1) != last_negative {
            let coding = MvdCoding {
                precision: MV_PRECISION_ONE_PEL,
                use_amvd: false,
                derive_sign: true,
                derive_threshold: 1,
            };
            let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
            let mut journal = CdfJournal::new(&cdfs);
            let written = write_mvds(
                (&mut cdfs, &mut journal),
                coding,
                &[mvd],
                &mut SymbolEncoder::new(),
            );
            assert!(
                matches!(written, Err(EncoderSyntaxError::Unsupported(_))),
                "{mvd:?}"
            );
            refused += 1;
            continue;
        }
        cases.push((false, MV_PRECISION_ONE_PEL, mvd));
    }
    assert!(refused > 50, "{refused} refused");
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut journal = CdfJournal::new(&cdfs);
    let mut encoder = SymbolEncoder::new();
    for &(use_amvd, precision, mvd) in &cases {
        let coding = MvdCoding {
            precision,
            use_amvd,
            derive_sign: !use_amvd,
            derive_threshold: 1,
        };
        write_mvds((&mut cdfs, &mut journal), coding, &[mvd], &mut encoder).unwrap();
    }
    let payload = encoder.finish().unwrap().into_bytes();
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap();
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let offset = ByteOffset::new(0);
    for &(use_amvd, precision, mvd) in &cases {
        let config = MvReadConfig::inter(precision);
        let magnitude = if use_amvd {
            read_newmv_amvd_block_mvd(&mut cdfs, &mut symbols, offset).unwrap()
        } else {
            read_newmv_block_mvd_magnitude_with_config(&mut cdfs, &mut symbols, offset, config)
                .unwrap()
        };
        let read =
            apply_inter_mvd_signs(magnitude, &mut symbols, offset, config, !use_amvd, 1).unwrap();
        assert_eq!(MotionVector::from(read), mvd, "amvd {use_amvd}");
    }
}
