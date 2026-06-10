// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: the AV2 validator must return a ValidationReport, never panic, on
// arbitrary input. This is the highest-coverage target: validation transitively
// reaches every OBU payload parser, both container formats, and every validator
// check. cargo-fuzz requires a NIGHTLY toolchain (AddressSanitizer + coverage are
// nightly-only). Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run validate_bytes
//
// On stable, the same invariant is covered by the `validator_never_panics` proptest
// in `crates/splot-validate/tests/validator_never_panics.rs`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_validate::Validator;
use splot_validate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};

fuzz_target!(|data: &[u8]| {
    // Derive the strictness flag and validation options deterministically from the
    // first byte so that option-gated branches (external-HLS resolution,
    // AV2 § 7.3.8) are exercised; the rest of the input is the bitstream. Empty
    // input falls back to defaults over an empty slice.
    let (config, bitstream) = match data.split_first() {
        Some((config, bitstream)) => (*config, bitstream),
        None => (0, &[][..]),
    };

    let strict = config & 0b0000_0001 != 0;
    let options = if config & 0b0000_0010 != 0 {
        // Seed a few external-HLS keys from the remaining option bits so the
        // Provided branch is reachable with non-empty declarations.
        let set = ExternalHlsSet::new()
            .with_sequence_header_id(u32::from(config >> 4))
            .with_operating_point_set(config & 0b0001_1111, (config >> 2) & 0b0000_0011);
        ValidationOptions {
            external_hls: ExternalHlsMode::Provided(set),
        }
    } else {
        ValidationOptions::default()
    };

    let validator = Validator::new(strict);
    let report = validator.validate_bytes_with_options(bitstream, &options);
    let _ = report.diagnostics.len();
});
