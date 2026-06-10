// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: the IVF container parsers must return errors, never panic, on
// arbitrary input. cargo-fuzz requires a NIGHTLY toolchain (AddressSanitizer +
// coverage are nightly-only). Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run parse_ivf
//
// On stable, the same invariant is covered by the `ivf_parsers_never_panic` proptest
// in `splot-core::ivf`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::ivf::{is_ivf, parse_ivf_header, parse_ivf_partial};

fuzz_target!(|data: &[u8]| {
    let _ = is_ivf(data);
    let _ = parse_ivf_header(data);
    let _ = parse_ivf_partial(data);
});
