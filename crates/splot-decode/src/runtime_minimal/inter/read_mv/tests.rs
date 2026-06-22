// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `read_mv` shell-scheme unit tests.
//!
//! These cover the structural building blocks ([`mv_clamp_to_integer`] and the
//! [`read_ns`] § 4.11.13 arithmetic non-symmetric literal) that the shell decode
//! composes. The full end-to-end bit-exactness of the shell read is proven by the
//! `syn-2frame-subpel-inter-64x64.ivf` oracle fixture decode test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::SymbolDecoder;

use super::{MV_LOW, MV_UPP, mv_clamp_to_integer, read_ns};

#[test]
fn mv_clamp_to_integer_is_identity_in_range() {
    assert_eq!(mv_clamp_to_integer(0), 0);
    assert_eq!(mv_clamp_to_integer(4), 4);
    assert_eq!(mv_clamp_to_integer(-4), -4);
    assert_eq!(mv_clamp_to_integer(1000), 1000);
    assert_eq!(mv_clamp_to_integer(-1000), -1000);
}

#[test]
fn mv_clamp_to_integer_clamps_out_of_range() {
    assert_eq!(mv_clamp_to_integer(MV_LOW), MV_LOW + 8);
    assert_eq!(mv_clamp_to_integer(MV_LOW - 100), MV_LOW + 8);
    assert_eq!(mv_clamp_to_integer(MV_UPP), MV_UPP - 8);
    assert_eq!(mv_clamp_to_integer(MV_UPP + 100), MV_UPP - 8);
}

#[test]
fn read_ns_small_n_reads_nothing() {
    // n <= 1 has a single value (0) and consumes no bits.
    let mut symbols = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
    let before = symbols.consumed_bits();
    assert_eq!(read_ns(&mut symbols, 1, ByteOffset::new(0)).unwrap(), 0);
    assert_eq!(symbols.consumed_bits(), before);
    assert_eq!(read_ns(&mut symbols, 0, ByteOffset::new(0)).unwrap(), 0);
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn read_ns_returns_values_in_range() {
    // NS(n) returns values strictly in 0..n for any decoded bit pattern.
    for n in 2..16i64 {
        let mut symbols = SymbolDecoder::new(&[0xAA, 0x55, 0x80]).unwrap();
        let value = read_ns(&mut symbols, n, ByteOffset::new(0)).unwrap();
        assert!(
            (0..n).contains(&value),
            "NS({n}) returned {value}, outside 0..{n}"
        );
    }
}
