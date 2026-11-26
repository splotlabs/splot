// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Public decoder checks with expectations from [AVM].
//! Build AVM with `CONFIG_INSPECTION=1` and feed the IVF packets to `examples/inspect.c`.
//! Register an `inspect_sb_cb` with `AV2_SET_INSPECTION_CALLBACK`.
//! In that callback, read the settled nodes in `xd.sbi->ptree_root`.
//! Use `mi_col` and `mi_row` times `MI_SIZE` for their origins.
//! In `av2/decoder/inspection.c`, read the block modes, `current_qindex`,
//! `segment_id`, `mrl_index`, `pb_mv_precision` and `mv` from `MB_MODE_INFO`.
//! Use `get_ref_frame_map_idx` for the reference slots.
//!
//! [AVM]: https://github.com/AOMediaCodec/avm/tree/a1ec4594d97028ebc34083ec8346616d697c2fc9

#![expect(clippy::unwrap_used)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::block_evidence::{
    BlockEvidenceData, BlockEvidenceKind, BlockEvidenceRecord, BlockEvidenceReport,
    BlockEvidenceRequest, BlockEvidenceStatus, BlockEvidenceTree, BlockInterMode,
    BlockLumaPrediction, BlockMotion, BlockMotionPrecision, BlockPartition, BlockPrediction,
    BlockReference, CompoundMode, InterMode, IntraMode, MotionVector,
};
use splot_decode::{
    DataProps, DecodeError, Decoder, DecoderSettings, FrameDelay, Picture, PictureFrame,
    SendStatus, ThreadCount,
};
use splot_recon::{DecodedFrameHashInput, PixelFormat, PlaneRect};

const TILES: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2tile-zone1-edge-128x64-q192.ivf");
const MRL: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-mrl-padded-444-67x67-10bit-q120.ivf"
);
const SDP: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-lossless-sdp-nondc-luma-d67-chroma-follow-intra-64x64.ivf"
);
const SUBPEL: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2frame-subpel-inter-64x64.ivf");
const SAME_REFERENCE: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-2frame-sameref-compound-64x32-10bit-q150.ivf"
);
const COPY: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-intrabc-bottom-edge-320x180-q180.ivf"
);
const BRIDGE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-bridge-celu-64x64.ivf");

fn rect(x: usize, y: usize, width: usize, height: usize) -> PlaneRect {
    PlaneRect::new(x, y, width, height).unwrap()
}

fn settings(threads: usize) -> DecoderSettings {
    DecoderSettings::default()
        .with_threads(ThreadCount::from(threads))
        .with_frame_delay(FrameDelay::from(threads))
}

fn decode(decoder: &mut Decoder, fixture: &[u8]) -> Result<Vec<Picture>, DecodeError> {
    let mut reader = TemporalUnitReader::new(fixture);
    let mut pictures = Vec::new();
    while let Some(unit) = reader.next_temporal_unit().unwrap() {
        let props = DataProps::default().with_user_data(17 + unit.index as u64);
        while decoder.send_data(unit.bytes, props)? == SendStatus::Full {
            if let Some(picture) = decoder.get_picture()? {
                pictures.push(picture);
            }
        }
    }
    decoder.drain();
    while let Some(picture) = decoder.get_picture()? {
        pictures.push(picture);
    }
    Ok(pictures)
}

fn capture(
    fixture: &[u8],
    threads: usize,
    request: BlockEvidenceRequest,
) -> (BlockEvidenceReport, Vec<Picture>) {
    let (mut decoder, capture) =
        Decoder::new_with_block_evidence(settings(threads), request).unwrap();
    assert!(!capture.is_finished());
    assert!(capture.take().is_none());
    let pictures = decode(&mut decoder, fixture).unwrap();
    assert!(capture.is_finished());
    let report = capture.take().unwrap();
    assert!(capture.take().is_none());
    (report, pictures)
}

