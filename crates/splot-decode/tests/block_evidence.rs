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
//! Read stage planes in `av2_decode_tg_tiles_and_wrapup` before deblock and
//! after each filter. Read `predicted_pixels` with inspection enabled.
//! For clipped residuals, record the signed input to `highbd_clip_pixel_add`
//! in the C inverse transform. Compare the resulting output with ordinary AVM.
//!
//! For entropy checks, record the value and descending Q15 cutoffs from
//! `avm_read_symbol` before adaptation in `av2_read_coeffs_txb`. Omit the terminal zero.
//!
//! [AVM]: https://github.com/AOMediaCodec/avm/tree/a1ec4594d97028ebc34083ec8346616d697c2fc9

#![expect(clippy::unwrap_used)]

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use splot_core::stream_reader::TemporalUnitReader;
use splot_decode::block_evidence::{
    BlockEvidenceData, BlockEvidenceKind, BlockEvidenceRecord, BlockEvidenceReport,
    BlockEvidenceRequest, BlockEvidenceStatus, BlockEvidenceTree, BlockInterMode,
    BlockLumaPrediction, BlockMotion, BlockMotionPrecision, BlockPartition, BlockPrediction,
    BlockReference, BlockTransformId, CompoundMode, InterMode, IntraMode, MotionVector,
};
use splot_decode::{
    DataProps, DecodeError, Decoder, DecoderSettings, FrameDelay, Picture, PictureFrame,
    SendStatus, ThreadCount,
};
use splot_recon::{DecodedFrameHashInput, PixelFormat, PlaneId, PlaneRect};

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
const FILTERS: &[u8] = include_bytes!(
    "../../../tests/conformance/vectors/valid/syn-3frame-padded-filters-422-134x69-10bit-q150.ivf"
);

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

#[test]
fn stage_samples_match_avm_and_preserve_output() {
    use splot_decode::block_evidence::{DecodeStage, StageEvidenceStatus};
    use splot_recon::PlaneId;

    const EIGHT: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-2sb-deblock-intra-128x64-q100.ivf"
    );
    const TEN: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-3frame-padded-filters-422-134x69-10bit-q150.ivf"
    );
    const GDF: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-gdf-xtile-lr-512x256-q190.ivf"
    );
    use DecodeStage::{
        Ccso, Cdef, Deblock, Gdf, Prediction, Reconstruction, Reference, Residual, Restoration,
    };
    let eight: &[(DecodeStage, i32)] = &[
        (Prediction, 128),
        (Residual, -54),
        (Reconstruction, 74),
        (Deblock, 73),
        (Reference, 73),
    ];
    let ten: &[(DecodeStage, i32)] = &[
        (Prediction, 375),
        (Residual, 622),
        (Reconstruction, 997),
        (Deblock, 935),
        (Cdef, 916),
        (Ccso, 895),
        (Restoration, 901),
        (Reference, 901),
    ];

    let eight_clip: &[(DecodeStage, i32)] =
        &[(Prediction, 225), (Residual, 33), (Reconstruction, 255)];
    let ten_clip: &[(DecodeStage, i32)] =
        &[(Prediction, 882), (Residual, 156), (Reconstruction, 1023)];
    let gdf: &[(DecodeStage, i32)] = &[
        (Deblock, 248),
        (Cdef, 248),
        (Ccso, 248),
        (Restoration, 248),
        (Gdf, 249),
        (Reference, 249),
    ];

    let gdf_skipped: &[DecodeStage] = &[Ccso, Restoration];
    for (fixture, index, region, golden, skipped) in [
        (EIGHT, 0, rect(4, 31, 1, 1), eight, &[][..]),
        (TEN, 1, rect(127, 50, 1, 1), ten, &[][..]),
        (GDF, 0, rect(145, 42, 1, 1), eight_clip, &[][..]),
        (GDF, 0, rect(503, 42, 1, 1), gdf, gdf_skipped),
        (TEN, 1, rect(31, 20, 1, 1), ten_clip, &[][..]),
    ] {
        for threads in [1, 4] {
            let mut normal = Decoder::new(settings(threads)).unwrap();
            let original = decode(&mut normal, fixture).unwrap();
            let coded = original[index].provenance().coded;
            let (report, inspected) = capture(
                fixture,
                threads,
                BlockEvidenceRequest::new(move |location| location == coded, None, 0, 4096)
                    .with_stage_samples(PlaneId::Y, region),
            );
            assert_eq!(picture_hashes(&original), picture_hashes(&inspected));
            assert_eq!(report.status, BlockEvidenceStatus::Complete);
            for &(stage, sample) in golden {
                let actual = report
                    .stages
                    .iter()
                    .find(|record| record.stage == stage)
                    .unwrap();
                let expected = if skipped.contains(&stage) {
                    StageEvidenceStatus::Skipped
                } else {
                    StageEvidenceStatus::Applied
                };
                assert_eq!(actual.status, expected, "{stage:?} at {region:?}");
                assert_eq!(actual.region, Some(region), "{stage:?}");
                assert_eq!(actual.samples, [sample], "{stage:?}");
            }
        }
    }
}

