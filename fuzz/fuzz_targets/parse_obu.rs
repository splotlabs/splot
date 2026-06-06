// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: the AV2 parsers must return errors, never panic, on arbitrary
// input. cargo-fuzz requires a NIGHTLY toolchain (AddressSanitizer + coverage are
// nightly-only). Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run parse_obu
//
// On stable, the same invariant is covered by the `parsers_never_panic` proptest
// in `splot-core::annexb`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::annexb::parse_annex_b_obus;
use splot_core::leb128::read_leb128;
use splot_core::obu::read_obu_header;
use splot_core::span::ByteOffset;

fuzz_target!(|data: &[u8]| {
    let _ = read_leb128(data, ByteOffset::new(0));
    let _ = read_obu_header(data, ByteOffset::new(0));
    let _ = parse_annex_b_obus(data);
});
