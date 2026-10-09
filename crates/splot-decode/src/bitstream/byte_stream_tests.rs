// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::DecodeUnsupportedReason;
use splot_core::ivf::{IvfHeader, write_ivf_frame, write_ivf_header};

#[test]
fn ivf_records_are_read_one_at_a_time_and_share_reused_buffers() {
    let mut bytes = Vec::new();
    let header = IvfHeader::new(*b"AV02", 16, 16, 24, 1, 3);
    write_ivf_header(&mut bytes, &header).unwrap();
    write_ivf_frame(&mut bytes, 0, &[0x01, 0x08, 0x01, 0x04]).unwrap();
    write_ivf_frame(&mut bytes, 1, &[]).unwrap();
    write_ivf_frame(&mut bytes, 2, &[0x01, 0x10]).unwrap();
    let prepared = prepare_stream(&mut Cursor::new(&bytes), &DecodeOptions::default()).unwrap();
    assert_eq!(prepared.plan.obu_count(), 3);

    let mut reader = Cursor::new(&bytes);
    let mut buffers = Vec::new();
    let mut records =
        IvfRecords::new(&mut reader, header, bytes.len() as u64, &mut buffers).unwrap();
    let mut seen = Vec::new();
    while records.advance().unwrap() {
        let (unit, record) = records.current().unwrap();
        let mut obus = Vec::new();
        runtime_obus(unit.view(), &mut obus).unwrap();
        seen.push((record, unit.view().base(), obus.len()));
    }
    assert_eq!(seen, [(0, 44, 2), (1, 72, 1)]);
    assert_eq!(records.buffers.len(), 1);
}

#[test]
fn a_record_past_the_planned_end_is_refused_before_it_is_read() {
    let mut bytes = Vec::new();
    let header = IvfHeader::new(*b"AV02", 16, 16, 24, 1, 1);
    write_ivf_header(&mut bytes, &header).unwrap();
    let planned_end = bytes.len() as u64 + 12 + 4;
    write_ivf_frame(&mut bytes, 0, &[0x01, 0x08, 0x01, 0x04, 0x01, 0x10]).unwrap();
    let mut buffers = Vec::new();
    let mut reader = Cursor::new(&bytes);
    let mut records = IvfRecords::new(&mut reader, header, planned_end, &mut buffers).unwrap();
    assert!(matches!(records.advance(), Err(DecodeError::Input { .. })));
    assert!(buffers.is_empty());
}

/// A source whose end seek reports less than it then yields, like a file that
/// grows while it is planned.
struct StaleLength<'a>(Cursor<&'a [u8]>);

impl Read for StaleLength<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Seek for StaleLength<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        match position {
            SeekFrom::End(_) => Ok(1),
            position => self.0.seek(position),
        }
    }
}

#[test]
fn input_limit_holds_for_bytes_read_not_the_reported_length() {
    let mut ivf = Vec::new();
    write_ivf_header(&mut ivf, &IvfHeader::new(*b"AV02", 16, 16, 24, 1, 2)).unwrap();
    write_ivf_frame(&mut ivf, 0, &[0x01, 0x08, 0x01, 0x04]).unwrap();
    let annex_b = [0x01, 0x08, 0x05, 0x10];
    let options = DecodeOptions::new(
        DecodeLimits::unlimited().with_max_input_bytes(crate::DecodeLimitThreshold::Max(2)),
    );
    for bytes in [ivf.as_slice(), annex_b.as_slice()] {
        let error = prepare_stream(&mut StaleLength(Cursor::new(bytes)), &options)
            .err()
            .unwrap();
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

    let bytes = [0x01, 0x50, 0x01, 0x08];
    let error = plan_byte_stream(&bytes, &options).unwrap_err();

    assert!(matches!(
        error,
        crate::DecodeError::UnsupportedStructure {
            unsupported
        } if unsupported.reason() == DecodeUnsupportedReason::MultistreamSelection
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
