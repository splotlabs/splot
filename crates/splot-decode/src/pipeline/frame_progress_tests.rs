// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Watermark and published-prefix tests for [`super::FrameProgress`].

#![allow(clippy::expect_used)]

use splot_recon::{BitDepth, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize};

use super::{FROZEN, FrameProgress, PublishedFrame};
use crate::pipeline::inflight::{FrameSlotWriter, RefFrameSlot};
use crate::prediction::inter::reference::HeldFrameSamples;
use splot_recon::PlaneId;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

fn info(width: usize, height: usize, format: PixelFormat) -> DecodedFrameInfo {
    DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        format,
        PlaneSize::new(width, height).expect("frame size"),
        PlaneRect::new(0, 0, width, height).expect("visible rect"),
    )
    .expect("frame info")
}

fn new_progress(width: usize, height: usize, format: PixelFormat) -> FrameProgress<u8> {
    FrameProgress::new(info(width, height, format)).expect("frame progress")
}

/// A pending slot, its writer, and the progress both share.
fn pending<T: splot_recon::ReconSample>(
    width: usize,
    height: usize,
    format: PixelFormat,
) -> (RefFrameSlot<T>, FrameSlotWriter<T>, Arc<FrameProgress<T>>) {
    let (slot, writer) = RefFrameSlot::pending(info(width, height, format)).expect("pending slot");
    let progress = writer.progress_handle();
    (slot, writer, progress)
}

#[test]
fn direct_stripes_write_the_canonical_allocation_and_publish_out_of_order() {
    let (slot, writer, progress) = pending::<u16>(8, 8, PixelFormat::Monochrome);
    progress.begin(&[(0, 4), (4, 8)]).expect("stripe geometry");

    let mut bottom = progress.direct_stripe(1).expect("bottom stripe lend");
    assert!(progress.direct_stripe(1).is_none(), "a live lend is unique");
    let mut bottom_target = bottom.take_target().expect("bottom target");
    bottom_target
        .take(splot_recon::PlaneId::Y)
        .expect("bottom luma")
        .samples_mut()
        .fill(22);
    assert!(bottom.submit());
    assert_eq!(progress.published_luma_rows(), 0);

    let mut top = progress.direct_stripe(0).expect("top stripe lend");
    let mut top_target = top.take_target().expect("top target");
    top_target
        .take(splot_recon::PlaneId::Y)
        .expect("top luma")
        .samples_mut()
        .fill(11);
    assert!(top.submit());
    assert_eq!(progress.published_luma_rows(), 8);

    let published = PublishedFrame::of_slot(&slot).expect("published frame");
    let plane = published
        .plane(splot_recon::PlaneId::Y)
        .expect("published luma");
    let (samples, half) = (plane.samples, 4 * plane.stride);
    assert!(samples[..half].iter().all(|&sample| sample == 11));
    assert!(samples[half..].iter().all(|&sample| sample == 22));

    progress.freeze_workspace(writer).expect("frozen frame");
    let frame = slot.try_frozen().expect("settled slot");
    assert_eq!(&frame.y().samples()[..half], vec![11; half]);
    assert_eq!(&frame.y().samples()[half..], vec![22; half]);
}

#[test]
fn a_direct_target_views_its_rows_then_writes_them() {
    let progress = Arc::new(new_progress(8, 8, PixelFormat::Monochrome));
    progress.begin(&[(0, 4), (4, 8)]).expect("stripe geometry");
    let mut lease = progress.direct_stripe(1).expect("bottom stripe lend");
    let mut target = lease.take_target().expect("bottom target");
    let mut luma = target.take(PlaneId::Y).expect("bottom luma");
    luma.samples_mut().fill(7);
    let rows = luma.rows().expect("bottom rows");
    let other = rows;
    assert_eq!((rows.origin_y(), rows.end_y(), rows.width()), (4, 8, 8));
    assert_eq!(rows.row(3), None);
    assert_eq!(rows.row(7).map(<[u8]>::len), Some(8));
    assert!((4..8).all(|y| other.row(y).is_some_and(|row| row.iter().all(|&s| s == 7))));
    let stride = rows.stride();
    luma.samples_mut()[stride * 3] = 9;
    assert_eq!(
        luma.rows().and_then(|rows| rows.row(7)).map(|row| row[0]),
        Some(9)
    );
    drop(luma);
    assert!(lease.submit());
}

