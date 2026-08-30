// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for completion-backed frame handles and the in-flight ring.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

use crate::test_support::decoded_frame;

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).unwrap()
}

fn pending_entry(
    ring: &mut InflightRing,
    frame_index: usize,
) -> (FrameSlotWriter<u8>, FinishReportWriter) {
    let frame = decoded_frame(4, 4);
    let (slot, writer) = RefFrameSlot::pending(frame.info()).expect("pending slot");
    let report = Arc::new(CompletionCell::new());
    ring.push(InflightEntry {
        frame_index,
        slot: PipelineFrameSlot::Eight(slot),
        report: Arc::clone(&report),
    });
    (
        writer,
        FinishReportWriter {
            cell: report,
            outcome: FinishOutcome::default(),
        },
    )
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
fn a_failed_writer_closes_the_published_prefix_instead_of_lending_it() {
    let (slot, writer) =
        RefFrameSlot::<u8>::pending(decoded_frame(8, 8).info()).expect("pending slot");
    let progress = slot.progress().expect("a pending slot publishes stripes");
    assert!(progress.begin(&[(0, 4), (4, 8)]));
    progress.publish(0);
    assert_eq!(slot.published_luma_rows(), 4);
    assert!(
        slot.hold_samples().is_some(),
        "a live filter phase lends the prefix it published"
    );

    drop(writer);

    assert!(slot.is_settled());
    assert_eq!(
        slot.published_luma_rows(),
        0,
        "a failed phase publishes no readable row"
    );
    assert!(
        slot.hold_samples().is_none(),
        "a reader of a failed frame must get nothing to read, not its unfiltered workspace"
    );
    assert!(slot.ready().is_err(), "and the slot reports the failure");
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

    let (first, first_report) = pending_entry(&mut ring, 0);
    let (second, second_report) = pending_entry(&mut ring, 1);
    let (third, third_report) = pending_entry(&mut ring, 2);
    first.complete(SharedFrame::new(decoded_frame(4, 4)));
    second.complete(SharedFrame::new(decoded_frame(4, 4)));
    third.complete(SharedFrame::new(decoded_frame(4, 4)));
    drop((first_report, second_report, third_report));
    assert_eq!(ring.entries.len(), 3);

    ring.reserve(&mut eight, &mut ten);

    assert_eq!(ring.entries.len(), 2);
    assert_eq!(ring.entries.front().map(|entry| entry.frame_index), Some(1));

    ring.harvest_all(&mut eight, &mut ten);

    assert!(ring.entries.is_empty());
}

#[test]
fn a_depth_of_two_walks_one_frame_beside_one_uncollected_finish() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(2));

    let (first, first_report) = pending_entry(&mut ring, 0);
    first.complete(SharedFrame::new(decoded_frame(4, 4)));
    drop(first_report);
    ring.reserve(&mut eight, &mut ten);

    assert!(
        ring.holds(0),
        "admitting frame 1 must not harvest frame 0 at depth two"
    );

    let (second, second_report) = pending_entry(&mut ring, 1);
    second.complete(SharedFrame::new(decoded_frame(4, 4)));
    drop(second_report);
    assert_eq!(ring.entries.len(), 2);

    ring.reserve(&mut eight, &mut ten);

    assert!(!ring.holds(0), "frame 0 must be harvested to admit frame 2");
    assert!(ring.holds(1));
    assert_eq!(ring.entries.len(), 1);

    ring.harvest_all(&mut eight, &mut ten);

    assert!(!ring.holds(1));
}

#[test]
fn a_depth_of_one_never_keeps_a_frame_in_flight() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(NonZeroUsize::MIN);

    ring.reserve(&mut eight, &mut ten);

    assert!(ring.entries.is_empty());
    assert!(ring.take_failure().is_none());
}

#[test]
fn the_lowest_indexed_filter_failure_outranks_later_ones() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(4));

    for (frame_index, reason) in [(2usize, "later_failure"), (1usize, "earlier_failure")] {
        let (writer, mut report) = pending_entry(&mut ring, frame_index);
        report.outcome.error = Some(unsupported(reason, None, "test filter phase failure"));
        drop(writer);
        drop(report);
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

    let (writer, mut report) = pending_entry(&mut ring, 0);
    let mut records = FrameFilterRecords::default();
    records.deblock_blocks.reserve(64);
    report.outcome.records = Some(records);
    writer.complete(SharedFrame::new(decoded_frame(4, 4)));
    assert!(
        ring.entries
            .front()
            .is_some_and(|entry| entry.slot.is_settled())
    );
    assert!(
        ring.entries
            .front()
            .is_some_and(|entry| !entry.report.is_set())
    );
    drop(report);
    assert!(
        ring.entries
            .front()
            .is_some_and(|entry| entry.report.is_set())
    );

    ring.harvest_all(&mut eight, &mut ten);

    assert!(eight.frame_filter_records_capacity() >= 64);
    assert_eq!(ten.frame_filter_records_capacity(), 0);
}

#[test]
fn failed_finish_reports_its_error_before_harvest() {
    let mut eight = InterDecodeScratch::<u8>::default();
    let mut ten = InterDecodeScratch::<u16>::default();
    let mut ring = InflightRing::new(nz(2));
    let frame = decoded_frame(4, 4);
    let (_slot, finish) =
        reserve_pending_slot(frame.info(), PipelineFrameSlot::Eight, &mut ring, 0)
            .expect("pending finish");

    finish.fail(unsupported(
        "reported_finish_failure",
        None,
        "test filter phase failure",
    ));
    assert!(
        ring.entries
            .front()
            .is_some_and(|entry| entry.report.is_set())
    );

    ring.harvest_all(&mut eight, &mut ten);

    let failure = ring.take_failure().expect("reported failure");
    assert!(format!("{failure:?}").contains("reported_finish_failure"));
}