fn picture_hashes(pictures: &[Picture]) -> Vec<String> {
    pictures
        .iter()
        .map(|picture| match picture.decoded_frame().unwrap() {
            PictureFrame::Eight(frame) => DecodedFrameHashInput::new(frame).compute_hash().to_hex(),
            PictureFrame::Ten(frame) => DecodedFrameHashInput::new(frame).compute_hash().to_hex(),
        })
        .collect()
}

#[test]
fn partition_evidence_preserves_tile_indices_and_parent_rectangles() {
    for threads in [1, 4] {
        let (report, _) = capture(
            TILES,
            threads,
            BlockEvidenceRequest::new(|_| true, None, 4, usize::MAX)
                .with_kind(BlockEvidenceKind::Partitions),
        );
        assert_eq!(report.status, BlockEvidenceStatus::Complete);
        let mut records = report.records;
        records.sort_by_key(|record| {
            let coded = record.id.coded;
            (
                record.id.tile,
                coded.y(),
                core::cmp::Reverse(coded.height()),
            )
        });
        assert_eq!(records.len(), 4);
        for (record, (tile, coded, parent, partition)) in records.iter().zip([
            (0, rect(0, 0, 64, 64), None, BlockPartition::None),
            (1, rect(64, 0, 64, 64), None, BlockPartition::Horz),
            (
                1,
                rect(64, 0, 64, 32),
                Some(records[1].id),
                BlockPartition::None,
            ),
            (
                1,
                rect(64, 32, 64, 32),
                Some(records[1].id),
                BlockPartition::None,
            ),
        ]) {
            assert_eq!(record.id.tile, tile);
            assert_eq!(record.id.tree, BlockEvidenceTree::Shared);
            assert_eq!(record.id.coded, coded);
            assert_eq!(record.visible, Some(coded));
            assert_eq!(record.parent, parent);
            assert_eq!(record.data, BlockEvidenceData::Partition(partition));
        }
    }
}

#[test]
fn region_and_budgets_limit_records_without_changing_output() {
    let mut ordinary = Decoder::new(settings(1)).unwrap();
    let expected = picture_hashes(&decode(&mut ordinary, TILES).unwrap());
    let record_size = size_of::<BlockEvidenceRecord>();
    for (region, events, bytes, status, count) in [
        (
            rect(64, 31, 1, 2),
            2,
            2 * record_size,
            BlockEvidenceStatus::Complete,
            2,
        ),
        (
            rect(64, 31, 1, 2),
            1,
            2 * record_size,
            BlockEvidenceStatus::Truncated,
            1,
        ),
        (
            rect(64, 31, 1, 2),
            2,
            2 * record_size - 1,
            BlockEvidenceStatus::Truncated,
            1,
        ),
        (rect(64, 31, 1, 2), 2, 0, BlockEvidenceStatus::Truncated, 0),
        (
            rect(64, 31, 1, 2),
            0,
            2 * record_size,
            BlockEvidenceStatus::Truncated,
            0,
        ),
        (rect(128, 0, 1, 1), 0, 0, BlockEvidenceStatus::Complete, 0),
    ] {
        let (report, pictures) = capture(
            TILES,
            1,
            BlockEvidenceRequest::new(|_| true, Some(region), events, bytes)
                .with_kind(BlockEvidenceKind::Blocks),
        );
        assert_eq!(report.status, status);
        assert_eq!(report.records.len(), count);
        assert!(report.records.capacity() <= bytes / record_size);
        for (record, y) in report.records.iter().zip([0, 32]) {
            assert_eq!(record.id.tile, 1);
            assert_eq!(record.id.coded, rect(64, y, 64, 32));
            assert_eq!(record.visible, Some(record.id.coded));
            assert_eq!(record.parent.unwrap().coded, rect(64, 0, 64, 64));
            assert!(matches!(
                record.data,
                BlockEvidenceData::Block(facts) if facts.qindex == 192 && facts.segment_id == 0
            ));
        }
        assert_eq!(picture_hashes(&pictures), expected);
    }
}