#[test]
fn detached_direct_target_keeps_its_stripe_leased() {
    let (slot, writer, progress) = pending::<u16>(8, 4, PixelFormat::Monochrome);
    progress.begin(&[(0, 4)]).expect("stripe geometry");

    let mut lease = progress.direct_stripe(0).expect("stripe lend");
    let mut target = lease.take_target().expect("stripe target");
    let plane = target.take(splot_recon::PlaneId::Y).expect("luma target");
    assert!(!lease.submit(), "a detached target cannot be published");
    assert!(progress.direct_stripe(0).is_none());
    assert!(
        progress.freeze_workspace(writer).is_err(),
        "freeze must refuse a live direct lend"
    );
    assert!(
        slot.ready().is_err(),
        "a refused freeze settles the slot failed"
    );

    drop(plane);
    assert!(progress.direct_stripe(0).is_some());
}

#[cfg(test)]
#[test]
fn detached_target_drop_hands_writes_back_before_submit() {
    let (slot, _writer, progress) = pending::<u16>(8, 4, PixelFormat::Monochrome);
    progress.begin(&[(0, 4)]).expect("stripe geometry");

    let mut lease = progress.direct_stripe(0).expect("stripe lend");
    let mut target = lease.take_target().expect("stripe target");
    let mut plane = target.take(splot_recon::PlaneId::Y).expect("luma target");
    std::thread::spawn(move || plane.samples_mut().fill(33))
        .join()
        .expect("target worker");
    assert!(lease.submit());

    let published = PublishedFrame::of_slot(&slot).expect("published frame");
    assert!(
        published
            .plane(splot_recon::PlaneId::Y)
            .expect("published luma")
            .samples
            .iter()
            .all(|&sample| sample == 33)
    );
}

#[test]
fn u8_direct_stripe_writes_the_canonical_allocation() {
    let (slot, writer, progress) = pending::<u8>(8, 4, PixelFormat::Monochrome);
    progress.begin(&[(0, 4)]).expect("stripe geometry");

    let mut lease = progress.direct_stripe(0).expect("stripe lend");
    let mut target = lease.take_target().expect("stripe target");
    target
        .take(splot_recon::PlaneId::Y)
        .expect("luma target")
        .samples_mut()
        .fill(77);
    assert!(lease.submit());

    progress.freeze_workspace(writer).expect("frozen frame");
    let frame = slot.try_frozen().expect("settled slot");
    assert_eq!(
        frame.y().samples(),
        vec![77; 4 * frame.y().stride_samples()]
    );
}

#[test]
fn direct_mode_rejects_misaligned_subsampled_stripes() {
    let progress = Arc::new(
        FrameProgress::<u16>::new(info(8, 128, PixelFormat::Yuv420)).expect("frame progress"),
    );

    progress
        .begin(&[(0, 65), (65, 128)])
        .expect("stripe geometry");
    assert!(progress.direct_stripe(0).is_none());
    assert!(progress.direct_stripe(1).is_none());
}

#[test]
fn out_of_order_stripes_advance_only_the_contiguous_prefix() {
    let progress = new_progress(64, 192, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128), (128, 192)])
        .expect("stripe geometry");
    assert_eq!(progress.published_luma_rows(), 0);

    progress.publish(2);
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "a late stripe alone publishes no prefix"
    );

    progress.publish(1);
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "the prefix still misses the frame top"
    );

    progress.publish(0);
    assert_eq!(
        progress.published_luma_rows(),
        192,
        "closing the top completes every landed stripe at once"
    );
}

#[test]
fn the_watermark_advances_one_stripe_at_a_time_in_order() {
    let progress = new_progress(64, 160, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128), (128, 160)])
        .expect("stripe geometry");

    for (stripe, expected) in [(0usize, 64usize), (1, 128), (2, 160)] {
        progress.publish(stripe);
        assert_eq!(progress.published_luma_rows(), expected);
    }
}

#[test]
fn a_repeated_or_out_of_range_publish_never_moves_the_watermark_backwards() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");

    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(7);
    assert_eq!(progress.published_luma_rows(), 64);
    progress.publish(1);
    assert_eq!(progress.published_luma_rows(), 128);
}

fn is_filter_state_error(result: &crate::Result<()>) -> bool {
    matches!(
        result,
        Err(crate::error::DecodeError::HeaderState {
            source: crate::error::DecodeHeaderStateError::InvalidLoopRestorationFilterState,
        })
    )
}

