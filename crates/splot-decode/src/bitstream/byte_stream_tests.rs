// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::DecodeUnsupportedReason;
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};

/// The base offset and OBU count of each unit `source` yields.
fn units(bytes: &[u8], options: &DecodeOptions) -> Result<(Vec<(u64, usize)>, usize)> {
    let mut source = ReaderUnits::new(bytes, options.limits());
    let mut buffers = Vec::new();
    let mut seen = Vec::new();
    while let Some(unit) = source.next_unit(&mut buffers)? {
        let mut obus = Vec::new();
        runtime_obus(unit.bytes.view(), &mut obus)?;
        seen.push((unit.bytes.view().base(), obus.len()));
    }
    Ok((seen, buffers.len()))
}

#[test]
fn ivf_records_are_read_one_at_a_time_and_share_reused_buffers() {
    let mut bytes = Vec::new();
    let header = IvfHeader::new(*b"AV02", 16, 16, 24, 1, 3);
    write_ivf_header(&mut bytes, &header).unwrap();
    write_ivf_frame(&mut bytes, 0, &[0x01, 0x08, 0x01, 0x04]).unwrap();
    write_ivf_frame(&mut bytes, 1, &[]).unwrap();
    write_ivf_frame(&mut bytes, 2, &[0x01, 0x10]).unwrap();
    let (seen, buffers) = units(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(seen, [(44, 2), (72, 1)]);
    assert_eq!(buffers, 1);
}

#[test]
fn ivf_payload_storage_moves_through_the_pool_without_touching_a_live_unit() {
    let mut input = Vec::new();
    write_ivf_header(&mut input, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 2)).unwrap();
    write_ivf_frame(&mut input, 7, &[0x01, 0x08]).unwrap();
    write_ivf_frame(&mut input, 8, &[0x01, 0x04]).unwrap();
    let mut source = ReaderUnits::new(input.as_slice(), DecodeLimits::unlimited());
    let spare = Vec::with_capacity(64);
    let spare_ptr = spare.as_ptr();
    let mut buffers = vec![Arc::new(spare)];

    let first = source.next_unit(&mut buffers).unwrap().unwrap();
    let second = source.next_unit(&mut buffers).unwrap().unwrap();

    assert_eq!(first.bytes.view().bytes(), &[0x01, 0x08]);
    assert_eq!(second.bytes.view().bytes(), &[0x01, 0x04]);
    assert_eq!(second.bytes.view().bytes().as_ptr(), spare_ptr);
    assert_eq!(buffers.len(), 2);
}

#[test]
fn annex_b_input_splits_into_units_at_temporal_delimiters() {
    let delimiter = [0x01, 0x08];
    let padding = [0x01, 0x04];
    let bytes = [
        delimiter.as_slice(),
        &padding,
        &padding,
        &delimiter,
        &padding,
        &delimiter,
    ]
    .concat();
    let (seen, _) = units(&bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(seen, [(0, 3), (6, 2), (10, 1)]);
}

#[test]
fn an_empty_annex_b_input_is_refused_and_an_empty_ivf_input_has_no_units() {
    let error = units(&[], &DecodeOptions::default()).unwrap_err();
    assert!(matches!(error, DecodeError::UnsupportedFeature { .. }));
    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 0)).unwrap();
    assert!(units(&ivf, &DecodeOptions::default()).unwrap().0.is_empty());
}

#[test]
fn input_limit_holds_for_the_bytes_read() {
    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 2)).unwrap();
    write_ivf_frame(&mut ivf, 0, &[0x01, 0x08, 0x01, 0x04]).unwrap();
    let annex_b = [0x01, 0x08, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_input_bytes(crate::DecodeLimitThreshold::Max(2)),
    );
    for bytes in [ivf.as_slice(), annex_b.as_slice()] {
        let error = units(bytes, &options).unwrap_err();
        assert!(matches!(
            error,
            DecodeError::Limit { source } if source.name() == DecodeLimitName::MaxInputBytes
        ));
    }
}

#[test]
fn raw_obu_limit_is_checked_before_parsing_next_obu() {
    let bytes = [0x01, 0x08, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_obus(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxObus
    ));

    let bytes = [0x02, 0xD0, 0x03, 0x01, 0x08];
    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::UnsupportedStructure {
            unsupported
        } if unsupported.reason() == DecodeUnsupportedReason::InvalidLayerScope
    ));
}

#[test]
fn malformed_suffix_is_reported_after_unsupported_prefix() {
    for bytes in [[0x01, 0x14, 0x05, 0x10], [0x01, 0x1D, 0x05, 0x10]] {
        let error = plan_byte_stream(&bytes, &DecodeOptions::default()).unwrap_err();

        assert!(matches!(
            error,
            crate::DecodeError::MalformedSource {
                issue
            } if issue.kind() == crate::DecodeSourceIssueKind::AnnexBParseError
        ));
    }
}

#[test]
fn raw_frame_candidate_limit_is_checked_before_later_malformed_bytes() {
    let bytes = [0x01, 0x10, 0x01, 0x10, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_frames_to_decode(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}

#[test]
fn raw_regular_tip_counts_toward_frame_candidate_limit() {
    let bytes = [0x01, 0x10, 0x01, 0x38];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_frames_to_decode(crate::DecodeLimitThreshold::Max(1)),
    );

    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::Limit {
            source
        } if source.name() == DecodeLimitName::MaxFramesToDecode
    ));
}
