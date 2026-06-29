// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::stream::parse_bitstream_partial;

fuzz_target!(|data: &[u8]| {
    let _ = parse_bitstream_partial(data);
});
