// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Embedded enhancement layer (`D-3`) decode, 2D dependency selection, streaming,
//! and fail-closed conformance tests.

#![expect(clippy::unwrap_used)]

use core::num::NonZeroU64;
use std::io::{self, Read};

use splot_core::annexb::parse_annex_b_obus;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{CoreSeqView, encode_show_existing_frame_obu};
use splot_core::headers::sequence::{ProfileIdc, parse_sequence_header};
use splot_core::obu::{ObuHeader, read_obu_header_from_slice};
use splot_core::span::ByteOffset;
use splot_core::stream_reader::TemporalUnitReader;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, ObuType, TemporalLayerId};
use splot_core::write::{BitWriter, seq_tile::write_sequence_header, write_annexb_obu};
use splot_decode::{
    DataProps, DecodeContext, DecodeError, DecodeLimitThreshold, DecodeLimits, DecodeOptions,
    DecodeRuntimeConfig, Decoder, DecoderSettings, EmbeddedLayerSelection, FrameDelay, Picture,
    PictureSamples, PlaneId, SendStatus, TemporalLayerSelection, ThreadCount,
};

const MLAYER2_CROSSREF_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer2-crossref-8bit.obu");
const MLAYER2_CROSSREF_10BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer2-crossref-10bit.obu");
const MLAYER2_MIXEDRES_8BIT: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer2-mixedres-8bit.obu");
const MLAYER_TLAYER_SPARSE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-mlayer-tlayer-sparse-state.obu");

