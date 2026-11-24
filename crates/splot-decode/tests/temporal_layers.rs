// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal enhancement layer (`D-2`) decode, dependency selection, streaming,
//! and fail-closed conformance tests.

#![expect(clippy::unwrap_used)]

use core::num::NonZeroU64;
use std::io::{self, Read};

use splot_core::annexb::parse_annex_b_obus;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{CoreSeqView, encode_show_existing_frame_obu};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::stream_reader::TemporalUnitReader;
use splot_core::types::{ExtendedLayerId, ObuType, TemporalLayerId};
use splot_core::write::{BitWriter, write_annexb_obu};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodePlannedObuRole, DecodeRuntimeConfig, Decoder, DecoderSettings, FrameDelay, Picture,
    PictureSamples, PlaneId, SendStatus, TemporalLayerSelection, ThreadCount,
};

const TLAYER2_BASE_AND_ENH: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-tlayer2-base-and-enhancement-64x64.obu"
);
const TLAYER2_DEP_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-tlayer2-dependent-8bit.ivf");
const TLAYER2_DEP_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-tlayer2-dependent-10bit.ivf");
const TLAYER4_SPARSE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-tlayer4-sparse-dependencies.obu");

const BASE_ENH_DIGESTS: &[(&str, u8)] = &[
    (
        "ce03721c5041e45d02106592c6c55db2bcda0d651568729b92f4ef5d4a665774",
        0,
    ),
    (
        "ce03721c5041e45d02106592c6c55db2bcda0d651568729b92f4ef5d4a665774",
        1,
    ),
];

const DEP_8BIT_DIGESTS: &[(&str, u8)] = &[
    (
        "0551c63c0534deba7c501a83f1d7f088dc40c24897d0008c963850e333efbe97",
        0,
    ),
    (
        "6ba58e4e61e0b3297e62f53ed9ab7a03a3c1507bbc3c408acdff1a0f3fc36984",
        1,
    ),
    (
        "b14dc41f7488f9446e80ff644c814a8764273ad04664afe2d3f1b49d46490fac",
        0,
    ),
    (
        "03aadfe5ea7a933f63962be3899bae88efef11c1ccbe46c15b57aa265ec488a8",
        1,
    ),
];

const DEP_10BIT_DIGESTS: &[(&str, u8)] = &[
    (
        "3dba796587f97d1f770ade1ef8d56077acd7ee85c862d5906e96b8c28eef30aa",
        0,
    ),
    (
        "951c0de9ee58d36008d0ff06cc40f7705d154ec13ba620ad56a0a7c0963bd0f3",
        1,
    ),
    (
        "e0db0d2065c710909bf97344178b10c42c3fca9768a399b542c76b843633e206",
        0,
    ),
    (
        "5bbf6d69ce5d7d4a86b6083491bb516c1edc7ea311949082e23d8ca1763f220a",
        1,
    ),
];

const SPARSE_DIGESTS: &[(&str, u8)] = &[
    (
        "38b0898c51be264f481b99e01a3dbb5e882c95acc6faae3d2de52b07855ee057",
        0,
    ),
    (
        "0fe1657597546af5ccb9d66bc644ccd39690de1d7cbcb4c67d5537cf878ba22d",
        1,
    ),
    (
        "bdd6cb0e609912c2654ccef7c0b0b960db77ff346576d0ef167287b13bdf4c46",
        2,
    ),
    (
        "f57956273cf7d4d10b22e3db499f4f4497fc78cda5d7c71f228c1023c79d6723",
        3,
    ),
    (
        "bb76cc6a36a203e1f0ff9765da33f1ae6e74c0a4b27568c9139a5f0f94c876ec",
        0,
    ),
];

struct ChunkedReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    chunk_size: usize,
}

impl<'a> ChunkedReader<'a> {
    const fn new(bytes: &'a [u8], chunk_size: usize) -> Self {
        Self {
            bytes,
            offset: 0,
            chunk_size,
        }
    }
}

impl Read for ChunkedReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.bytes[self.offset..];
        if remaining.is_empty() || buf.is_empty() {
            return Ok(0);
        }
        let n = remaining.len().min(buf.len()).min(self.chunk_size);
        buf[..n].copy_from_slice(&remaining[..n]);
        self.offset += n;
        Ok(n)
    }
}

