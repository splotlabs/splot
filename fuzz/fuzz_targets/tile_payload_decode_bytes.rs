// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_decode::fuzzing::run_tile_payload_decode_fuzz_case;

const EXPECTED_UNSUPPORTED_RULE_ID: &str = "decode/unsupported-feature";
const EXPECTED_UNSUPPORTED_MATRIX_ROW: &str = "tile-payload-decode";
const EXPECTED_UNSUPPORTED_FEATURE_ID: &str = "DECODE-TILE-PAYLOAD-BOUNDARY";
const EXPECTED_UNSUPPORTED_SPEC_SECTION: &str = "5.20.2.1";
const EXPECTED_UNSUPPORTED_REASON: &str = "decode_tile_syntax";

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
        assert_eq!(boundary.unsupported_rule_id, EXPECTED_UNSUPPORTED_RULE_ID);
        assert_eq!(
            boundary.unsupported_matrix_row,
            EXPECTED_UNSUPPORTED_MATRIX_ROW
        );
        assert_eq!(
            boundary.unsupported_feature_id,
            EXPECTED_UNSUPPORTED_FEATURE_ID
        );
        assert_eq!(
            boundary.unsupported_spec_section,
            EXPECTED_UNSUPPORTED_SPEC_SECTION
        );
        assert_eq!(boundary.unsupported_reason, EXPECTED_UNSUPPORTED_REASON);
    } else {
        assert!(outcome.typed_error_stage.is_some());
    }
});