type FrameExpectation = (&'static str, u8, u8, usize, usize);

const CROSSREF_8BIT_FRAMES: &[FrameExpectation] = &[
    (
        "288626c4f445ff3ec23b3fb548b7bf7144489c8c01168f918bde42ffae8a18bc",
        0,
        0,
        64,
        64,
    ),
    (
        "812f3f22aeb0397e7934fb8345c12d65f8aeb0ab3358ccbf63f21e454ef8823e",
        1,
        0,
        64,
        64,
    ),
    (
        "ef3f613220c478096e71c71137a182aee96d40f840c043b57872e8a5af300ce4",
        0,
        0,
        64,
        64,
    ),
    (
        "f1b9adf579789ac71f1faec94960a561bf363b7448753856cc209f4629d98980",
        1,
        0,
        64,
        64,
    ),
    (
        "78fe8fe6cbfaa7a99891a4d91b5d611608b1101390aba8e9b7b2b956d86b9c79",
        0,
        0,
        64,
        64,
    ),
    (
        "b670dd6d7d6e5e0e6f22018444b916bb142a8a23f44d8bab13bb99b1d60fa644",
        1,
        0,
        64,
        64,
    ),
];

const CROSSREF_10BIT_FRAMES: &[FrameExpectation] = &[
    (
        "5f3e15e550268d3e21c7519a11c580ed6c5fce17dcf70508e3956e051b6a06ed",
        0,
        0,
        64,
        64,
    ),
    (
        "d16b1d89ab5f1fe11426c05e6d0208148646624f672ffa6f433c41abb9ac1fe1",
        1,
        0,
        64,
        64,
    ),
    (
        "88006be671fae476b1af520b784e4e69ab7e4465db2a54f5747014c7254f49fe",
        0,
        0,
        64,
        64,
    ),
    (
        "623a651dd6a386e4e9a6a1ab4bbe9fa68dee02aedf68e6044c54c55849b4e69d",
        1,
        0,
        64,
        64,
    ),
    (
        "7565d3afc702929d00082ea4b32c7c38f7951a63235adc62ea59aeb65d70b2e9",
        0,
        0,
        64,
        64,
    ),
    (
        "ba5306340f27f520941bd3aa6ce44ce6ee3e59e952716f8ebc7ea565d1eee4a6",
        1,
        0,
        64,
        64,
    ),
];

const MIXEDRES_8BIT_FRAMES: &[FrameExpectation] = &[
    (
        "72871f34c436e342b7a3ff5cd06065187537204a58007ff3e5967dfc37f5c944",
        0,
        0,
        128,
        128,
    ),
    (
        "43a620e7f289b6ae0fde7047b6cc79fc8f2e91a58bfec581be472101599b0c4a",
        1,
        0,
        256,
        256,
    ),
    (
        "9a65800fc01dcd61ac5a7b97fab9e708e0231885cb42bfe3434f2c46ef6f7911",
        0,
        0,
        128,
        128,
    ),
    (
        "99c3734efd4068db3307462d5b3e5d5d1f868efd2f773ea4da60da07932d60ce",
        1,
        0,
        256,
        256,
    ),
    (
        "7032464f5e57045b218b87b77dfd86eb6af35f8db541eb7b29987b98a32a72f2",
        0,
        0,
        128,
        128,
    ),
    (
        "36b2bdcea2ae3bd7a26675202d975e4468e142f3e1c0e80497c25a18d822de18",
        1,
        0,
        256,
        256,
    ),
];

const SPARSE_STATE_FRAMES: &[FrameExpectation] = &[
    (
        "288626c4f445ff3ec23b3fb548b7bf7144489c8c01168f918bde42ffae8a18bc",
        0,
        0,
        64,
        64,
    ),
    (
        "812f3f22aeb0397e7934fb8345c12d65f8aeb0ab3358ccbf63f21e454ef8823e",
        1,
        0,
        64,
        64,
    ),
    (
        "ef3f613220c478096e71c71137a182aee96d40f840c043b57872e8a5af300ce4",
        0,
        1,
        64,
        64,
    ),
    (
        "f1b9adf579789ac71f1faec94960a561bf363b7448753856cc209f4629d98980",
        1,
        1,
        64,
        64,
    ),
    (
        "78fe8fe6cbfaa7a99891a4d91b5d611608b1101390aba8e9b7b2b956d86b9c79",
        0,
        1,
        64,
        64,
    ),
    (
        "b670dd6d7d6e5e0e6f22018444b916bb142a8a23f44d8bab13bb99b1d60fa644",
        1,
        1,
        64,
        64,
    ),
    (
        "46c42673015b8b364bcad631239cb8466e6e0a639266ebb20b10e012577888b0",
        0,
        1,
        64,
        64,
    ),
    (
        "ad21eb73ded16bfee2867b96fcae7363d68baddff068f1aba95c26fa9ea5b735",
        1,
        1,
        64,
        64,
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

fn raw_via_context(
    bytes: &[u8],
    tlayer_selection: TemporalLayerSelection,
    mlayer_selection: EmbeddedLayerSelection,
) -> Vec<u8> {
    let mut out = Vec::new();
    context(1, 2)
        .decode_raw_bytes(
            bytes,
            DecodeOptions::default()
                .with_temporal_layer_selection(tlayer_selection)
                .with_embedded_layer_selection(mlayer_selection),
            &mut out,
        )
        .unwrap();
    out
}

fn verify_layer_subset(
    bytes: &[u8],
    expected_all: &[FrameExpectation],
    tlayer_selection: TemporalLayerSelection,
    mlayer_selection: EmbeddedLayerSelection,
    tlayer_mask: u8,
    mlayer_mask: u8,
) {
    let options = DecodeOptions::default()
        .with_temporal_layer_selection(tlayer_selection)
        .with_embedded_layer_selection(mlayer_selection);
    let report = context(1, 2)
        .decode_hash_report_bytes(bytes, options)
        .unwrap();
    let expected: Vec<FrameExpectation> = expected_all
        .iter()
        .copied()
        .filter(|&(_, ml, tl, _, _)| {
            (mlayer_mask & (1u8 << ml)) != 0 && (tlayer_mask & (1u8 << tl)) != 0
        })
        .collect();
    assert_eq!(
        report.frames.len(),
        expected.len(),
        "frame count mismatch for ({tlayer_selection:?}, {mlayer_selection:?})"
    );
    for (frame, &(digest, _, _, width, height)) in report.frames.iter().zip(&expected) {
        assert_eq!(
            frame.hashes[0].digest_hex, digest,
            "digest mismatch at output_index {} for ({tlayer_selection:?}, {mlayer_selection:?})",
            frame.output_index
        );
        assert_eq!(usize::try_from(frame.visible_luma_width).unwrap(), width);
        assert_eq!(usize::try_from(frame.visible_luma_height).unwrap(), height);
    }

    let expected_raw = raw_via_context(bytes, tlayer_selection, mlayer_selection);
    let pics = stream_decode_with_settings(
        bytes,
        &DecoderSettings::default()
            .with_threads(ThreadCount::from(1usize))
            .with_frame_delay(FrameDelay::from(2usize))
            .with_temporal_layer_selection(tlayer_selection)
            .with_embedded_layer_selection(mlayer_selection),
    )
    .unwrap();
    assert_eq!(pics.len(), expected.len());
    for (pic, &(_, ml, tl, width, height)) in pics.iter().zip(&expected) {
        assert_eq!(pic.embedded_layer_id(), EmbeddedLayerId::from_bits(ml));
        assert_eq!(pic.temporal_layer_id(), TemporalLayerId::from_bits(tl));
        assert_eq!(pic.width(), width);
        assert_eq!(pic.height(), height);
    }
    assert_eq!(pictures_to_raw(&pics), expected_raw);
}

#[test]
fn avm_digest_parity_dimensions_and_metadata_across_embedded_selections() {
    for (fixture, expected) in [
        (MLAYER2_CROSSREF_8BIT, CROSSREF_8BIT_FRAMES),
        (MLAYER2_CROSSREF_10BIT, CROSSREF_10BIT_FRAMES),
        (MLAYER2_MIXEDRES_8BIT, MIXEDRES_8BIT_FRAMES),
    ] {
        for (selection, mask) in [
            (EmbeddedLayerSelection::All, 0b11u8),
            (EmbeddedLayerSelection::BASE_ONLY, 0b01u8),
            (
                EmbeddedLayerSelection::Target(EmbeddedLayerId::from_bits(1)),
                0b10u8,
            ),
            (EmbeddedLayerSelection::OutputMask(0b10), 0b10u8),
            (EmbeddedLayerSelection::OutputMask(0b11), 0b11u8),
        ] {
            verify_layer_subset(
                fixture,
                expected,
                TemporalLayerSelection::All,
                selection,
                0x0F,
                mask,
            );
        }
    }
}

#[test]
fn joint_embedded_and_temporal_2d_layer_selection() {
    for (t_sel, m_sel, t_mask, m_mask) in [
        (
            TemporalLayerSelection::All,
            EmbeddedLayerSelection::All,
            0b11u8,
            0b11u8,
        ),
        (
            TemporalLayerSelection::BASE_ONLY,
            EmbeddedLayerSelection::All,
            0b01u8,
            0b11u8,
        ),
        (
            TemporalLayerSelection::BASE_ONLY,
            EmbeddedLayerSelection::BASE_ONLY,
            0b01u8,
            0b01u8,
        ),
        (
            TemporalLayerSelection::BASE_ONLY,
            EmbeddedLayerSelection::OutputMask(0b10),
            0b01u8,
            0b10u8,
        ),
        (
            TemporalLayerSelection::OutputMask(0b10),
            EmbeddedLayerSelection::BASE_ONLY,
            0b10u8,
            0b01u8,
        ),
        (
            TemporalLayerSelection::OutputMask(0b10),
            EmbeddedLayerSelection::OutputMask(0b10),
            0b10u8,
            0b10u8,
        ),
        (
            TemporalLayerSelection::OutputMask(0b10),
            EmbeddedLayerSelection::Target(EmbeddedLayerId::from_bits(1)),
            0b10u8,
            0b10u8,
        ),
    ] {
        verify_layer_subset(
            MLAYER_TLAYER_SPARSE,
            SPARSE_STATE_FRAMES,
            t_sel,
            m_sel,
            t_mask,
            m_mask,
        );
    }
}

#[test]
fn chunked_annex_b_streaming_and_thread_widths_match_batch() {
    for fixture in [
        MLAYER2_CROSSREF_8BIT,
        MLAYER2_MIXEDRES_8BIT,
        MLAYER_TLAYER_SPARSE,
    ] {
        for selection in [
            EmbeddedLayerSelection::All,
            EmbeddedLayerSelection::BASE_ONLY,
            EmbeddedLayerSelection::OutputMask(0b10),
        ] {
            let expected = raw_via_context(fixture, TemporalLayerSelection::All, selection);
            let options = DecodeOptions::default().with_embedded_layer_selection(selection);
            for chunk_size in [1usize, 7] {
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
                        .with_embedded_layer_selection(selection),
                )
                .unwrap();
                assert_eq!(pictures_to_raw(&pics), expected);
            }
        }
    }

    let hdr = read_obu_header_from_slice(&[0x9D, 0x23], ByteOffset::new(0)).unwrap();
    assert!(hdr.has_header_extension);
    assert_eq!(hdr.obu_type, ObuType::RegularTileGroup);
    assert_eq!(hdr.temporal_layer_id, TemporalLayerId::from_bits(1));
    assert_eq!(hdr.embedded_layer_id, EmbeddedLayerId::from_bits(1));
    assert_eq!(hdr.extended_layer_id, ExtendedLayerId::from_bits(3));
    assert!(read_obu_header_from_slice(&[0x9D], ByteOffset::new(0)).is_err());
}

#[test]
fn output_frame_limits_do_not_charge_suppressed_ml0_prerequisites() {
    let options = DecodeOptions::default()
        .with_embedded_layer_selection(EmbeddedLayerSelection::OutputMask(0b10))
        .with_output_frame_limit(NonZeroU64::new(3));
    let limited_options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(3)),
    )
    .with_embedded_layer_selection(EmbeddedLayerSelection::OutputMask(0b10));

    let report = context(1, 2)
        .decode_hash_report_bytes(MLAYER2_CROSSREF_8BIT, options)
        .unwrap();
    assert_eq!(report.frames.len(), 3);
    assert_eq!(
        report.frames[0].hashes[0].digest_hex,
        CROSSREF_8BIT_FRAMES[1].0
    );
    assert_eq!(
        report.frames[1].hashes[0].digest_hex,
        CROSSREF_8BIT_FRAMES[3].0
    );
    assert_eq!(
        report.frames[2].hashes[0].digest_hex,
        CROSSREF_8BIT_FRAMES[5].0
    );

    let limited_report = context(1, 2)
        .decode_hash_report_bytes(MLAYER2_CROSSREF_8BIT, limited_options)
        .unwrap();
    assert_eq!(limited_report, report);

    let err_options = DecodeOptions::new(
        DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(2)),
    )
    .with_embedded_layer_selection(EmbeddedLayerSelection::OutputMask(0b10));
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(MLAYER2_CROSSREF_8BIT, err_options),
        Err(DecodeError::Limit { .. })
    ));

    let pics = stream_decode_with_settings(
        MLAYER2_CROSSREF_8BIT,
        &DecoderSettings::default()
            .with_limits(
                DecodeLimits::default().with_max_output_frames(DecodeLimitThreshold::max(3)),
            )
            .with_embedded_layer_selection(EmbeddedLayerSelection::OutputMask(0b10)),
    )
    .unwrap();
    assert_eq!(pics.len(), 3);
    for pic in &pics {
        assert_eq!(pic.embedded_layer_id(), EmbeddedLayerId::from_bits(1));
    }
}

