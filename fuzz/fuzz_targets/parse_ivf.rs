// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::ivf::{is_ivf, parse_ivf_header, parse_ivf_partial};

fuzz_target!(|data: &[u8]| {
    let _ = is_ivf(data);
    let _ = parse_ivf_header(data);
    let _ = parse_ivf_partial(data);
});