#[test]
fn a_non_contiguous_geometry_is_refused_and_publishes_nothing() {
    let progress = new_progress(64, 192, PixelFormat::Monochrome);
    assert!(
        is_filter_state_error(&progress.begin(&[(0, 64), (128, 192)])),
        "a gap must be refused"
    );
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 0);

    let descending = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(is_filter_state_error(
        &descending.begin(&[(64, 128), (0, 64)])
    ));

    let empty_stripe = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(is_filter_state_error(
        &empty_stripe.begin(&[(0, 0), (0, 128)])
    ));
}

#[test]
fn the_geometry_installs_once() {
    let progress = new_progress(64, 128, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    assert!(
        is_filter_state_error(&progress.begin(&[(0, 128)])),
        "a second geometry must not replace the first"
    );
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
}

#[test]
fn chroma_rows_truncate_to_the_fully_published_luma_pairs() {
    let (slot, _writer, progress) = pending::<u8>(64, 192, PixelFormat::Yuv420);
    progress
        .begin(&[(0, 65), (65, 192)])
        .expect("stripe geometry");

    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 65);
    assert_eq!(
        PublishedFrame::of_slot(&slot)
            .expect("a published prefix")
            .chroma_rows(),
        32,
        "luma row 64 alone does not complete chroma row 32"
    );

    let (slot, _full_writer, full) = pending::<u8>(64, 128, PixelFormat::Yuv420);
    full.begin(&[(0, 64), (64, 128)]).expect("stripe geometry");
    full.publish(0);
    let published = PublishedFrame::of_slot(&slot).expect("a published prefix");
    assert_eq!(published.chroma_rows(), 32);

    let (slot, _writer, unsubsampled) = pending::<u8>(64, 128, PixelFormat::Yuv444);
    unsubsampled
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    unsubsampled.publish(0);
    let published = PublishedFrame::of_slot(&slot).expect("a published prefix");
    assert_eq!(published.chroma_rows(), 64);
}

#[test]
fn a_complete_odd_height_frame_publishes_its_terminal_chroma_row() {
    let (slot, _writer, progress) = pending::<u8>(64, 129, PixelFormat::Yuv420);
    progress
        .begin(&[(0, 64), (64, 129)])
        .expect("stripe geometry");

    progress.publish(0);
    assert_eq!(
        PublishedFrame::of_slot(&slot)
            .expect("an interior prefix")
            .chroma_rows(),
        32,
        "an interior prefix still requires complete luma pairs"
    );

    progress.publish(1);
    assert_eq!(
        PublishedFrame::of_slot(&slot)
            .expect("the complete frame")
            .chroma_rows(),
        65,
        "the final unpaired luma row completes the terminal chroma row"
    );
}