#[test]
fn reject_invalid_embedded_selection_and_excess_obu_mlayer_id() {
    for invalid in [
        EmbeddedLayerSelection::Target(EmbeddedLayerId::from_bits(8)),
        EmbeddedLayerSelection::OutputMask(0),
    ] {
        let options = DecodeOptions::default().with_embedded_layer_selection(invalid);
        assert!(matches!(
            DecodeContext::plan_bytes(MLAYER2_CROSSREF_8BIT, options),
            Err(DecodeError::InvalidEmbeddedSelection { .. })
        ));
        assert!(matches!(
            context(1, 1).decode_hash_report_bytes(MLAYER2_CROSSREF_8BIT, options),
            Err(DecodeError::InvalidEmbeddedSelection { .. })
        ));
        assert!(matches!(
            Decoder::new(DecoderSettings::default().with_embedded_layer_selection(invalid)),
            Err(DecodeError::InvalidEmbeddedSelection { .. })
        ));
    }

    let mut excess_writer = BitWriter::new();
    let mut bumped = false;
    for unit in temporal_units(MLAYER2_CROSSREF_8BIT) {
        for obu in parse_annex_b_obus(&unit).unwrap() {
            let mut hdr = obu.header;
            if hdr.obu_type == ObuType::RegularTileGroup && hdr.embedded_layer_id.get() == 1 {
                hdr.embedded_layer_id = EmbeddedLayerId::from_bits(2);
                bumped = true;
            }
            write_obu(&mut excess_writer, hdr, obu.payload);
        }
    }
    assert!(bumped);
    let excess = excess_writer.into_bytes();
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(&excess, DecodeOptions::default()),
        Err(DecodeError::MalformedSource { .. })
    ));
    assert!(matches!(
        stream_decode_with_settings(&excess, &DecoderSettings::default()),
        Err(DecodeError::MalformedSource { .. })
    ));
}

