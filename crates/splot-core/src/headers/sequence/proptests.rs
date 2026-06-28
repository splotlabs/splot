// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Property (no-panic) tests for the [`super`] sequence-header parsers.

use super::*;
use proptest::prelude::*;

proptest! {
    /// `sequence_header_obu()` general parsing must never panic on arbitrary input.
    #[test]
    fn parse_sequence_header_general_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..128),
    ) {
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let _ = parse_sequence_header_general(&mut reader);
    }

    /// The full `sequence_header_obu()` walk (general + all child configs) must
    /// never panic on arbitrary input (CLAUDE.md § 8 no-panic requirement).
    #[test]
    fn parse_sequence_header_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let _ = parse_sequence_header(&mut reader);
    }

    /// `timing_info()` parsing must never panic on arbitrary input.
    #[test]
    fn parse_timing_info_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let _ = parse_timing_info(&mut reader);
    }
}