#[test]
fn reads_are_refused_before_the_first_stripe_and_after_the_freeze() {
    let (slot, writer, progress) = pending::<u8>(64, 128, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    assert!(
        PublishedFrame::of_slot(&slot).is_none(),
        "an unpublished frame exposes no rows"
    );

    progress.publish(0);
    let published = PublishedFrame::of_slot(&slot).expect("a published prefix");
    assert_eq!(published.luma_rows(), 64);
    let rows = published.plane(PlaneId::Y).expect("published luma").samples;

    progress.freeze_workspace(writer).expect("the frozen frame");
    assert!(
        PublishedFrame::of_slot(&slot).is_none(),
        "the freeze closes every new banded read"
    );
    let frame = slot.try_frozen().expect("the settled frame");
    assert!(
        core::ptr::eq(rows.as_ptr(), frame.y().samples().as_ptr()),
        "a read taken before the freeze keeps reading the frozen frame's own rows"
    );
    assert_eq!(rows, &frame.y().samples()[..rows.len()]);
    assert!(matches!(
        slot.hold_samples(),
        Some(HeldFrameSamples::Settled(_))
    ));

    let mut owned = new_progress(64, 128, PixelFormat::Monochrome);
    assert!(owned.freeze().is_ok());
    assert!(owned.freeze().is_err(), "the workspace is frozen once");
}

#[test]
fn a_failed_phase_publishes_no_readable_row() {
    let (slot, _writer, progress) = pending::<u8>(64, 128, PixelFormat::Yuv420);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    progress.publish(0);
    assert_eq!(progress.published_luma_rows(), 64);
    assert!(!progress.terminal_published.is_set());

    progress.publish_terminal(false);
    assert!(progress.terminal_published.is_set());
    assert_eq!(
        progress.published_luma_rows(),
        0,
        "a failed phase names no readable row, not the whole frame"
    );
    assert!(
        PublishedFrame::of_slot(&slot).is_none(),
        "the never-frozen workspace of a failed phase must not be readable"
    );
}

#[test]
fn a_finished_phase_publishes_the_whole_frame() {
    let (slot, _writer, progress) = pending::<u8>(64, 128, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    assert!(!progress.terminal_published.is_set());

    progress.publish_terminal(true);
    assert!(progress.terminal_published.is_set());
    assert_eq!(
        progress.published_luma_rows(),
        128,
        "a phase that filtered every row admits every row threshold"
    );
    assert!(
        PublishedFrame::of_slot(&slot).is_none(),
        "a finished phase's rows are read from its settled slot"
    );
}

#[test]
fn the_freeze_settles_the_slot_before_it_publishes_frozen() {
    let (slot, writer, progress) = pending::<u8>(64, 128, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 64), (64, 128)])
        .expect("stripe geometry");
    progress.publish(0);

    progress.freeze_workspace(writer).expect("the frozen frame");
    assert_eq!(progress.published_luma_rows.current(), FROZEN);
    assert_eq!(progress.published_luma_rows(), 128);
    assert!(matches!(
        slot.hold_samples(),
        Some(HeldFrameSamples::Settled(_))
    ));
    let (_, other, _) = pending::<u8>(64, 128, PixelFormat::Monochrome);
    assert!(
        progress.freeze_workspace(other).is_err(),
        "only the paired writer freezes a frame"
    );
}

#[test]
fn a_failed_freeze_keeps_the_rows_a_reader_holds() {
    let (slot, writer, progress) = pending::<u16>(8, 4, PixelFormat::Monochrome);
    progress.begin(&[(0, 4)]).expect("stripe geometry");
    let mut lease = progress.direct_stripe(0).expect("stripe lend");
    let mut target = lease.take_target().expect("stripe target");
    target
        .take(PlaneId::Y)
        .expect("luma target")
        .samples_mut()
        .fill(30);
    drop(target);
    assert!(lease.submit());
    let published = PublishedFrame::of_slot(&slot).expect("the published frame");
    let rows = published.plane(PlaneId::Y).expect("held luma").samples;
    let taken = progress.workspace.lock().take();

    assert!(
        progress.freeze_workspace(writer).is_err(),
        "a taken workspace cannot freeze"
    );
    assert!(slot.ready().is_err());
    assert!(PublishedFrame::of_slot(&slot).is_none());
    assert!(
        published
            .plane(PlaneId::Y)
            .is_some_and(|plane| plane.samples.is_empty()),
        "a failed phase lends no further row"
    );
    assert!(
        rows.iter().all(|&sample| sample == 30),
        "rows lent before the failure stay allocated"
    );
    drop(taken);
}