fn context(threads: usize, frame_delay: usize) -> DecodeContext {
    DecodeContext::new(
        DecodeRuntimeConfig::new(ThreadCount::from(threads))
            .with_frame_delay(FrameDelay::from(frame_delay)),
    )
    .unwrap()
}

fn temporal_units(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut reader = TemporalUnitReader::new(bytes);
    let mut out = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        out.push(unit.bytes.to_vec());
    }
    out
}

fn write_obu(writer: &mut BitWriter, header: ObuHeader, payload: &[u8]) {
    write_annexb_obu(writer, &header, payload).unwrap();
}

fn stream_decode_with_settings(
    bytes: &[u8],
    settings: &DecoderSettings,
) -> Result<Vec<Picture>, DecodeError> {
    let mut decoder = Decoder::new(*settings)?;
    let mut pictures = Vec::new();
    for (idx, unit) in temporal_units(bytes).iter().enumerate() {
        let props = DataProps::default().with_pts(i64::try_from(idx).unwrap());
        loop {
            match decoder.send_data(unit, props)? {
                SendStatus::Accepted => break,
                SendStatus::Full => {
                    while let Some(picture) = decoder.get_picture()? {
                        pictures.push(picture);
                    }
                }
            }
        }
        while let Some(picture) = decoder.get_picture()? {
            pictures.push(picture);
        }
    }
    decoder.drain();
    while let Some(picture) = decoder.get_picture()? {
        pictures.push(picture);
    }
    Ok(pictures)
}

fn pictures_to_raw(pictures: &[Picture]) -> Vec<u8> {
    let mut out = Vec::new();
    for picture in pictures {
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            match picture.plane(plane) {
                Some(PictureSamples::Eight(plane)) => {
                    for row in plane.visible_rows() {
                        out.extend_from_slice(row);
                    }
                }
                Some(PictureSamples::Ten(plane)) => {
                    for sample in plane.visible_rows().flatten() {
                        out.extend_from_slice(&sample.to_le_bytes());
                    }
                }
                None => {}
            }
        }
    }
    out
}

fn raw_via_context(bytes: &[u8], selection: TemporalLayerSelection) -> Vec<u8> {
    let mut out = Vec::new();
    context(1, 2)
        .decode_raw_bytes(
            bytes,
            DecodeOptions::default().with_temporal_layer_selection(selection),
            &mut out,
        )
        .unwrap();
    out
}

fn verify_selection_digests(
    bytes: &[u8],
    expected_all: &[(&str, u8)],
    selection: TemporalLayerSelection,
    expected_mask: u8,
) {
    let options = DecodeOptions::default().with_temporal_layer_selection(selection);
    let report = context(1, 2)
        .decode_hash_report_bytes(bytes, options)
        .unwrap();
    let expected: Vec<(&str, u8)> = expected_all
        .iter()
        .copied()
        .filter(|&(_, tl)| (expected_mask & (1u8 << tl)) != 0)
        .collect();
    assert_eq!(
        report.frames.len(),
        expected.len(),
        "frame count mismatch for {selection:?}"
    );
    for (frame, &(digest, _tl)) in report.frames.iter().zip(&expected) {
        assert_eq!(
            frame.hashes[0].digest_hex, digest,
            "digest mismatch for {selection:?} at output_index {}",
            frame.output_index
        );
    }

    let expected_raw = raw_via_context(bytes, selection);
    let pics = stream_decode_with_settings(
        bytes,
        &DecoderSettings::default()
            .with_threads(ThreadCount::from(1usize))
            .with_frame_delay(FrameDelay::from(2usize))
            .with_temporal_layer_selection(selection),
    )
    .unwrap();
    assert_eq!(pics.len(), expected.len());
    for (pic, &(_digest, tl)) in pics.iter().zip(&expected) {
        assert_eq!(pic.temporal_layer_id(), TemporalLayerId::from_bits(tl));
    }
    assert_eq!(pictures_to_raw(&pics), expected_raw);
}

fn ivf_to_annex_b(ivf: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in temporal_units(ivf) {
        out.extend_from_slice(&unit);
    }
    out
}