#[test]
fn reject_mlayer_dependency_violation_annex_a_profile_and_missing_lcr() {
    let obus = parse_annex_b_obus(MLAYER2_CROSSREF_8BIT).unwrap();
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
    let prefix_tu0 = prefix_writer.into_bytes();

    let mut rejected_slots = Vec::new();
    let mut accepted_slots = Vec::new();
    for slot in 0..8u32 {
        let sef_bytes =
            encode_show_existing_frame_obu(&seq_view, slot, None, None, (false, false)).unwrap();
        let sef_obu = &parse_annex_b_obus(&sef_bytes).unwrap()[0];
        let mut candidate_writer = BitWriter::new();
        candidate_writer.write_le(&prefix_tu0).unwrap();
        write_obu(&mut candidate_writer, obus[0].header, obus[0].payload);
        write_obu(&mut candidate_writer, sef_obu.header, sef_obu.payload);
        let candidate = candidate_writer.into_bytes();
        match context(1, 2).decode_hash_report_bytes(&candidate, DecodeOptions::default()) {
            Ok(_) | Err(DecodeError::ReferenceState { .. }) => accepted_slots.push(slot),
            Err(DecodeError::MalformedSource { .. }) => rejected_slots.push(slot),
            Err(other) => panic!("unexpected error for slot {slot}: {other:?}"),
        }
    }
    assert!(
        !rejected_slots.is_empty(),
        "expected ML1-refreshed slot to fail MLayerDependencyMap check from ML0"
    );
    assert!(
        !accepted_slots.is_empty(),
        "expected ML0 slot to pass MLayerDependencyMap check (accepted={accepted_slots:?}, rejected={rejected_slots:?})"
    );

    let mut bad_profile_seq = seq;
    bad_profile_seq.general.seq_profile_idc = ProfileIdc::from_bits(0);
    let mut seq_bit_writer = BitWriter::new();
    write_sequence_header(&mut seq_bit_writer, &bad_profile_seq).unwrap();
    let bad_seq_payload = seq_bit_writer.into_bytes();

    let mut bad_profile_writer = BitWriter::new();
    for obu in &obus {
        if obu.header.obu_type == ObuType::SequenceHeader {
            write_obu(&mut bad_profile_writer, obu.header, &bad_seq_payload);
        } else {
            write_obu(&mut bad_profile_writer, obu.header, obu.payload);
        }
    }
    let bad_profile_stream = bad_profile_writer.into_bytes();
    assert!(matches!(
        context(1, 2).decode_hash_report_bytes(&bad_profile_stream, DecodeOptions::default()),
        Err(DecodeError::MalformedSource { issue }) if issue.spec_section() == Some("A.2")
    ));

    for fixture in [MLAYER2_CROSSREF_8BIT, MLAYER2_CROSSREF_10BIT] {
        let mut no_lcr_writer = BitWriter::new();
        for obu in parse_annex_b_obus(fixture).unwrap() {
            if obu.header.obu_type != ObuType::LayerConfigurationRecord {
                write_obu(&mut no_lcr_writer, obu.header, obu.payload);
            }
        }
        let no_lcr_stream = no_lcr_writer.into_bytes();
        assert!(matches!(
            context(1, 2).decode_hash_report_bytes(&no_lcr_stream, DecodeOptions::default()),
            Err(DecodeError::MalformedSource { issue })
                if issue.spec_section() == Some("A.2")
                    && issue.message().contains("Annex A Table A.4")
        ));
    }
}