#[test]
fn separate_plane_trees_keep_distinct_block_identities() {
    let (report, _) = capture(
        SDP,
        1,
        BlockEvidenceRequest::new(|_| true, None, 2, usize::MAX)
            .with_kind(BlockEvidenceKind::Blocks),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 2);
    let luma = &report.records[0];
    let chroma = &report.records[1];
    assert_eq!(luma.id.coded, rect(0, 0, 64, 64));
    assert_eq!(chroma.id.coded, luma.id.coded);
    assert_eq!(luma.id.tree, BlockEvidenceTree::Luma);
    assert_eq!(chroma.id.tree, BlockEvidenceTree::Chroma);
    assert_ne!(chroma.id, luma.id);
    assert!(matches!(
        luma.data,
        BlockEvidenceData::Block(facts) if facts.qindex == 0
            && matches!(facts.prediction, BlockPrediction::Intra(pred)
                if pred.chroma.is_none()
                    && matches!(pred.luma, Some(BlockLumaPrediction::Mode { mode: IntraMode::D67, .. })))
    ));
    assert!(matches!(
        chroma.data,
        BlockEvidenceData::Block(facts) if facts.qindex == 0
            && matches!(facts.prediction, BlockPrediction::Intra(pred)
                if pred.chroma.is_some() && pred.luma.is_none())
    ));
}

