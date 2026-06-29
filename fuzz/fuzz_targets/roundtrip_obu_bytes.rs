// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::obu::PayloadStatus;
use splot_core::write::{RoundtripOutcome, roundtrip_obu};

fuzz_target!(|data: &[u8]| {
    let parsed = parse_annex_b_obus_partial(data);
    for env in &parsed.obus {
        let Ok(PayloadStatus::Parsed(model)) = env.payload_status() else {
            continue;
        };
        match roundtrip_obu(&env.header, env.payload, &model) {
            RoundtripOutcome::RoundTripped | RoundtripOutcome::Unwritable { .. } => {}
            RoundtripOutcome::Failed { reason } => panic!(
                "writer round-trip failed for a parsed OBU (obu_type {:?}): {reason}",
                env.header.obu_type
            ),
        }
    }
});
