// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: the current minimal splot-decode tile-payload byte boundary must
// return typed results, never panic, on bounded arbitrary tile payload bytes and
// bounded mutations of the known-good minimal frontier payload. This target
// intentionally does not implement full AV2 §5.20 decode_tile(), recursive
// partition/block syntax, broad §8.3 CDF selection, reconstruction expansion,
// filesystem I/O, subprocesses, AVM, dav2d, or ffmpeg invocation. Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run tile_payload_decode_bytes
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_decode::fuzzing::{TilePayloadFuzzStage, run_tile_payload_decode_fuzz_case};

const EXPECTED_UNSUPPORTED_RULE_ID: &str = "decode/unsupported-feature";
const EXPECTED_UNSUPPORTED_MATRIX_ROW: &str = "tile-payload-decode";
const EXPECTED_UNSUPPORTED_FEATURE_ID: &str = "DECODE-TILE-PAYLOAD-BOUNDARY";
const EXPECTED_UNSUPPORTED_SPEC_SECTION: &str = "5.20.2.1";
const EXPECTED_UNSUPPORTED_REASON: &str = "decode_tile_syntax";
const EXPECTED_FRONTIER_SYMBOL_COUNT: u64 = 6;
const EXPECTED_RECONSTRUCTION_TRACE: &str = "luma_dc_no_residual_8bit420_64x64";

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

    if let Some(frontier) = outcome.frontier {
        let boundary = outcome
            .boundary
            .expect("successful frontier requires a successful boundary");
        assert_eq!(frontier.symbol_count, EXPECTED_FRONTIER_SYMBOL_COUNT);
        assert_eq!(frontier.consumed_bits, frontier.padding_end_position);
        assert!(frontier.padding_end_position <= boundary.tile_size.saturating_mul(8));
        assert!(frontier.trailing_bit_position <= frontier.padding_end_position);
        assert_eq!(frontier.reconstruction_trace, EXPECTED_RECONSTRUCTION_TRACE);
        assert_eq!(outcome.typed_error_stage, None);
    } else if outcome.boundary.is_some() {
        assert!(matches!(
            outcome.typed_error_stage,
            None | Some(TilePayloadFuzzStage::Frontier)
        ));
    }
});