/// Two writers lease and publish stripes while two readers spin on the
/// published prefix, then a freeze through the slot's own writer; the readers
/// keep reading the rows they hold across the freeze and check them against
/// the frozen frame.
#[test]
fn readers_check_published_rows_across_concurrent_stripes_and_the_freeze() {
    const STRIPES: usize = 6;
    const ROWS: usize = 4;
    const HEIGHT: usize = STRIPES * ROWS;
    fn value(row: usize) -> u16 {
        u16::try_from(row * 7 + 3).expect("8-bit sample")
    }
    fn check(published: PublishedFrame<'_, u16>) {
        let plane = published.plane(PlaneId::Y).expect("published luma");
        for (row, samples) in plane.samples.chunks(plane.stride).enumerate() {
            assert!(row < published.luma_rows());
            assert!(samples[..8].iter().all(|&sample| sample == value(row)));
        }
    }
    let (slot, writer, progress) = pending::<u16>(8, HEIGHT, PixelFormat::Monochrome);
    let ranges: Vec<_> = (0..STRIPES)
        .map(|stripe| (stripe * ROWS, (stripe + 1) * ROWS))
        .collect();
    progress.begin(&ranges).expect("stripe geometry");
    let (next, whole) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let writers = (0..2).map(|_| {
        let (progress, next) = (Arc::clone(&progress), Arc::clone(&next));
        splot_parallel::spawn_thread("stripe writer", move || {
            loop {
                let stripe = next.fetch_add(1, Ordering::Relaxed);
                if stripe >= STRIPES {
                    break;
                }
                let mut lease = progress.direct_stripe(stripe).expect("stripe lend");
                let mut target = lease.take_target().expect("stripe target");
                let mut luma = target.take(PlaneId::Y).expect("luma target");
                let (origin, stride) = (luma.origin_y(), luma.stride());
                for (row, samples) in luma.samples_mut().chunks_mut(stride).enumerate() {
                    samples.fill(value(origin + row));
                }
                drop((luma, target));
                assert!(lease.submit());
            }
        })
    });
    let readers = (0..2).map(|_| {
        let (slot, whole) = (slot.share(), Arc::clone(&whole));
        splot_parallel::spawn_thread("prefix reader", move || {
            let mut held = None;
            loop {
                if let Some(published) = PublishedFrame::of_slot(&slot) {
                    check(published);
                    if held.is_none() && published.luma_rows() == HEIGHT {
                        whole.fetch_add(1, Ordering::Release);
                        held = Some(published);
                    }
                } else if let Some(frame) = slot.try_frozen() {
                    let held = held.expect("the freeze waits for this reader");
                    check(held);
                    let rows = held.plane(PlaneId::Y).expect("held luma").samples;
                    assert_eq!(rows, &frame.y().samples()[..rows.len()]);
                    break;
                }
                std::thread::yield_now();
            }
        })
    });
    let threads: Vec<_> = writers
        .chain(readers)
        .collect::<Result<_, _>>()
        .expect("test threads");
    while whole.load(Ordering::Acquire) < 2 && !threads[2..].iter().any(JoinHandle::is_finished) {
        std::thread::yield_now();
    }
    progress.freeze_workspace(writer).expect("the frozen frame");
    for thread in threads {
        thread.join().expect("test thread");
    }
}

#[test]
fn resetting_progress_retains_stripes_and_clears_terminal_publication() {
    let geometry = info(8, 8, PixelFormat::Monochrome);
    let mut progress = FrameProgress::<u8>::new(geometry).expect("frame progress");
    let mut planes = splot_recon::FramePlaneSamples::default();
    progress.begin(&[(0, 4), (4, 8)]).expect("stripe geometry");
    let stripes = progress
        .layout
        .get()
        .expect("stripe layout")
        .lock()
        .stripes
        .as_ptr();
    for cycle in 0..1200 {
        progress.publish_terminal(cycle % 2 == 0);
        progress
            .reset(
                splot_recon::CurrentFrameWorkspace::new_recycled_from(geometry, &mut planes)
                    .expect("reset workspace"),
            )
            .expect("reset progress");
        assert_eq!(progress.published_luma_rows(), 0);
        assert!(!progress.terminal_published.is_set());
        assert!(progress.layout.get().is_none());
        progress.begin(&[(0, 4), (4, 8)]).expect("stripe geometry");
        assert_eq!(
            progress
                .layout
                .get()
                .expect("stripe layout")
                .lock()
                .stripes
                .as_ptr(),
            stripes
        );
    }
}