fn retag_frame_obus_with_extension_xlayer3(ivf: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::new();
    for unit in temporal_units(ivf) {
        for obu in parse_annex_b_obus(&unit).unwrap() {
            let mut hdr = obu.header;
            if !hdr.obu_type.requires_global_xlayer() {
                assert!(!hdr.has_header_extension);
                hdr.has_header_extension = true;
                hdr.extended_layer_id = ExtendedLayerId::from_bits(3);
                hdr.header_size_bytes = 2;
            }
            write_obu(&mut writer, hdr, obu.payload);
        }
    }
    writer.into_bytes()
}

#[test]
fn single_and_extended_tlayer1_obu_headers_plan_and_decode_identically() {
    let base_annex_b = ivf_to_annex_b(TLAYER2_DEP_8BIT);
    let ext_annex_b = retag_frame_obus_with_extension_xlayer3(TLAYER2_DEP_8BIT);

    let base_plan = DecodeContext::plan_bytes(&base_annex_b, DecodeOptions::default()).unwrap();
    let ext_plan = DecodeContext::plan_bytes(&ext_annex_b, DecodeOptions::default()).unwrap();

    let base_tg1 = base_plan
        .obus()
        .find(|o| o.header().temporal_layer_id == TemporalLayerId::from_bits(1))
        .unwrap();
    let ext_tg1 = ext_plan
        .obus()
        .find(|o| o.header().temporal_layer_id == TemporalLayerId::from_bits(1))
        .unwrap();
    let base_obus = parse_annex_b_obus(&base_annex_b).unwrap();
    let ext_obus = parse_annex_b_obus(&ext_annex_b).unwrap();
    let base_tg1_env = base_obus
        .iter()
        .find(|o| o.header.temporal_layer_id == TemporalLayerId::from_bits(1))
        .unwrap();
    let ext_tg1_env = ext_obus
        .iter()
        .find(|o| o.header.temporal_layer_id == TemporalLayerId::from_bits(1))
        .unwrap();
    let base_hdr_start = base_tg1_env.payload_offset().get() as usize
        - usize::from(base_tg1_env.header.header_size_bytes);
    let ext_hdr_start = ext_tg1_env.payload_offset().get() as usize
        - usize::from(ext_tg1_env.header.header_size_bytes);
    assert_eq!(base_annex_b[base_hdr_start], 0x1D);
    assert_eq!(
        &ext_annex_b[ext_hdr_start..ext_hdr_start + 2],
        &[0x9D, 0x03]
    );
    assert!(!base_tg1.header().has_header_extension);
    assert!(ext_tg1.header().has_header_extension);
    assert_eq!(ext_tg1.header().extended_layer_id.get(), 3);
    assert_eq!(base_tg1.role(), DecodePlannedObuRole::InterFrameCandidate);
    assert_eq!(ext_tg1.role(), DecodePlannedObuRole::InterFrameCandidate);

    assert_eq!(
        raw_via_context(&ext_annex_b, TemporalLayerSelection::All),
        raw_via_context(&base_annex_b, TemporalLayerSelection::All)
    );
    let pics = stream_decode_with_settings(
        &ext_annex_b,
        &DecoderSettings::default().with_temporal_layer_selection(TemporalLayerSelection::All),
    )
    .unwrap();
    assert_eq!(pics.len(), 4);
    assert_eq!(pics[1].temporal_layer_id(), TemporalLayerId::from_bits(1));
    assert_eq!(pics[3].temporal_layer_id(), TemporalLayerId::from_bits(1));
}

#[test]
fn avm_digest_parity_and_picture_metadata_across_selection_modes() {
    for (fixture, expected) in [
        (TLAYER2_BASE_AND_ENH, BASE_ENH_DIGESTS),
        (TLAYER2_DEP_8BIT, DEP_8BIT_DIGESTS),
        (TLAYER2_DEP_10BIT, DEP_10BIT_DIGESTS),
    ] {
        verify_selection_digests(fixture, expected, TemporalLayerSelection::All, 0b1111);
        verify_selection_digests(fixture, expected, TemporalLayerSelection::BASE_ONLY, 0b0001);
        verify_selection_digests(
            fixture,
            expected,
            TemporalLayerSelection::Target(TemporalLayerId::from_bits(1)),
            0b0011,
        );
        verify_selection_digests(
            fixture,
            expected,
            TemporalLayerSelection::OutputMask(0b0010),
            0b0010,
        );
    }
}

