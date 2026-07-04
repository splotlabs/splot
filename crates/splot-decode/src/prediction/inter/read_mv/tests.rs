// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{
    MV_LOW, MV_PRECISION_EIGHT_PEL, MV_PRECISION_EIGHTH_PEL, MV_PRECISION_FOUR_PEL,
    MV_PRECISION_ONE_PEL, MV_UPP, Mv, amvd_index_to_mvd, lower_mv_precision, mv_clamp_to_integer,
    read_ns,
};

#[test]
fn mv_clamp_to_integer_is_identity_in_range() {
    for value in [0, 4, -4, 1000, -1000] {
        assert_eq!(mv_clamp_to_integer(value), value);
    }
}

#[test]
fn mv_clamp_to_integer_clamps_out_of_range() {
    for (value, clamped) in [
        (MV_LOW, MV_LOW + 8),
        (MV_LOW - 100, MV_LOW + 8),
        (MV_UPP, MV_UPP - 8),
        (MV_UPP + 100, MV_UPP - 8),
    ] {
        assert_eq!(mv_clamp_to_integer(value), clamped);
    }
}

#[test]
fn read_ns_small_n_reads_nothing() {
    let mut symbols = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
    let before = symbols.consumed_bits();

    for n in [1, 0] {
        assert_eq!(read_ns(&mut symbols, n, ByteOffset::new(0)).unwrap(), 0);
        assert_eq!(symbols.consumed_bits(), before);
    }
}

#[test]
fn read_ns_returns_values_in_range() {
    for n in 2..16i64 {
        let mut symbols = SymbolDecoder::new(&[0xAA, 0x55, 0x80]).unwrap();
        let value = read_ns(&mut symbols, n, ByteOffset::new(0)).unwrap();
        assert!(
            (0..n).contains(&value),
            "NS({n}) returned {value}, outside 0..{n}"
        );
    }
}

#[test]
fn amvd_index_table_matches_spec_values() {
    let offset = ByteOffset::new(0);
    let values: Vec<i32> = (0..=8)
        .map(|index| amvd_index_to_mvd(index, offset).unwrap())
        .collect();
    assert_eq!(values, [0, 2, 4, 6, 8, 16, 32, 64, 128]);
    assert!(amvd_index_to_mvd(9, offset).is_err());
}

/// § 5.20.7.13 `lower_mv_precision`: `aInt = Round2(a - 1, bits)`, sign
/// restored, clamp only when the value changed. Asymmetric row/col values
/// guard against a transposed or symmetric-masking implementation.
#[test]
fn lower_mv_precision_rounds_asymmetric_components_per_spec() {
    let one_pel = lower_mv_precision(MV_PRECISION_ONE_PEL, Mv { row: 13, col: -27 });
    assert_eq!(one_pel, Mv { row: 16, col: -24 });

    let four_pel = lower_mv_precision(MV_PRECISION_FOUR_PEL, Mv { row: 17, col: -48 });
    assert_eq!(four_pel, Mv { row: 32, col: -32 });

    let eight_pel = lower_mv_precision(MV_PRECISION_EIGHT_PEL, Mv { row: 63, col: -65 });
    assert_eq!(eight_pel, Mv { row: 64, col: -64 });

    let zero = lower_mv_precision(MV_PRECISION_ONE_PEL, Mv::ZERO);
    assert_eq!(zero, Mv::ZERO);

    let on_grid = lower_mv_precision(MV_PRECISION_ONE_PEL, Mv { row: -32, col: 8 });
    assert_eq!(on_grid, Mv { row: -32, col: 8 });

    let eighth_pel = lower_mv_precision(MV_PRECISION_EIGHTH_PEL, Mv { row: 5, col: -3 });
    assert_eq!(
        eighth_pel,
        Mv { row: 5, col: -3 },
        "radix 1 is the identity"
    );
}
