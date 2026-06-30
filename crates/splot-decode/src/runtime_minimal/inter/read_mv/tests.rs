// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{MV_LOW, MV_UPP, mv_clamp_to_integer, read_ns};

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