#[test]
fn stage_budget_and_cancellation_preserve_normal_output() {
    use splot_decode::block_evidence::StageEvidenceStatus;
    use splot_recon::PlaneId;

    let mut normal = Decoder::new(settings(4)).unwrap();
    let base = picture_hashes(&decode(&mut normal, TILES).unwrap());
    for status in [
        BlockEvidenceStatus::Truncated,
        BlockEvidenceStatus::Cancelled,
    ] {
        let request = BlockEvidenceRequest::new(|_| true, None, 0, 0)
            .with_stage_samples(PlaneId::Y, rect(0, 0, 12, 12))
            .with_cancellation(move || status == BlockEvidenceStatus::Cancelled);
        let (report, pictures) = capture(TILES, 4, request);
        assert_eq!(report.status, status);
        assert_eq!(base, picture_hashes(&pictures));
        assert!(report.stages.iter().all(|stage| {
            stage.status == StageEvidenceStatus::Uncaptured
                && stage.samples.is_empty()
                && stage.region.is_none()
        }));
    }
}

#[test]
fn film_grain_presentation_keeps_the_reference_values() {
    use splot_decode::PictureSamples;
    use splot_decode::block_evidence::DecodeStage;
    use splot_recon::PlaneId;

    const GRAIN: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-3frame-filmgrain-inter-64x64-10bit.ivf"
    );
    let request = BlockEvidenceRequest::new(|_| true, None, 0, 4096)
        .with_stage_samples(PlaneId::U, rect(17, 0, 1, 1));
    let (mut decoder, capture) =
        Decoder::new_with_block_evidence(settings(4).with_skip_film_grain(true), request).unwrap();
    let decoded = decode(&mut decoder, GRAIN).unwrap();
    let reference = &decoded[0];
    assert!(reference.has_film_grain());
    let report = capture.take().unwrap();
    assert_eq!(report.frame.unwrap().provenance, reference.provenance());
    let actual = report
        .stages
        .iter()
        .find(|stage| stage.stage == DecodeStage::Reference)
        .unwrap();
    assert_eq!(actual.samples, [961]);

    let sample = |picture: &Picture| match picture.plane(PlaneId::U).unwrap() {
        PictureSamples::Ten(plane) => plane.visible_rows().next().unwrap()[17],
        PictureSamples::Eight(_) => panic!("expected 10-bit film-grain fixture"),
    };
    let presentation = reference.apply_film_grain().unwrap();
    assert_eq!(reference.provenance(), presentation.provenance());
    assert_eq!(sample(&presentation), 940);
    assert_eq!(sample(reference), 961);
}

#[test]
fn monochrome_frame_cannot_supply_chroma_stages() {
    use splot_recon::PlaneId;

    const MONO: &[u8] = include_bytes!(
        "../../../tests/conformance/vectors/valid/syn-4frame-mono-inter-256x128.ivf"
    );
    let (report, _) = capture(
        MONO,
        1,
        BlockEvidenceRequest::new(|_| true, None, 0, 4096)
            .with_stage_samples(PlaneId::V, rect(0, 0, 1, 1)),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Unavailable);
    assert!(report.frame.is_some());
    assert!(report.stages.is_empty());
}

fn selected_transform(
    fixture: &[u8],
    tile: usize,
    plane: PlaneId,
    coded: PlaneRect,
) -> BlockTransformId {
    let (geometry, _) = capture(
        fixture,
        1,
        BlockEvidenceRequest::new(|_| true, None, 100_000, usize::MAX)
            .with_kind(BlockEvidenceKind::Transforms),
    );
    geometry
        .records
        .iter()
        .find_map(|record| {
            let BlockEvidenceData::Transform(transform) = record.data else {
                return None;
            };
            (transform.id.block.tile == tile
                && transform.id.plane == plane
                && transform.id.coded == coded)
                .then_some(transform.id)
        })
        .unwrap()
}