#[test]
fn frontier_rows_lend_only_rows_no_stripe_or_reader_holds() {
    let (_slot, writer, progress) = pending::<u16>(4, 32, PixelFormat::Monochrome);
    progress
        .begin(&[(0, 16), (16, 32)])
        .expect("stripe geometry");
    let mut rows = progress.frontier_rows().expect("frontier rows");
    assert!(progress.frontier_rows().is_none(), "the frontier is unique");
    rows.with_plane_rows_mut(
        splot_recon::PlaneId::Y,
        0,
        32,
        |samples, stride, _, _, _| {
            for (y, row) in samples.chunks_mut(stride).enumerate() {
                row.fill(u16::try_from(y).expect("row"));
            }
        },
    )
    .expect("unfinalized rows");
    assert!(rows.publish_final_rows(32));
    assert!(progress.direct_stripe(0).is_none(), "an unreleased stripe");

    let mut first = crate::filters::source::DeblockedWindow::default();
    let mut carry = crate::filters::source::DeblockedWindow::default();
    first
        .fill(&mut rows, &mut carry, (0, 16), 4)
        .expect("first window");
    let mut lease = progress.direct_stripe(0).expect("released stripe");
    lease
        .take_target()
        .and_then(|mut target| target.take(splot_recon::PlaneId::Y))
        .expect("luma target")
        .samples_mut()
        .fill(99);
    assert!(lease.submit());
    assert!(
        rows.append_rows(
            splot_recon::PlaneId::Y,
            12,
            20,
            &mut splot_recon::plane_buf()
        )
        .is_none(),
        "a published row is not the frontier's"
    );
    assert!(
        rows.with_plane_rows_mut(splot_recon::PlaneId::Y, 24, 32, |_, _, _, _, _| ())
            .is_none(),
        "a final row is not written again"
    );

    let mut second = crate::filters::source::DeblockedWindow::default();
    second
        .fill(&mut rows, &mut carry, (16, 32), 4)
        .expect("second window");
    let planes = second.planes().expect("window planes");
    assert_eq!(
        planes.y.row(12),
        Some(&[12; 4][..]),
        "carried deblocked row"
    );
    assert_eq!(planes.y.row(20), Some(&[20; 4][..]));
    assert!(
        progress.freeze_workspace(writer).is_err(),
        "a live frontier"
    );
    drop(rows);
    assert!(progress.direct_stripe(1).is_some());
}

#[cfg(test)]
#[test]
fn the_commit_band_and_the_frontier_write_disjoint_rows_concurrently() {
    use splot_recon::PlaneId::{U, Y};
    let (_slot, writer, progress) = pending::<u8>(16, 64, PixelFormat::Yuv420);
    progress.begin(&[(0, 64)]).expect("stripe geometry");
    let mut rows = progress.frontier_rows().expect("frontier rows");
    let scratch = splot_recon::IntraPredictionScratch::new;
    let mut band = rows.lend_commit_rows(0, 16, scratch()).expect("first band");
    assert!(
        rows.lend_commit_rows(16, 16, scratch()).is_err(),
        "one live band"
    );
    let whole = |y, height| PlaneRect::new(0, y, 16, height).expect("rows");
    band.fill_rect(Y, whole(0, 16), 1).expect("luma rows");
    band.fill_rect(U, PlaneRect::new(0, 0, 8, 8).expect("rows"), 1)
        .expect("chroma rows");
    assert!(
        band.fill_rect(Y, whole(16, 1), 1).is_err(),
        "a row it was not lent"
    );
    let kept = rows.return_commit_rows(band).expect("the live band");
    let mut band = rows.lend_commit_rows(16, 16, kept).expect("next band");
    assert!(
        rows.with_plane_rows_mut(Y, 0, 16, |_, _, _, _, _| ())
            .is_none()
    );
    assert!(
        rows.with_plane_rows_mut(U, 0, 8, |_, _, _, _, _| ())
            .is_none()
    );
    assert!(!rows.publish_final_rows(16), "a lent row is not final");
    std::thread::scope(|scope| {
        scope.spawn(|| {
            assert_eq!(band.reconstructed_sample(Y, 3, 15).expect("edge row"), 1);
            band.fill_rect(Y, whole(16, 16), 2).expect("luma rows");
        });
        rows.with_plane_rows_mut(Y, 0, 15, |samples, _, _, _, _| samples.fill(7))
            .expect("frontier luma rows");
        rows.with_plane_rows_mut(U, 0, 7, |samples, _, _, _, _| samples.fill(7))
            .expect("frontier chroma rows");
    });
    let stranger = splot_recon::CurrentFrameWorkspace::new_band(rows.info(), 0, None);
    assert!(rows.return_commit_rows(stranger.expect("band")).is_err());
    rows.return_commit_rows(band).expect("the live band");
    let mut seen = Vec::new();
    rows.with_plane_rows_mut(Y, 0, 64, |samples, stride, _, _, _| {
        seen.extend(samples.chunks(stride).map(|row| row[0]));
    })
    .expect("every row is the frontier's again");
    assert_eq!(&seen[..14], &[7; 14]);
    assert_eq!(seen[15], 1);
    assert_eq!(&seen[16..32], &[2; 16]);
    assert!(rows.publish_final_rows(16));
    let _band = rows
        .lend_commit_rows(32, 16, scratch())
        .expect("a band after the return");
    drop(rows);
    assert!(
        progress.freeze_workspace(writer).is_err(),
        "a live band keeps the frontier claimed"
    );
}
