// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stable-toolchain counterpart to the `validate_bytes` fuzz target: the validator
//! must always return a `ValidationReport` (never panic) on arbitrary bytes and
//! arbitrary options. Mirrors the `parsers_never_panic` proptests in `splot-core`.

use proptest::prelude::*;
use splot_validate::Validator;
use splot_validate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};

/// Derives validation options from two config bytes the same way the
/// `validate_bytes` fuzz target does, so both surfaces exercise the same
/// option-gated branches. `flags` bit 0 = strict, bit 1 = external-HLS Provided,
/// bit 2 = xlayer-id MSB, bits 3-7 = sequence-header id; `keys` bits 0-3 = ops_id
/// (full f(4) range), bits 4-7 plus the MSB = obu_xlayer_id (full f(5) range).
fn options_from_bytes(flags: u8, keys: u8) -> (bool, ValidationOptions) {
    let strict = flags & 0b0000_0001 != 0;
    let options = if flags & 0b0000_0010 != 0 {
        let xlayer_id = ((flags & 0b0000_0100) << 2) | (keys >> 4);
        let set = ExternalHlsSet::new()
            .with_sequence_header_id(u32::from(flags >> 3))
            .with_operating_point_set(xlayer_id, keys & 0b0000_1111);
        ValidationOptions {
            external_hls: ExternalHlsMode::Provided(set),
        }
    } else {
        ValidationOptions::default()
    };
    (strict, options)
}

proptest! {
    /// Validation never panics on arbitrary input bytes and arbitrary options; it
    /// always returns a `ValidationReport`.
    #[test]
    fn validator_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..2048),
        flags in any::<u8>(),
        keys in any::<u8>(),
    ) {
        let (strict, options) = options_from_bytes(flags, keys);
        let validator = Validator::new(strict);
        let report = validator.validate_bytes_with_options(&data, &options);
        let _ = report.diagnostics.len();
    }
}