#[test]
fn entropy_transform_prefix_matches_avm_cdf_rows_and_bypass_values() {
    use splot_decode::block_evidence::{BlockEntropyScope, EntropyCoding};

    struct Case {
        fixture: &'static [u8],
        tile: usize,
        coded: PlaneRect,
        skip_context: u16,
        cdf: &'static [(usize, u32, &'static [u16])],
        bypass: &'static [(usize, u32, u8)],
    }
    let cases = [
        Case {
            fixture: TILES,
            tile: 0,
            coded: rect(0, 0, 64, 64),
            skip_context: 0,
            cdf: &[
                (0, 0, &[85]),
                (1, 0, &[26930, 23847, 18314, 11606, 4880, 493, 344]),
                (2, 4, &[4545, 1120, 457, 241]),
                (3, 3, &[18803, 11789, 6250]),
                (4, 0, &[12660]),
            ],
            bypass: &[],
        },
        Case {
            fixture: TILES,
            tile: 1,
            coded: rect(64, 0, 64, 32),
            skip_context: 0,
            cdf: &[
                (0, 0, &[85]),
                (1, 4, &[26930, 23847, 18314, 11606, 4880, 493, 344]),
                (2, 0, &[12404]),
                (4, 0, &[584, 73, 24, 20]),
            ],
            bypass: &[(3, 1, 2)],
        },
        Case {
            fixture: FILTERS,
            tile: 0,
            coded: rect(0, 0, 32, 32),
            skip_context: 1,
            cdf: &[
                (0, 0, &[32164]),
                (1, 7, &[32134, 30991, 28895, 26475, 20115, 8943, 8056]),
                (3, 1, &[14442]),
                (5, 0, &[477, 20]),
                (9, 1, &[4677, 126, 20]),
            ],
            bypass: &[(2, 3, 2), (4, 255, 8)],
        },
    ];
    for case in cases {
        let transform = selected_transform(case.fixture, case.tile, PlaneId::Y, case.coded);
        let scope = BlockEntropyScope::Transform(transform);
        let (report, pictures) = capture(
            case.fixture,
            4,
            BlockEvidenceRequest::new(|_| true, None, 100_000, usize::MAX).with_entropy(scope),
        );
        let mut ordinary = Decoder::new(settings(1)).unwrap();
        assert_eq!(
            picture_hashes(&pictures),
            picture_hashes(&decode(&mut ordinary, case.fixture).unwrap())
        );
        assert_eq!(report.status, BlockEvidenceStatus::Complete, "{scope:?}");
        assert!(report.records.is_empty());
        let evidence = report.entropy.unwrap();
        assert_eq!(evidence.scope, scope);
        assert!(evidence.tile_payload_bytes > 0);
        let mut ordinal = 0;
        for (index, symbol) in evidence.symbols.iter().enumerate() {
            assert_eq!(evidence.transform_for(index), Some(transform));
            assert_eq!(symbol.ordinal, ordinal);
            assert!(symbol.estimated_bits().unwrap().is_finite());
            ordinal += u64::from(symbol.bypass_bits.max(1));
        }
        assert_eq!(ordinal, evidence.end_ordinal);
        let skip = evidence.symbols[0].syntax.unwrap();
        assert_eq!(skip.key, "all_zero");
        assert_eq!(skip.context, "plane_type,tx_size_ctx,txb_skip_ctx");
        assert_eq!(skip.values[2], case.skip_context);
        let eob = evidence.symbols[1].syntax.unwrap();
        assert_eq!(eob.key, "eob_pt");
        assert_eq!(eob.context, "eob_multi_size,eob_ctx");
        assert_eq!(eob.values[..2], [6, 0]);
        for &(index, value, cutoffs) in case.cdf {
            let symbol = evidence.symbols[index];
            assert_eq!(symbol.coding, EntropyCoding::Cdf);
            assert_eq!(symbol.value, value);
            let cdf = symbol.cdf.unwrap();
            assert_eq!(usize::from(cdf.arity), cutoffs.len() + 1);
            assert_eq!(cdf.adaptation_count, 0);
            for (&actual, &expected) in cdf.cutoffs.iter().zip(cutoffs) {
                assert_eq!(actual, 32768 - expected);
            }
        }
        for &(index, value, bits) in case.bypass {
            let symbol = evidence.symbols[index];
            assert_eq!(symbol.coding, EntropyCoding::Bypass);
            assert_eq!(symbol.value, value);
            assert_eq!(symbol.bypass_bits, bits);
            assert_eq!(symbol.estimated_bits(), Some(f64::from(bits)));
        }
    }
}