#[test]
fn sparse_dependency_map_selection_modes_and_layer_skipping() {
    for (selection, mask) in [
        (TemporalLayerSelection::All, 0b1111),
        (TemporalLayerSelection::BASE_ONLY, 0b0001),
        (
            TemporalLayerSelection::Target(TemporalLayerId::from_bits(1)),
            0b0011,
        ),
        (
            TemporalLayerSelection::Target(TemporalLayerId::from_bits(2)),
            0b0111,
        ),
        (
            TemporalLayerSelection::Target(TemporalLayerId::from_bits(3)),
            0b1111,
        ),
        (TemporalLayerSelection::OutputMask(0b0100), 0b0100),
        (TemporalLayerSelection::OutputMask(0b1000), 0b1000),
        (TemporalLayerSelection::OutputMask(0b0101), 0b0101),
    ] {
        verify_selection_digests(TLAYER4_SPARSE, SPARSE_DIGESTS, selection, mask);
    }

    let mask8_plan = DecodeContext::plan_bytes(
        TLAYER4_SPARSE,
        DecodeOptions::default()
            .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b1000)),
    )
    .unwrap();
    let active_frame_tlayers: Vec<u8> = mask8_plan
        .obus()
        .filter(|o| o.role().is_frame_candidate())
        .map(|o| o.header().temporal_layer_id.get())
        .collect();
    assert_eq!(active_frame_tlayers, [0, 2, 3, 0]);
    let skipped_tl1: Vec<_> = mask8_plan
        .obus()
        .filter(|o| o.header().temporal_layer_id.get() == 1)
        .collect();
    assert_eq!(skipped_tl1.len(), 1);
    assert_eq!(skipped_tl1[0].role(), DecodePlannedObuRole::UnselectedLayer);
}

#[test]
fn chunked_annex_b_streaming_and_thread_widths_match_batch() {
    let annex_b = ivf_to_annex_b(TLAYER2_DEP_8BIT);
    for (fixture, is_annex_b) in [
        (&annex_b[..], true),
        (TLAYER2_DEP_10BIT, false),
        (TLAYER4_SPARSE, true),
    ] {
        for selection in [
            TemporalLayerSelection::All,
            TemporalLayerSelection::BASE_ONLY,
            TemporalLayerSelection::OutputMask(0b0010),
        ] {
            let expected = raw_via_context(fixture, selection);
            let options = DecodeOptions::default().with_temporal_layer_selection(selection);
            for chunk_size in [1usize, 7] {
                if is_annex_b {
                    let mut chunked_raw = Vec::new();
                    context(1, 2)
                        .decode_raw_reader(
                            ChunkedReader::new(fixture, chunk_size),
                            options,
                            &mut chunked_raw,
                        )
                        .unwrap();
                    assert_eq!(
                        chunked_raw, expected,
                        "chunked {chunk_size}B mismatch for {selection:?}"
                    );
                }
            }
            for threads in [1usize, 3, 12] {
                let mut threaded = Vec::new();
                context(threads, 3)
                    .decode_raw_bytes(fixture, options, &mut threaded)
                    .unwrap();
                assert_eq!(threaded, expected, "threads={threads} mismatch");

                let pics = stream_decode_with_settings(
                    fixture,
                    &DecoderSettings::default()
                        .with_threads(ThreadCount::from(threads))
                        .with_frame_delay(FrameDelay::from(3usize))
                        .with_temporal_layer_selection(selection),
                )
                .unwrap();
                assert_eq!(pictures_to_raw(&pics), expected);
            }
        }
    }
}

#[test]
fn output_frame_limits_do_not_charge_suppressed_prerequisite_frames() {
    let options = DecodeOptions::default()
        .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b1000))
        .with_output_frame_limit(NonZeroU64::new(1));
    let limited_options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(1)),
    )
    .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b1000));

    let report = context(1, 2)
        .decode_hash_report_bytes(TLAYER4_SPARSE, options)
        .unwrap();
    assert_eq!(report.frames.len(), 1);
    assert_eq!(report.frames[0].hashes[0].digest_hex, SPARSE_DIGESTS[3].0);

    let limited_report = context(1, 2)
        .decode_hash_report_bytes(TLAYER4_SPARSE, limited_options)
        .unwrap();
    assert_eq!(limited_report, report);

    let err_options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(2)),
    )
    .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b0101));
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(TLAYER4_SPARSE, err_options),
        Err(DecodeError::Limit { .. })
    ));

    let pics = stream_decode_with_settings(
        TLAYER4_SPARSE,
        &DecoderSettings::default()
            .with_limits(
                DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(1)),
            )
            .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b1000)),
    )
    .unwrap();
    assert_eq!(pics.len(), 1);
    assert_eq!(pics[0].temporal_layer_id(), TemporalLayerId::from_bits(3));
}

