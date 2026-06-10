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
    // Derive the strictness flag and validation options deterministically from a
    // leading config prefix so that option-gated branches (external-HLS
    // resolution, AV2 § 7.3.8) are exercised; the rest of the input is the
    // bitstream. Byte 0: bit 0 = strict, bit 1 = external-HLS Provided, bit 2 =
    // xlayer-id MSB, bits 3-7 = sequence-header id. When Provided, byte 1 carries
    // the full key ranges: bits 0-3 = ops_id (f(4), 0..=15), bits 4-7 plus the
    // MSB above = obu_xlayer_id (f(5), 0..=31) — the keys are caller-supplied, so
    // bitstream mutation alone could never reach ids the prefix cannot encode.
    // Empty input falls back to defaults over an empty slice.
    let (flags, rest) = match data.split_first() {
        Some((flags, rest)) => (*flags, rest),
        None => (0, &[][..]),
    };

    let strict = flags & 0b0000_0001 != 0;
    let (options, bitstream) = if flags & 0b0000_0010 != 0 {
        let (keys, bitstream) = match rest.split_first() {
            Some((keys, bitstream)) => (*keys, bitstream),
            None => (0, &[][..]),
        };
        let xlayer_id = ((flags & 0b0000_0100) << 2) | (keys >> 4);
        let set = ExternalHlsSet::new()
            .with_sequence_header_id(u32::from(flags >> 3))
            .with_operating_point_set(xlayer_id, keys & 0b0000_1111);
        let options = ValidationOptions {
            external_hls: ExternalHlsMode::Provided(set),
        };
        (options, bitstream)
    } else {
        (ValidationOptions::default(), rest)
    };

    let validator = Validator::new(strict);
    let report = validator.validate_bytes_with_options(bitstream, &options);
    let _ = report.diagnostics.len();
});
