// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_decode::fuzzing::run_tile_payload_decode_fuzz_case;

fuzz_target!(|data: &[u8]| {
    let outcome = run_tile_payload_decode_fuzz_case(data);

    if let Some(boundary) = outcome.boundary {
        assert_eq!(boundary.tile_num, 0);
        assert_eq!(boundary.tile_bytes_len as u64, boundary.tile_size);
    } else {
        assert!(outcome.typed_error);
    }
});