#[test]
fn reject_invalid_temporal_selection_and_excess_obu_tlayer_id() {
    for invalid in [
        TemporalLayerSelection::Target(TemporalLayerId::from_bits(4)),
        TemporalLayerSelection::OutputMask(0),
        TemporalLayerSelection::OutputMask(0x10),
    ] {
        let options = DecodeOptions::default().with_temporal_layer_selection(invalid);
        assert!(matches!(
            DecodeContext::plan_bytes(TLAYER2_DEP_8BIT, options),
            Err(DecodeError::InvalidTemporalSelection { .. })
        ));
        assert!(matches!(
            context(1, 1).decode_hash_report_bytes(TLAYER2_DEP_8BIT, options),
            Err(DecodeError::InvalidTemporalSelection { .. })
        ));
        assert!(matches!(
            Decoder::new(DecoderSettings::default().with_temporal_layer_selection(invalid)),
            Err(DecodeError::InvalidTemporalSelection { .. })
        ));
    }

    let mut excess_writer = BitWriter::new();
    let mut bumped = false;
    for unit in temporal_units(TLAYER2_DEP_8BIT) {
        for obu in parse_annex_b_obus(&unit).unwrap() {
            let mut hdr = obu.header;
            if hdr.obu_type == ObuType::RegularTileGroup && hdr.temporal_layer_id.get() == 1 {
                hdr.temporal_layer_id = TemporalLayerId::from_bits(2);
                bumped = true;
            }
            write_obu(&mut excess_writer, hdr, obu.payload);
        }
    }
    assert!(bumped);
    let excess_annex_b = excess_writer.into_bytes();
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(&excess_annex_b, DecodeOptions::default()),
        Err(DecodeError::MalformedSource { .. })
    ));
    assert!(matches!(
        stream_decode_with_settings(&excess_annex_b, &DecoderSettings::default()),
        Err(DecodeError::MalformedSource { .. })
    ));
}

#[test]
fn reject_inter_and_sef_references_violating_tlayer_dependency_map() {
    let mut bad_inter_writer = BitWriter::new();
    let mut retagged = false;
    for obu in parse_annex_b_obus(TLAYER4_SPARSE).unwrap() {
        let mut hdr = obu.header;
        if hdr.obu_type == ObuType::RegularTileGroup && hdr.temporal_layer_id.get() == 2 {
            hdr.temporal_layer_id = TemporalLayerId::from_bits(1);
            retagged = true;
        }
        write_obu(&mut bad_inter_writer, hdr, obu.payload);
    }
    assert!(retagged);
    let bad_inter = bad_inter_writer.into_bytes();
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(&bad_inter, DecodeOptions::default()),
        Err(DecodeError::MalformedSource { .. })
    ));

    let obus = parse_annex_b_obus(TLAYER4_SPARSE).unwrap();
    let seq_payload = obus
        .iter()
        .find(|o| o.header.obu_type == ObuType::SequenceHeader)
        .unwrap()
        .payload;
    let seq = parse_sequence_header(&mut BitReader::new(seq_payload, ByteOffset::new(0))).unwrap();
    let seq_view = CoreSeqView::from_sequence(&seq).unwrap();

    let mut prefix_writer = BitWriter::new();
    for obu in &obus[..=4] {
        write_obu(&mut prefix_writer, obu.header, obu.payload);
    }
    let prefix_through_tl1 = prefix_writer.into_bytes();

    let mut rejected_slots = Vec::new();
    let mut accepted_slots = Vec::new();
    for slot in 0..8u32 {
        let sef_bytes =
            encode_show_existing_frame_obu(&seq_view, slot, None, None, (false, false)).unwrap();
        let sef_obu = &parse_annex_b_obus(&sef_bytes).unwrap()[0];
        let mut hdr = sef_obu.header;
        hdr.temporal_layer_id = TemporalLayerId::from_bits(2);
        let mut candidate_writer = BitWriter::new();
        candidate_writer.write_le(&prefix_through_tl1).unwrap();
        write_obu(&mut candidate_writer, obus[0].header, obus[0].payload);
        write_obu(&mut candidate_writer, hdr, sef_obu.payload);
        let candidate = candidate_writer.into_bytes();
        match context(1, 2).decode_hash_report_bytes(&candidate, DecodeOptions::default()) {
            Ok(_) | Err(DecodeError::ReferenceState { .. }) => accepted_slots.push(slot),
            Err(DecodeError::MalformedSource { .. }) => rejected_slots.push(slot),
            Err(other) => panic!("unexpected error for slot {slot}: {other:?}"),
        }
    }
    assert!(
        !rejected_slots.is_empty(),
        "expected TL1-refreshed slot to fail TLayerDependencyMap SEF check"
    );
    assert!(
        !accepted_slots.is_empty(),
        "expected unrefreshed TL0 slot to pass TLayerDependencyMap SEF check (slots accepted={accepted_slots:?}, rejected={rejected_slots:?})"
    );
}

