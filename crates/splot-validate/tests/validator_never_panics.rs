// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Stable-toolchain counterpart to the `validate_bytes` fuzz target: the validator
//! must always return a `ValidationReport` (never panic) on arbitrary bytes and
//! arbitrary options. Mirrors the `parsers_never_panic` proptests in `splot-core`.

use proptest::prelude::*;
use splot_validate::Validator;
use splot_validate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};

/// Derives validation options from a single byte the same way the `validate_bytes`
/// fuzz target does, so both surfaces exercise the same option-gated branches.
fn options_from_byte(config: u8) -> (bool, ValidationOptions) {
    let strict = config & 0b0000_0001 != 0;
    let options = if config & 0b0000_0010 != 0 {
        let set = ExternalHlsSet::new()
            .with_sequence_header_id(u32::from(config >> 4))
            .with_operating_point_set(config & 0b0001_1111, (config >> 2) & 0b0000_0011);
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
        config in any::<u8>(),
    ) {
        let (strict, options) = options_from_byte(config);
        let validator = Validator::new(strict);
        let report = validator.validate_bytes_with_options(&data, &options);
        // Touch the report so the call result is observed, not optimized away.
        let _ = report.diagnostics.len();
    }
}
