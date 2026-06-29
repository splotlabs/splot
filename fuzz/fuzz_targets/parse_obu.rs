// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::annexb::parse_annex_b_obus;
use splot_core::leb128::read_leb128;
use splot_core::obu::read_obu_header;
use splot_core::span::ByteOffset;

fuzz_target!(|data: &[u8]| {
    let _ = read_leb128(data, ByteOffset::new(0));
    let _ = read_obu_header(data, ByteOffset::new(0));
    if let Ok(obus) = parse_annex_b_obus(data) {
        for obu in &obus {
            let _ = obu.payload_status();
        }
    }
});