#[test]
fn truncated_tl1_continuation_tile_group_fails_closed_recovers_on_flush_and_passes_base_only() {
    let units = temporal_units(TLAYER2_DEP_8BIT);
    assert_eq!(units.len(), 4);
    let mut unit1_writer = BitWriter::new();
    let frame1_obus = parse_annex_b_obus(&units[1]).unwrap();
    assert_eq!(frame1_obus.len(), 3);
    assert_eq!(frame1_obus[2].header.temporal_layer_id.get(), 1);
    write_obu(
        &mut unit1_writer,
        frame1_obus[0].header,
        frame1_obus[0].payload,
    );
    write_obu(
        &mut unit1_writer,
        frame1_obus[1].header,
        frame1_obus[1].payload,
    );
    let short_cont_payload = &frame1_obus[2].payload[..8];
    write_obu(&mut unit1_writer, frame1_obus[2].header, short_cont_payload);
    let truncated_unit1 = unit1_writer.into_bytes();

    let mut truncated_annex_b = Vec::new();
    for (idx, unit) in units.iter().enumerate() {
        truncated_annex_b.extend_from_slice(if idx == 1 { &truncated_unit1 } else { unit });
    }

    assert_eq!(
        raw_via_context(&truncated_annex_b, TemporalLayerSelection::BASE_ONLY),
        raw_via_context(TLAYER2_DEP_8BIT, TemporalLayerSelection::BASE_ONLY)
    );
    assert!(
        context(1, 2)
            .decode_hash_report_bytes(&truncated_annex_b, DecodeOptions::default())
            .is_err()
    );
    assert!(
        context(1, 2)
            .decode_hash_report_bytes(
                &truncated_annex_b,
                DecodeOptions::default()
                    .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b0010)),
            )
            .is_err()
    );

    let mut decoder = Decoder::new(
        DecoderSettings::default()
            .with_threads(ThreadCount::from(1usize))
            .with_frame_delay(FrameDelay::from(2usize))
            .with_temporal_layer_selection(TemporalLayerSelection::OutputMask(0b0010)),
    )
    .unwrap();
    assert_eq!(
        decoder.send_data(&units[0], DataProps::default()).unwrap(),
        SendStatus::Accepted
    );
    assert_eq!(
        decoder
            .send_data(&truncated_unit1, DataProps::default())
            .unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    let mut saw_err = false;
    loop {
        match decoder.get_picture() {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                saw_err = true;
                break;
            }
        }
    }
    assert!(saw_err, "truncated TL1 continuation must fail closed");

    decoder.flush();
    let mut recovered = Vec::new();
    for unit in &units {
        assert_eq!(
            decoder.send_data(unit, DataProps::default()).unwrap(),
            SendStatus::Accepted
        );
        while let Some(pic) = decoder.get_picture().unwrap() {
            recovered.push(pic);
        }
    }
    decoder.drain();
    while let Some(pic) = decoder.get_picture().unwrap() {
        recovered.push(pic);
    }
    assert_eq!(
        pictures_to_raw(&recovered),
        raw_via_context(TLAYER2_DEP_8BIT, TemporalLayerSelection::OutputMask(0b0010))
    );
}