#[test]
fn block_entropy_attributes_only_transform_reads_to_the_native_plane() {
    use splot_decode::block_evidence::{BlockEntropyScope, BlockEvidenceId};

    let block = BlockEvidenceId::new(1, BlockEvidenceTree::Shared, rect(64, 0, 64, 32));
    let (report, _) = capture(
        TILES,
        4,
        BlockEvidenceRequest::new(|_| true, None, 1000, usize::MAX)
            .with_entropy(BlockEntropyScope::Block(block)),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Complete);
    let entropy = report.entropy.unwrap();
    assert_eq!(entropy.transform_for(0), None);
    assert_eq!(entropy.transform_for(entropy.symbols.len()), None);
    let owners: std::collections::HashSet<_> = (0..entropy.symbols.len())
        .filter_map(|index| entropy.transform_for(index))
        .collect();
    assert_eq!(
        owners,
        std::collections::HashSet::from([
            BlockTransformId::new(block, PlaneId::Y, rect(64, 0, 64, 32)),
            BlockTransformId::new(block, PlaneId::U, rect(32, 0, 32, 16)),
            BlockTransformId::new(block, PlaneId::V, rect(32, 0, 32, 16)),
        ])
    );
    for (index, symbol) in entropy.symbols.iter().enumerate() {
        if symbol.syntax.is_some_and(|syntax| syntax.key == "all_zero") {
            assert!(entropy.transform_for(index).is_some());
        }
    }
}

#[test]
fn entropy_capture_respects_record_byte_and_cancellation_limits() {
    use splot_decode::block_evidence::{BlockEntropyScope, BlockEvidenceId, EntropySymbol};

    let block = BlockEvidenceId::new(0, BlockEvidenceTree::Shared, rect(0, 0, 64, 64));
    let scope = BlockEntropyScope::Block(block);
    let mut ordinary = Decoder::new(settings(1)).unwrap();
    let expected = picture_hashes(&decode(&mut ordinary, TILES).unwrap());

    for (max_events, max_bytes) in [
        (2, usize::MAX),
        (usize::MAX, 3 * size_of::<EntropySymbol>() - 1),
    ] {
        let (report, pictures) = capture(
            TILES,
            1,
            BlockEvidenceRequest::new(|_| true, None, max_events, max_bytes).with_entropy(scope),
        );
        assert_eq!(report.status, BlockEvidenceStatus::Truncated);
        let evidence = report.entropy.unwrap();
        assert_eq!(evidence.symbols.len(), 2);
        assert!(evidence.symbols.capacity() <= 2);
        assert!(evidence.end_ordinal > evidence.symbols[1].ordinal);
        assert_eq!(picture_hashes(&pictures), expected);
    }

    let polls = AtomicUsize::new(0);
    let (report, pictures) = capture(
        TILES,
        1,
        BlockEvidenceRequest::new(|_| true, None, 100, usize::MAX)
            .with_entropy(scope)
            .with_cancellation(move || polls.fetch_add(1, Ordering::Relaxed) >= 2),
    );
    assert_eq!(report.status, BlockEvidenceStatus::Cancelled);
    let evidence = report.entropy.unwrap();
    assert_eq!(evidence.symbols.len(), 2);
    assert!(evidence.end_ordinal > evidence.symbols[1].ordinal);
    assert_eq!(picture_hashes(&pictures), expected);
}

