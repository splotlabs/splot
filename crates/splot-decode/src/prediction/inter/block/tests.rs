// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;

use super::validate_intra_segment_id;
use crate::error::DecodeError;

#[test]
fn intra_segment_id_validation_accepts_last_active_segment() {
    assert_eq!(
        validate_intra_segment_id(3, 3, ByteOffset::new(11)).expect("segment id"),
        3
    );
}

#[test]
fn intra_segment_id_validation_rejects_out_of_range_segment() {
    let error = validate_intra_segment_id(4, 3, ByteOffset::new(11))
        .expect_err("segment_id above LastActiveSegId must fail closed");
    let DecodeError::UnsupportedFeature { unsupported } = error else {
        panic!("expected unsupported-feature");
    };
    assert_eq!(unsupported.reason(), "inter_intra_segment_id_out_of_range");
    assert_eq!(unsupported.spec_section(), "5.20.5.8");
}