#[test]
fn padded_444_block_preserves_visible_geometry_and_reference_line() {
    let (report, pictures) = capture(
        MRL,
        1,
        BlockEvidenceRequest::new(|_| true, Some(rect(32, 66, 1, 1)), 1, usize::MAX)
            .with_kind(BlockEvidenceKind::Blocks),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(pictures[0].pixel_format(), PixelFormat::Yuv444);
    assert_eq!((pictures[0].width(), pictures[0].height()), (67, 67));
    assert_eq!(report.records.len(), 1);
    let record = report.records[0];
    assert_eq!(record.id.coded, rect(32, 64, 16, 8));
    assert_eq!(record.visible, Some(rect(32, 64, 16, 3)));
    assert_eq!(record.parent.unwrap().coded, rect(32, 64, 16, 16));
    assert!(matches!(
        record.data,
        BlockEvidenceData::Block(facts) if facts.qindex == 168
            && facts.segment_id == 0 && facts.has_chroma
            && matches!(facts.prediction, BlockPrediction::Intra(pred)
                if matches!(pred.luma, Some(BlockLumaPrediction::Mode {
                    mode: IntraMode::D203, direction: Some(direction), dpcm: None, ..
                }) if direction.angle_delta == 2 && direction.reference_line == 1))
    ));
}

#[test]
fn capture_selects_the_coded_obu_and_reports_resolved_subpel_motion() {
    for threads in [1, 4] {
        let (report, pictures) = capture(
            SUBPEL,
            threads,
            BlockEvidenceRequest::new(
                |coded| coded.user_data == 18 && coded.offset.get() == 3,
                None,
                2,
                usize::MAX,
            )
            .with_kind(BlockEvidenceKind::Blocks),
        );
        assert_eq!(report.status, BlockEvidenceStatus::Complete);
        assert_eq!(pictures.len(), 2);
        let frame = report.frame.unwrap();
        let picture = &pictures[1];
        assert_eq!(frame.provenance, picture.provenance());
        assert_eq!(frame.provenance.sequence_header.user_data, 17);
        assert_eq!(frame.provenance.sequence_header.offset.get(), 3);
        assert_eq!(frame.extended_layer_id, picture.extended_layer_id());
        assert_eq!(frame.embedded_layer_id, picture.embedded_layer_id());
        assert_eq!(frame.temporal_layer_id, picture.temporal_layer_id());
        assert_eq!(report.records.len(), 2);
        let block = report.records[0];
        let motion = report.records[1];
        assert_eq!(block.id.coded, rect(0, 0, 64, 64));
        assert_eq!(motion.id, block.id);
        assert!(matches!(
            block.data,
            BlockEvidenceData::Block(facts) if facts.qindex == 119
                && matches!(facts.prediction, BlockPrediction::Inter(pred)
                    if pred.mode == BlockInterMode::Single(InterMode::New)
                        && pred.precision == Some(BlockMotionPrecision::EighthPel)
                        && matches!(pred.references, [
                            Some(BlockReference::Frame { list_index: 0, slot: 0, scaled: Some(false), .. }), None,
                        ]))
        ));
        assert!(matches!(
            motion.data,
            BlockEvidenceData::Motion(BlockMotion::Inter(motion))
                if motion.vectors == [Some(MotionVector { row: 0, col: -4 }), None]
        ));
    }
}

#[test]
fn frame_range_preserves_coded_order_and_shares_budgets() {
    let record_size = size_of::<BlockEvidenceRecord>();
    for (events, bytes, final_count, final_status) in [
        (3, 3 * record_size, 2, BlockEvidenceStatus::Complete),
        (2, 3 * record_size, 1, BlockEvidenceStatus::Truncated),
        (3, 3 * record_size - 1, 1, BlockEvidenceStatus::Truncated),
    ] {
        let request = BlockEvidenceRequest::new(|_| true, None, events, bytes)
            .with_kind(BlockEvidenceKind::Blocks)
            .with_frame_limit(NonZeroUsize::new(2).unwrap());
        let (mut decoder, capture) =
            Decoder::new_with_block_evidence(settings(4), request).unwrap();
        let pictures = decode(&mut decoder, SUBPEL).unwrap();
        assert!(capture.is_finished());
        let first = capture.take().unwrap();
        let second = capture.take().unwrap();
        assert!(capture.take().is_none());

        assert_eq!(pictures.len(), 2);
        assert_eq!(first.status, BlockEvidenceStatus::Complete);
        assert_eq!(second.status, final_status);
        assert_eq!(first.records.len(), 1);
        assert_eq!(second.records.len(), final_count);
        let first_frame = first.frame.unwrap().provenance;
        let second_frame = second.frame.unwrap().provenance;
        assert_eq!(first_frame, pictures[0].provenance());
        assert_eq!(second_frame, pictures[1].provenance());
        assert_eq!(
            (first_frame.coded.user_data, first_frame.coded.offset.get()),
            (17, 26)
        );
        assert_eq!(
            (
                second_frame.coded.user_data,
                second_frame.coded.offset.get()
            ),
            (18, 3)
        );
        assert!(first.records.capacity() + second.records.capacity() <= bytes / record_size);
    }
}

#[test]
fn compound_motion_preserves_both_vectors_for_one_reference() {
    let (report, _) = capture(
        SAME_REFERENCE,
        1,
        BlockEvidenceRequest::new(
            |coded| coded.user_data == 18,
            Some(rect(0, 0, 1, 1)),
            2,
            usize::MAX,
        )
        .with_kind(BlockEvidenceKind::Blocks),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert_eq!(report.records.len(), 2);
    let block = report.records[0];
    let motion = report.records[1];
    assert_eq!(block.id.coded, rect(0, 0, 64, 16));
    assert_eq!(motion.id, block.id);
    assert!(matches!(
        block.data,
        BlockEvidenceData::Block(facts) if facts.qindex == 198
            && matches!(facts.prediction, BlockPrediction::Inter(pred)
                if pred.mode == BlockInterMode::Compound(CompoundMode::NewNew)
                    && pred.precision == Some(BlockMotionPrecision::QuarterPel)
                    && pred.references.iter().all(|reference| matches!(reference,
                        Some(BlockReference::Frame { list_index: 0, slot: 0, .. }))))
    ));
    assert!(matches!(
        motion.data,
        BlockEvidenceData::Motion(BlockMotion::Inter(motion))
            if motion.vectors == [
                Some(MotionVector { row: 0, col: 32 }),
                Some(MotionVector { row: 0, col: -128 }),
            ]
    ));
}

#[test]
fn intrabc_motion_reports_the_source_for_a_padded_block() {
    let (report, _) = capture(
        COPY,
        1,
        BlockEvidenceRequest::new(|_| true, Some(rect(0, 179, 1, 1)), 8, usize::MAX)
            .with_kind(BlockEvidenceKind::Blocks),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    let record = report
        .records
        .iter()
        .find(|record| {
            record.id.coded == rect(0, 160, 32, 32)
                && matches!(
                    record.data,
                    BlockEvidenceData::Motion(BlockMotion::IntraBlockCopy(_))
                )
        })
        .unwrap();
    assert_eq!(record.visible, Some(rect(0, 160, 32, 20)));
    let coded_luma_height = 184;
    assert!(
        matches!(
            record.data,
            BlockEvidenceData::Motion(BlockMotion::IntraBlockCopy(copy))
                if copy.vector == (MotionVector { row: -512, col: 0 })
                    && copy.source == Some(rect(0, 96, 32, coded_luma_height - 160))
        ),
        "{:?}",
        record.data
    );
    assert!(report.records.iter().any(|block| block.id == record.id
        && matches!(block.data, BlockEvidenceData::Block(facts)
            if facts.qindex == 51 && facts.prediction == BlockPrediction::IntraBlockCopy)));
}

#[test]
fn cancellation_between_frames_preserves_the_first_report_and_the_output() {
    let mut ordinary = Decoder::new(settings(1)).unwrap();
    let expected = picture_hashes(&decode(&mut ordinary, SUBPEL).unwrap());

    let stop = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&stop);
    let request = BlockEvidenceRequest::new(|_| true, None, 3, usize::MAX)
        .with_kind(BlockEvidenceKind::Blocks)
        .with_frame_limit(NonZeroUsize::new(2).unwrap())
        .with_cancellation(move || cancelled.load(Ordering::Relaxed));
    let (mut decoder, capture) = Decoder::new_with_block_evidence(settings(1), request).unwrap();
    let mut reader = TemporalUnitReader::new(SUBPEL);
    let key = reader.next_temporal_unit().unwrap().unwrap();
    let key_props = DataProps::default().with_user_data(17);
    assert_eq!(
        decoder.send_data(key.bytes, key_props).unwrap(),
        SendStatus::Accepted
    );
    let first = decoder.wait_picture().unwrap().unwrap();
    assert!(decoder.wait_picture().unwrap().is_none());

    assert!(!capture.is_finished());
    stop.store(true, Ordering::Relaxed);
    let inter = reader.next_temporal_unit().unwrap().unwrap();
    let inter_props = DataProps::default().with_user_data(18);
    assert_eq!(
        decoder.send_data(inter.bytes, inter_props).unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    let second = decoder.get_picture().unwrap().unwrap();
    assert!(decoder.get_picture().unwrap().is_none());
    assert_eq!(picture_hashes(&[first, second]), expected);

    assert!(capture.is_finished());
    let first = capture.take().unwrap();
    assert_eq!(first.status, BlockEvidenceStatus::Complete);
    assert_eq!(first.records.len(), 1);
    let second = capture.take().unwrap();
    assert_eq!(second.status, BlockEvidenceStatus::Cancelled);
    assert!(second.records.is_empty());
    assert!(capture.take().is_none());
}

#[test]
fn a_flush_ends_capture_before_the_next_session() {
    let (mut decoder, capture) = Decoder::new_with_block_evidence(
        settings(4),
        BlockEvidenceRequest::new(|coded| coded.user_data == 18, None, 8, usize::MAX),
    )
    .unwrap();
    let mut reader = TemporalUnitReader::new(SUBPEL);
    let unit = reader.next_temporal_unit().unwrap().unwrap();
    assert_eq!(
        decoder
            .send_data(unit.bytes, DataProps::default().with_user_data(17))
            .unwrap(),
        SendStatus::Accepted
    );
    assert!(!capture.is_finished());
    decoder.flush();
    assert!(capture.is_finished());
    let report = capture.take().unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::FrameNotFound);
    assert!(report.frame.is_none());
    assert!(report.records.is_empty());

    assert_eq!(decode(&mut decoder, SUBPEL).unwrap().len(), 2);
    assert!(capture.take().is_none());
}

#[test]
fn capture_distinguishes_absent_frames_and_frames_without_a_block_walk() {
    for (offset, status) in [
        (40, BlockEvidenceStatus::FrameNotFound),
        (41, BlockEvidenceStatus::Unavailable),
    ] {
        let (report, pictures) = capture(
            BRIDGE,
            4,
            BlockEvidenceRequest::new(
                move |coded| coded.offset.get() == offset,
                None,
                8,
                usize::MAX,
            ),
        );
        assert_eq!(pictures.len(), 2);
        assert_eq!(report.status, status);
        assert_eq!(
            report
                .frame
                .map(|frame| frame.provenance.coded.offset.get()),
            (status == BlockEvidenceStatus::Unavailable).then_some(41)
        );
        assert!(report.records.is_empty());
    }
}

#[test]
fn capture_uses_coded_locations_for_hidden_and_show_existing_frames() {
    let hidden = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-hidden-clk-ras-initial-tu-64x64.ivf"
    );
    let (report, pictures) = capture(
        hidden,
        4,
        BlockEvidenceRequest::new(|coded| coded.offset.get() == 27, None, 32, usize::MAX),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    assert!(!report.records.is_empty());
    let coded = report.frame.unwrap().provenance.coded;
    assert_eq!((coded.user_data, coded.offset.get()), (17, 27));
    assert_eq!(pictures.len(), 1);
    assert_ne!(pictures[0].provenance().coded, coded);

    let repeated =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-sef-families-64x64.ivf");
    let (report, _) = capture(
        repeated,
        4,
        BlockEvidenceRequest::new(
            |coded| coded.user_data == 18 && coded.offset.get() == 54,
            None,
            32,
            usize::MAX,
        ),
    );
    assert_eq!(report.status, BlockEvidenceStatus::FrameNotFound);
    assert!(report.records.is_empty());
}

#[test]
fn failed_decode_finishes_a_pending_capture() {
    let (mut decoder, capture) = Decoder::new_with_block_evidence(
        settings(1),
        BlockEvidenceRequest::new(|_| true, None, 8, usize::MAX),
    )
    .unwrap();
    assert_eq!(
        decoder
            .send_data(&[0x01, 0x08, 0x05, 0x10], DataProps::default())
            .unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    assert!(decoder.get_picture().is_err());
    assert!(capture.is_finished());
    let report = capture.take().unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::DecodeFailed);
    assert!(report.frame.is_none());
    assert!(report.records.is_empty());
}

#[test]
fn tile_decode_failure_preserves_the_selected_identity() {
    let mut reader = TemporalUnitReader::new(TILES);
    let unit = reader.next_temporal_unit().unwrap().unwrap();
    let obu_header_offset = 16;
    let first_tile_end = 28;
    let mut damaged = unit.bytes[..first_tile_end].to_vec();
    damaged[obu_header_offset - 1] = u8::try_from(first_tile_end - obu_header_offset).unwrap();
    let (mut decoder, capture) = Decoder::new_with_block_evidence(
        settings(1),
        BlockEvidenceRequest::new(|coded| coded.offset.get() == 16, None, 8, usize::MAX),
    )
    .unwrap();
    assert_eq!(
        decoder
            .send_data(&damaged, DataProps::default().with_user_data(17))
            .unwrap(),
        SendStatus::Accepted
    );
    decoder.drain();
    assert!(matches!(
        decoder.get_picture(),
        Err(DecodeError::MalformedSource { .. })
    ));
    let report = capture.take().unwrap();
    assert_eq!(report.status, BlockEvidenceStatus::DecodeFailed);
    let coded = report.frame.unwrap().provenance.coded;
    assert_eq!((coded.user_data, coded.offset.get()), (17, 16));
}
