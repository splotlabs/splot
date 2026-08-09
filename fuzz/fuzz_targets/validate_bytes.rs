// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_validate::Validator;
use splot_validate::options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};

fuzz_target!(|data: &[u8]| {
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
    let _ = validator.validate_bytes_with_options(bitstream, &options);
});
