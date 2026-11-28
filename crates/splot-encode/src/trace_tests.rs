// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::BitDepth;

use super::*;

fn request(max_events: usize) -> DecisionTraceRequest {
    DecisionTraceRequest::new(0..8, max_events, usize::MAX).with_kind(DecisionKind::PartitionTrials)
}

fn attempt(display_index: u64) -> FrameAttempt {
    FrameAttempt {
        plan: FramePlanInfo {
            frame_index: display_index,
            display_index,
            frame_type: FrameType::Key,
            layer: 0,
            complexity: 0,
            min_complexity: 0,
            qindex_offset: 0,
            bit_depth: BitDepth::Eight,
        },
        pts: display_index,
        attempt: 0,
        qindex: 80,
        bits: 64,
        buffer_room_bits: None,
        outcome: FrameOutcome::Committed(CommittedFrame {
            frame_type: FrameType::Key,
            packet: display_index,
            offset: 0,
            tip_output: false,
        }),
    }
}

/// A search trace of `events` superblock events under `cap`.
fn searched(cap: usize, events: u32) -> SearchTrace {
    let mut trace = SearchTrace {
        tile: 0,
        region: None,
        kinds: request(0).kinds,
        cap,
        events: Vec::new(),
        seen: 0,
        failed: false,
    };
    for qindex in 0..events {
        trace.store(DecisionEvent::Superblock(SuperblockSearch {
            block: BlockEvidenceId::new(
                0,
                BlockEvidenceTree::Shared,
                PlaneRect::new(0, 0, 64, 64).unwrap(),
            ),
            qindex,
            lambda_q16: 1,
            segment: None,
        }));
    }
    trace
}

/// Superblocks that finish out of order give the events of the raster-order
/// prefix, and a superblock behind a full prefix holds no event.
#[test]
fn the_collector_keeps_the_raster_order_prefix_of_the_budget() {
    let spec = Capture::new(request(5)).spec().unwrap();
    let collector = FrameCollector::new(Some(&spec), 0).unwrap();
    assert!(FrameCollector::new(Some(&spec), 8).is_none());
    for index in [3, 0, 2, 1] {
        collector.collect(index, searched(spec.cap, 3));
        let held: usize = (collector.superblocks.lock().unwrap().values())
            .map(|trace| trace.events.len())
            .sum();
        assert!(held <= 2 * spec.cap, "{held} events held");
    }
    let frame = collector.finish();
    assert_eq!((frame.seen, frame.failed), (12, false));
    let order: Vec<u32> = frame
        .events
        .iter()
        .map(|event| match event {
            DecisionEvent::Superblock(search) => search.qindex,
            _ => u32::MAX,
        })
        .collect();
    assert_eq!(order, [0, 1, 2, 0, 1]);
}

/// A capture that could not store the events of a frame stops with what it
/// has, and takes no later frame.
#[test]
fn a_failed_frame_trace_stops_the_capture() {
    let mut capture = Capture::new(request(100));
    capture.frame(Some(attempt(0)), Some(frame_of(searched(100, 2))));
    let spec = capture.spec().unwrap();
    let collector = FrameCollector::new(Some(&spec), 1).unwrap();
    collector.collect(0, searched(spec.cap, 1));
    let mut failed = searched(spec.cap, 2);
    failed.failed = true;
    collector.collect(1, failed);
    collector.collect(2, searched(spec.cap, 4));
    let blocks = collector.finish();
    assert_eq!((blocks.events.len(), blocks.seen), (1, 7));
    capture.frame(Some(attempt(1)), Some(blocks));
    assert!(capture.spec().is_none());
    capture.frame(Some(attempt(2)), None);
    let trace = capture.finish(None);
    let stop = DecisionTraceStatus::Partial(DecisionTraceStop::CaptureFailed);
    assert_eq!((trace.status, trace.events.len()), (stop, 5));
    assert_eq!(trace.dropped_events, 6);

    let mut capture = Capture::new(request(100));
    capture.frame(None, None);
    let stop = DecisionTraceStatus::Dropped(DecisionTraceStop::CaptureFailed);
    assert_eq!(capture.finish(None).status, stop);

    let wide = PlaneRect::new(1, 1, usize::MAX, usize::MAX).unwrap();
    assert!(overlaps(wide, PlaneRect::new(8, 8, 8, 8).unwrap()));
    assert!(!overlaps(wide, PlaneRect::new(0, 8, 1, 8).unwrap()));
}

/// A frame outside the request leaves no event and does not count.
#[test]
fn a_capture_ignores_frames_outside_its_request() {
    let mut capture = Capture::new(request(100));
    capture.frame(Some(attempt(8)), None);
    let trace = capture.finish(Some(DecisionTraceStop::Unfinished));
    let stop = DecisionTraceStatus::Dropped(DecisionTraceStop::Unfinished);
    assert_eq!((trace.status, trace.dropped_events), (stop, 0));
}

fn frame_of(trace: SearchTrace) -> FrameTrace {
    FrameTrace {
        seen: trace.seen,
        failed: trace.failed,
        events: trace.events,
    }
}
