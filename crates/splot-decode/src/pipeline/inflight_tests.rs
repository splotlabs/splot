// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for completion-backed frame handles and the in-flight ring.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

use splot_recon::{
    BitDepth, DecodedFrame, FramePlanes, OutputIndex, PixelFormat, Plane, PlaneRect, PlaneSize,
};

fn decoded_frame(width: usize, height: usize) -> DecodedFrame<u8> {
    let size = PlaneSize::new(width, height).unwrap();
    let rect = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Monochrome,
        size,
        rect,
    )
    .unwrap();
    let y = Plane::from_vec(size, width, rect, vec![0; width * height]).unwrap();
    DecodedFrame::try_new(info, FramePlanes::new(y, None, None)).unwrap()
}

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}

fn pending_entry(
    ring: &mut InflightRing,
    frame_index: usize,
) -> (FrameSlotWriter<u8>, Arc<Mutex<FinishOutcome>>) {
    let frame = decoded_frame(4, 4);
    let (slot, writer) = RefFrameSlot::pending(frame.info()).expect("pending slot");
    let outcome = Arc::new(Mutex::new(FinishOutcome::default()));
    ring.push(InflightEntry {
        frame_index,
        slot: PipelineFrameSlot::Eight(slot),
        outcome: Arc::clone(&outcome),
    });
    (writer, outcome)
}

#[test]
fn pending_slot_publishes_samples_to_try_frozen_and_wait_ready() {
    let frame = decoded_frame(8, 4);
    let info = frame.info();
    let (slot, writer) = RefFrameSlot::pending(info).expect("pending slot");

    assert!(!slot.is_settled());
    assert!(slot.try_frozen().is_none());
    assert!(slot.ready().is_err());
    assert_eq!(slot.info(), info);

    writer.complete(SharedFrame::new(frame));

    assert!(slot.is_settled());
    assert_eq!(slot.try_frozen().map(DecodedFrame::info), Some(info));
    assert_eq!(slot.wait_ready().unwrap().get().info(), info);
    assert!(slot.wait_settled().is_ok());
}

#[test]
fn failed_slot_reports_a_typed_error_to_every_reader() {
    let (slot, writer) =
        RefFrameSlot::<u8>::pending(decoded_frame(4, 4).info()).expect("pending slot");

    drop(writer);

    assert!(slot.is_settled());
    assert!(slot.try_frozen().is_none());
    assert!(slot.ready().is_err());
    assert!(slot.wait_ready().is_err());
    assert!(slot.wait_settled().is_err());
}

#[test]
fn dropping_the_writer_settles_the_slot_as_failed() {
    let (slot, writer) =
        RefFrameSlot::<u8>::pending(decoded_frame(4, 4).info()).expect("pending slot");

    drop(writer);

    assert!(slot.is_settled());
    assert!(slot.wait_settled().is_err());
}

#[test]
fn a_completed_writer_leaves_the_published_samples_in_place() {
    let frame = decoded_frame(4, 4);
    let info = frame.info();
    let (slot, writer) = RefFrameSlot::pending(info).expect("pending slot");

    writer.complete(SharedFrame::new(frame));

    assert_eq!(slot.try_frozen().map(DecodedFrame::info), Some(info));
    assert_eq!(
        slot.share().try_frozen().map(DecodedFrame::info),
        Some(info)
    );
}

#[test]
fn pending_slot_geometry_matches_the_published_frame() {
    let frame = decoded_frame(12, 8);
    let info = frame.info();
    let (slot, writer) = RefFrameSlot::pending(info).expect("pending slot");

    writer.complete(SharedFrame::new(frame));

    assert_eq!(slot.info(), slot.try_frozen().unwrap().info());
}

#[test]
fn ring_admission_harvests_the_oldest_entry_first() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(3));

    let (first, _first_outcome) = pending_entry(&mut ring, 0);
    let (second, _second_outcome) = pending_entry(&mut ring, 1);
    let (third, _third_outcome) = pending_entry(&mut ring, 2);
    first.complete(SharedFrame::new(decoded_frame(4, 4)));
    second.complete(SharedFrame::new(decoded_frame(4, 4)));
    third.complete(SharedFrame::new(decoded_frame(4, 4)));
    assert_eq!(ring.max_in_flight(), 3);

    ring.reserve(&mut eight, &mut ten);

    assert_eq!(ring.entries.len(), 1);
    assert_eq!(ring.entries.front().map(|entry| entry.frame_index), Some(2));

    ring.harvest_all(&mut eight, &mut ten);

    assert!(ring.entries.is_empty());
}

#[test]
fn a_depth_of_one_never_keeps_a_frame_in_flight() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(NonZeroUsize::MIN);

    ring.reserve(&mut eight, &mut ten);

    assert_eq!(ring.max_in_flight(), 0);
    assert!(ring.take_failure().is_none());
}

#[test]
fn the_lowest_indexed_filter_failure_outranks_later_ones() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(4));

    for (frame_index, reason) in [(2usize, "later_failure"), (1usize, "earlier_failure")] {
        let (writer, outcome) = pending_entry(&mut ring, frame_index);
        outcome.lock().unwrap_or_else(PoisonError::into_inner).error =
            Some(unsupported(reason, None, "test filter phase failure"));
        drop(writer);
    }

    ring.harvest_all(&mut eight, &mut ten);

    let failure = ring.take_failure().expect("a collected failure");
    assert!(
        format!("{failure:?}").contains("earlier_failure"),
        "expected the lowest-indexed failure, got {failure:?}"
    );
    assert!(ring.take_failure().is_none());
}

#[test]
fn harvesting_recycles_filter_records_into_the_matching_scratch() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(2));

    let (writer, outcome) = pending_entry(&mut ring, 0);
    let mut records = FrameFilterRecords::default();
    records.deblock_blocks.reserve(64);
    outcome
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .records = Some(records);
    writer.complete(SharedFrame::new(decoded_frame(4, 4)));

    ring.harvest_all(&mut eight, &mut ten);

    assert!(eight.frame_filter_records_capacity() >= 64);
    assert_eq!(ten.frame_filter_records_capacity(), 0);
}