#[test]
fn entropy_start_ordinal_pages_a_scope_without_changing_records_or_output() {
    use splot_decode::block_evidence::{BlockEntropyEvidence, BlockEntropyScope};

    let owners = |evidence: &BlockEntropyEvidence| -> Vec<_> {
        (0..evidence.symbols.len())
            .map(|index| evidence.transform_for(index))
            .collect()
    };
    let mut split_reads = 0;
    for (fixture, coded) in [(TILES, rect(0, 0, 64, 64)), (FILTERS, rect(0, 0, 32, 32))] {
        let transform = selected_transform(fixture, 0, PlaneId::Y, coded);
        let mut ordinary = Decoder::new(settings(1)).unwrap();
        let expected = picture_hashes(&decode(&mut ordinary, fixture).unwrap());
        for scope in [
            BlockEntropyScope::Block(transform.block),
            BlockEntropyScope::Transform(transform),
        ] {
            let part = |start: u64, max_events: usize| {
                let (report, pictures) = capture(
                    fixture,
                    4,
                    BlockEvidenceRequest::new(|_| true, None, max_events, usize::MAX)
                        .with_entropy(scope)
                        .with_entropy_start(start),
                );
                assert_eq!(picture_hashes(&pictures), expected);
                (report.status, report.entropy.unwrap())
            };
            let (status, full) = part(0, 100_000);
            assert_eq!(status, BlockEvidenceStatus::Complete);
            let page_len = full.symbols.len().div_ceil(3);
            assert!(page_len > 1, "{scope:?}");
            let full_owners = owners(&full);

            let middle = full.symbols.len() / 2;
            let split = full.symbols.iter().position(|read| read.bypass_bits > 1);
            split_reads += usize::from(split.is_some());
            for (start, first) in [(full.symbols[middle].ordinal, middle)]
                .into_iter()
                .chain(split.map(|index| (full.symbols[index].ordinal + 1, index + 1)))
            {
                let (status, tail) = part(start, 100_000);
                assert_eq!(status, BlockEvidenceStatus::Complete);
                assert_eq!(tail.symbols, full.symbols[first..]);
                assert_eq!(owners(&tail), full_owners[first..]);
                assert_eq!(tail.end_ordinal, full.end_ordinal);
            }

            let (mut start, mut index, mut truncated_middle) = (0, 0, false);
            loop {
                let (status, page) = part(start, page_len);
                let end = index + page.symbols.len();
                assert_eq!(page.symbols, full.symbols[index..end]);
                assert_eq!(owners(&page), full_owners[index..end]);
                assert_eq!(page.end_ordinal, full.end_ordinal);
                index = end;
                if status == BlockEvidenceStatus::Complete {
                    break;
                }
                assert_eq!(status, BlockEvidenceStatus::Truncated);
                truncated_middle |= start > 0;
                let last = page.symbols[page_len - 1];
                start = last.ordinal + u64::from(last.bypass_bits.max(1));
            }
            assert_eq!(index, full.symbols.len());
            assert!(truncated_middle, "{scope:?}");

            for start in [full.end_ordinal, u64::MAX] {
                let (status, empty) = part(start, page_len);
                assert_eq!(status, BlockEvidenceStatus::Complete);
                assert!(empty.symbols.is_empty());
                assert_eq!(empty.end_ordinal, full.end_ordinal);
            }
        }
    }
    assert!(split_reads > 0);
}

#[test]
fn entropy_range_keeps_reports_bound_to_their_coded_frames() {
    use splot_decode::block_evidence::{BlockEntropyScope, BlockEvidenceId};

    let scope = BlockEntropyScope::Block(BlockEvidenceId::new(
        0,
        BlockEvidenceTree::Shared,
        rect(0, 0, 64, 64),
    ));
    let request = BlockEvidenceRequest::new(|_| true, None, 100_000, usize::MAX)
        .with_entropy(scope)
        .with_frame_limit(NonZeroUsize::new(2).unwrap());
    let (mut decoder, capture) = Decoder::new_with_block_evidence(settings(4), request).unwrap();
    let pictures = decode(&mut decoder, SUBPEL).unwrap();

    let first = capture.take().unwrap();
    assert_eq!(first.frame.unwrap().provenance, pictures[0].provenance());
    assert_eq!(first.status, BlockEvidenceStatus::Complete);
    let first_entropy = first.entropy.unwrap();
    assert_eq!(first_entropy.scope, scope);
    let second = capture.take().unwrap();
    assert_eq!(second.frame.unwrap().provenance, pictures[1].provenance());
    assert_eq!(second.status, BlockEvidenceStatus::Complete);
    let second_entropy = second.entropy.unwrap();
    assert_eq!(second_entropy.scope, scope);
    assert_eq!(first_entropy.symbols[0].ordinal, 0);
    assert_eq!(second_entropy.symbols[0].ordinal, 0);
    assert_ne!(first_entropy.symbols, second_entropy.symbols);
    assert!(capture.is_finished());
}

#[test]
fn entropy_keeps_the_chroma_pair_type_unassigned_to_either_transform() {
    use splot_decode::block_evidence::BlockEntropyScope;

    const CCTX: &[u8] =
        include_bytes!("../../../tests/conformance/vectors/valid/syn-cctx-444-intra-128x128.ivf");
    let transform = selected_transform(CCTX, 0, PlaneId::U, rect(12, 4, 4, 4));
    for scope in [
        BlockEntropyScope::Block(transform.block),
        BlockEntropyScope::Transform(transform),
    ] {
        let (report, _) = capture(
            CCTX,
            1,
            BlockEvidenceRequest::new(|_| true, None, 100_000, usize::MAX).with_entropy(scope),
        );
        assert_eq!(report.status, BlockEvidenceStatus::Complete);
        let evidence = report.entropy.unwrap();
        assert!(evidence.symbols.iter().enumerate().any(|(index, symbol)| {
            symbol.value == 5
                && symbol
                    .syntax
                    .is_some_and(|syntax| syntax.key == "cctx_type")
                && evidence.transform_for(index).is_none()
        }));
        assert!(
            (0..evidence.symbols.len())
                .any(|index| evidence.transform_for(index) == Some(transform))
        );
    }
}
