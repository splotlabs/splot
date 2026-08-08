// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_decode::fuzzing::run_tile_payload_decode_fuzz_case;

fuzz_target!(|data: &[u8]| {
    let outcome = run_tile_payload_decode_fuzz_case(data);

    if let Some(boundary) = outcome.boundary {
        assert_eq!(boundary.work_units_len, 1);
        assert_eq!(boundary.tile_num, 0);
        assert_eq!(boundary.tile_row, 0);
        assert_eq!(boundary.tile_col, 0);
        assert_eq!(boundary.tile_bytes_len as u64, boundary.tile_size);
        assert_eq!(
            boundary.symbol_consumed_bits,
            boundary.tile_size.saturating_mul(8).min(15)
        );
        assert_eq!(boundary.symbol_max_bits, boundary.tile_size as i64 * 8 - 15);
        assert_eq!(
            boundary.symbol_cdf_update_enabled,
            boundary.cdf_update_enabled
        );
    } else {
        assert!(outcome.typed_error_stage.is_some());
    }
});
