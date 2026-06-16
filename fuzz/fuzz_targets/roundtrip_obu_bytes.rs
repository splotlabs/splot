// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target for ENC-BITSTREAM-WRITER: the writer is the inverse of the parser, so
// `parse -> write -> reparse` over arbitrary OBUs must round-trip. For every OBU whose
// payload parses to a `ParsedObu`, the round-trip harness must return `RoundTripped`
// (the five written OBU types) or `Unwritable` (an OBU type with no body writer yet) —
// never a panic and never a round-trip `Failed`. This target parses arbitrary bytes as an
// Annex B stream and drives `splot_core::write::roundtrip_obu`; it does not invoke the
// validator, decoder, or any filesystem/output path. Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run roundtrip_obu_bytes
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::obu::PayloadStatus;
use splot_core::write::{RoundtripOutcome, roundtrip_obu};

fuzz_target!(|data: &[u8]| {
    // Partial-parse so a trailing structural error still exercises every OBU that did parse.
    let parsed = parse_annex_b_obus_partial(data);
    for env in &parsed.obus {
        // Only payloads that parse to a typed model can be round-tripped; Opaque / PrefixParsed /
        // Unimplemented payloads are out of scope here (the parser fuzz target covers their
        // never-panic invariant).
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
