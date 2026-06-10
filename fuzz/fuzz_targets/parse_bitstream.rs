// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: container auto-detection and OBU payload dispatch must return a
// partial parse, never panic, on arbitrary input (raw Annex B or IVF-wrapped).
// cargo-fuzz requires a NIGHTLY toolchain (AddressSanitizer + coverage are
// nightly-only). Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run parse_bitstream
//
// On stable, the same invariant is covered by the `bitstream_parser_never_panics`
// proptest in `splot-core::stream`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::stream::parse_bitstream_partial;

fuzz_target!(|data: &[u8]| {
    let _ = parse_bitstream_partial(data);
});
