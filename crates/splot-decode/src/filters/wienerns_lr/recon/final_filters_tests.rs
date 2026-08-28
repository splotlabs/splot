// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::super::{OwnedFilterJob, OwnedFilterSetup};
use super::*;
use crate::filters::wienerns_lr::WienerNsLrTxSkipTransformRecord;
use splot_recon::{BitDepth, CurrentFrameWorkspace};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct AdmitCounter(AtomicUsize);

impl<'job> splot_parallel::Admit<'job> for AdmitCounter {
    fn admit_ready(&self) -> usize {
        self.0.fetch_add(1, Ordering::SeqCst);
        0
    }

    fn submit(
        &self,
        _order_key: u64,
        _conditions: &[splot_parallel::Condition<'_>],
        job: splot_parallel::Job<'job>,
    ) {
        drop(job);
    }

    fn spawn_ready(&self, job: splot_parallel::Job<'job>) {
        drop(job);
    }

    fn submit_ready_batch(&self, _order_key: u64, jobs: Vec<splot_parallel::Job<'job>>) {
        drop(jobs);
    }

    fn continue_ready(&self, _order_key: u64, job: splot_parallel::Job<'job>) {
        drop(job);
    }
}

fn block(plane: usize, x: usize, y: usize) -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        restoration_type: crate::bitstream::tile_payload::LrUnitRestorationType::WienerNonsep,
        plane,
        unit_row: 0,
        unit_col: 0,
        unit_filter_index: None,
        tile_mi_row_start: 0,
        tile_mi_row_end: 4,
        tile_mi_col_end: 4,
        x,
        y,
        width: 4,
        height: 4,
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 15,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 15,
    }
}

#[test]
fn lr_unit_filter_lookup_uses_the_recorded_offset() {
    let mut source = block(1, 0, 0);
    let matching = WienerNsLrUnitFilter {
        plane: 1,
        unit_row: 0,
        unit_col: 0,
        coeff_count: WIENER_NS_CHROMA_COEFFS,
        coeffs: [1; WIENER_NS_CHROMA_COEFFS],
    };
    let mut other = matching;
    other.unit_col = 1;
    let filters = [matching, other];

    source.unit_filter_index = Some(1);
    assert!(lr_unit_filter_for_block(&filters, &source).is_err());
    source.unit_filter_index = Some(0);
    assert_eq!(
        lr_unit_filter_for_block(&filters, &source).unwrap(),
        &matching
    );
}

fn switchable_core() -> FrameHeaderCore {
    let fixture = include_bytes!(
        "../../../../../../tests/conformance/vectors/valid/\
         syn-2frame-lr-switchable-768x256-8bit.ivf"
    );
    crate::prediction::inter::test_support::fixture_sequence_and_key_core(fixture).1
}

fn lr_sink(snapshot: &[u8]) -> WienerNsLrReconSink<u8> {
    let mut workspace = crate::test_support::yuv420_workspace(16, 16, 0);
    for (index, &sample) in snapshot.iter().enumerate() {
        workspace
            .set_reconstructed_sample(PlaneId::Y, index % 16, index / 16, sample)
            .unwrap();
    }
    let mut sink =
        WienerNsLrReconSink::for_final_filtering(workspace, 16, 16, splot_recon::BitDepth::Eight);
    sink.tx_skip_grid =
        Some(crate::filters::wienerns_lr::WienerNsLrTxSkipGrid::new(4, 4, vec![0; 16]).unwrap());
    sink
}

fn skip_grid_records(skip_flag: bool, eob: usize) -> Vec<WienerNsLrTxSkipTransformRecord> {
    vec![WienerNsLrTxSkipTransformRecord {
        row: 0,
        col: 0,
        rows: 4,
        cols: 4,
        skip_flag,
        eob,
    }]
}

#[test]
fn cdef_only_builds_cdef_grid_without_retaining_lr_grid() {
    let mut core = switchable_core();
    core.cdef_params
        .as_mut()
        .unwrap()
        .cdef_on_skip_txfm_frame_enable = Some(false);
    core.lr_params = None;
    let workspace = crate::test_support::yuv420_workspace(16, 16, 0);
    let mut sink =
        WienerNsLrReconSink::for_final_filtering(workspace, 16, 16, splot_recon::BitDepth::Eight);
    sink.filter_records.tx_skip_records = skip_grid_records(true, 0);

    assert!(!sink.needs_lr_tx_skip_grid(&core));
    assert_eq!(
        sink.cdef_skip_grid(&core, 4, 4).unwrap(),
        Some(crate::filters::cdef::CdefSkipGrid::new(4, 4, vec![true; 16]).unwrap())
    );
    assert!(sink.tx_skip_grid.is_none());
}

#[test]
fn luma_lr_path_retains_its_distinct_tx_skip_grid() {
    let core = switchable_core();
    let workspace = crate::test_support::yuv420_workspace(16, 16, 0);
    let mut sink =
        WienerNsLrReconSink::for_final_filtering(workspace, 16, 16, splot_recon::BitDepth::Eight);
    sink.filter_records.tx_skip_records = skip_grid_records(false, 0);
    sink.filter_records.lr_source_blocks.push(block(0, 0, 0));

    assert!(sink.needs_lr_tx_skip_grid(&core));
    sink.ensure_tx_skip_grid(4, 4).unwrap();
    let grid = sink.tx_skip_grid.as_ref().unwrap();
    assert_eq!(
        grid.lookup(crate::filters::wienerns_lr::WienerNsLrTxSkipLookup { row: 0, col: 0 })
            .unwrap(),
        1
    );
}

const fn deblock_prediction(r: usize, c: usize) -> crate::filters::deblock::DeblockPredictionUnit {
    crate::filters::deblock::DeblockPredictionUnit {
        base_r: r,
        base_c: c,
        default_sub_pu_tx: 3,
    }
}

fn deblock_records() -> Vec<crate::filters::deblock::DeblockBlock> {
    [0, 8]
        .into_iter()
        .map(|c| crate::filters::deblock::DeblockBlock {
            r: 0,
            c,
            luma_prediction: deblock_prediction(0, c),
            chroma_prediction: deblock_prediction(0, c),
            chroma_base_r: 0,
            chroma_base_c: c,
            n4w: 8,
            n4h: 8,
            luma_tx: 3,
            chroma_tx: Some(2),
            sub_pu_size: None,
            chroma_transform_only: false,
            qindex: 100,
            skip: false,
            lossless: false,
        })
        .collect()
}

fn deblock_workspace() -> CurrentFrameWorkspace<u8> {
    let mut workspace = crate::test_support::yuv420_workspace(64, 32, 0);
    for y in 0..32 {
        for x in 0..64 {
            workspace
                .set_reconstructed_sample(PlaneId::Y, x, y, if x < 32 { 100 } else { 108 })
                .unwrap();
        }
    }
    workspace
}

#[test]
fn predeblocked_filter_tail_matches_the_combined_path() {
    let mut core = switchable_core();
    let params =
        splot_core::headers::frame::DeblockingFilterParams::new([true; 4], [false; 4], [0; 4]);
    core.deblocking_filter_params = Some(params);
    core.tile_info = None;
    assert_eq!(
        crate::filters::gdf::stripe_ranges(&core, 32).unwrap(),
        [(0, 32)]
    );
    let records = deblock_records();

    let mut combined =
        WienerNsLrReconSink::for_final_filtering(deblock_workspace(), 64, 32, BitDepth::Eight);
    combined.filter_records.deblock_blocks.clone_from(&records);
    let (combined, _) = combined
        .into_filtered_frame(
            std::sync::Arc::new(core.clone()),
            false,
            crate::filters::deblock::DeblockQuantDeltas::ZERO,
            None,
            None,
            core::convert::identity,
        )
        .unwrap();

    let mut staged_workspace = deblock_workspace();
    let chroma_records = crate::filters::deblock::ChromaDeblockRecords::new();
    let mut deblock = crate::filters::deblock::FrameDeblock::prepare(
        &records,
        &chroma_records,
        8,
        16,
        params,
        None,
        false,
        crate::filters::deblock::DeblockQuantDeltas::ZERO,
    )
    .unwrap()
    .unwrap();
    deblock
        .advance(&mut staged_workspace, 8, BitDepth::Eight)
        .unwrap();
    assert!(deblock.finish().is_none());
    let mut staged =
        WienerNsLrReconSink::for_final_filtering(staged_workspace, 64, 32, BitDepth::Eight);
    staged.filter_records.deblock_blocks = records;
    let (staged, _) = staged
        .into_filtered_frame_from_deblocked(
            std::sync::Arc::new(core.clone()),
            false,
            crate::filters::deblock::DeblockQuantDeltas::ZERO,
            None,
            None,
            core::convert::identity,
        )
        .unwrap();

    assert_eq!(
        splot_recon::DecodedFrameHashInput::new(&staged).compute_hash(),
        splot_recon::DecodedFrameHashInput::new(&combined).compute_hash(),
    );
}

fn patterned_10bit_workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    let mut workspace =
        crate::test_support::yuv420_workspace_with(BitDepth::Ten, width, height, 0_u16);
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = workspace.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let value = 128 + ((x * 3 + y * 5 + plane.index() * 17) % 700) as u16;
                workspace
                    .set_reconstructed_sample(plane, x, y, value)
                    .unwrap();
            }
        }
    }
    workspace
}

fn final_filter_sink_10bit() -> WienerNsLrReconSink<u16> {
    WienerNsLrReconSink::for_final_filtering(
        patterned_10bit_workspace(128, 128),
        128,
        128,
        BitDepth::Ten,
    )
}

fn patterned_8bit_workspace<T: splot_recon::ReconSample>(
    width: usize,
    height: usize,
) -> CurrentFrameWorkspace<T> {
    let mut workspace =
        crate::test_support::yuv420_workspace_with(BitDepth::Eight, width, height, T::default());
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let size = workspace.plane(plane).unwrap().storage_size();
        for y in 0..size.height() {
            for x in 0..size.width() {
                let value = ((x * 3 + y * 5 + plane.index() * 17) & 255) as u16;
                workspace
                    .set_reconstructed_sample(plane, x, y, T::try_from_u16(value).unwrap())
                    .unwrap();
            }
        }
    }
    workspace
}

fn final_filter_sink_8bit<T: splot_recon::ReconSample>() -> WienerNsLrReconSink<T> {
    WienerNsLrReconSink::for_final_filtering(
        patterned_8bit_workspace(128, 128),
        128,
        128,
        BitDepth::Eight,
    )
}

#[test]
fn owned_multi_stripe_u8_direct_output_matches_u16_storage_exactly() {
    let core = Arc::new(switchable_core());
    let (expected, _) = final_filter_sink_8bit::<u16>()
        .into_filtered_frame_from_deblocked(
            Arc::clone(&core),
            false,
            crate::filters::deblock::DeblockQuantDeltas::ZERO,
            None,
            None,
            core::convert::identity,
        )
        .unwrap();

    let sink = final_filter_sink_8bit::<u8>();
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(sink.frame_info()).unwrap(),
    );
    let admit = AdmitCounter::default();
    let (setup, workspace) = sink
        .into_owned_filter_setup(
            Arc::clone(&core),
            false,
            Some(Arc::clone(&progress)),
            Some(&admit),
        )
        .unwrap();
    let workspace = workspace.unwrap();
    let ranges = setup.stripe_ranges().to_vec();
    assert_eq!(ranges.len(), 3, "fixture must exercise multiple stripes");

    for stripe in [1usize, 0, 2] {
        let (start, end) = ranges[stripe];
        let window = crate::filters::source::DeblockedWindow::extract(
            &workspace,
            start,
            end,
            super::super::STRIPE_WINDOW_MARGIN,
        )
        .unwrap();
        let filtered = setup.run_owned_window(stripe, window).unwrap();
        setup.publish(filtered).unwrap();
    }
    assert_eq!(
        admit.0.load(Ordering::SeqCst),
        ranges.len(),
        "every direct publication must wake row-gated dependents"
    );
    let (actual, _) = setup.finish(core::convert::identity).unwrap();
    workspace.recycle_planes();
    assert_eq!(
        splot_recon::DecodedFrameHashInput::new(&actual).compute_hash(),
        splot_recon::DecodedFrameHashInput::new(&expected).compute_hash(),
    );
}

#[test]
fn owned_multi_stripe_10bit_filter_matches_monolithic_and_publishes_contiguously() {
    let core = Arc::new(switchable_core());
    let (expected, _) = final_filter_sink_10bit()
        .into_filtered_frame_from_deblocked(
            Arc::clone(&core),
            false,
            crate::filters::deblock::DeblockQuantDeltas::ZERO,
            None,
            None,
            core::convert::identity,
        )
        .unwrap();

    let info = final_filter_sink_10bit().frame_info();
    let progress =
        Arc::new(crate::pipeline::frame_progress::FrameProgress::<u16>::new(info).unwrap());
    let (setup, workspace) = final_filter_sink_10bit()
        .into_owned_filter_setup(Arc::clone(&core), false, Some(Arc::clone(&progress)), None)
        .unwrap();
    let workspace = workspace.unwrap();
    let ranges = setup.stripe_ranges().to_vec();
    assert_eq!(ranges.len(), 3, "fixture must exercise multiple stripes");

    for (stripe, expected_rows) in [(1usize, 0usize), (0, ranges[1].1), (2, 128)] {
        let (start, end) = ranges[stripe];
        let window = crate::filters::source::DeblockedWindow::extract(
            &workspace,
            start,
            end,
            super::super::STRIPE_WINDOW_MARGIN,
        )
        .unwrap();
        let filtered = setup.run_owned_window(stripe, window).unwrap();
        setup.publish(filtered).unwrap();
        assert_eq!(progress.published_luma_rows(), expected_rows);
    }

    let freezes = AtomicUsize::new(0);
    let (actual, _) = setup
        .finish(|frame| {
            freezes.fetch_add(1, Ordering::SeqCst);
            frame
        })
        .unwrap();
    workspace.recycle_planes();

    assert_eq!(freezes.load(Ordering::SeqCst), 1);
    assert!(
        progress.read().is_none(),
        "the terminal freeze must take the progressive workspace"
    );
    assert_eq!(
        splot_recon::DecodedFrameHashInput::new(&actual).compute_hash(),
        splot_recon::DecodedFrameHashInput::new(&expected).compute_hash(),
    );
}

#[test]
fn owned_filter_failure_never_freezes_and_settles_the_pending_slot_once() {
    let core = Arc::new(switchable_core());
    let sink = final_filter_sink_10bit();
    let info = sink.frame_info();
    let (slot, writer) = crate::pipeline::inflight::RefFrameSlot::<u16>::pending(info).unwrap();
    let progress = slot.progress_handle().unwrap();
    let (setup, workspace) = sink
        .into_owned_filter_setup(Arc::clone(&core), false, Some(progress), None)
        .unwrap();
    let workspace = workspace.unwrap();
    let ranges = setup.stripe_ranges().to_vec();

    let (start, end) = ranges[0];
    let window = crate::filters::source::DeblockedWindow::extract(
        &workspace,
        start,
        end,
        super::super::STRIPE_WINDOW_MARGIN,
    )
    .unwrap();
    let filtered = setup.run_owned_window(0, window).unwrap();
    setup.publish(filtered).unwrap();
    assert!(
        setup
            .run_owned_window(
                0,
                crate::filters::source::DeblockedWindow::extract(
                    &workspace,
                    start,
                    end,
                    super::super::STRIPE_WINDOW_MARGIN,
                )
                .unwrap()
            )
            .is_err(),
        "a stripe cannot be claimed twice"
    );
    assert!(
        setup
            .run_owned_window(
                ranges.len(),
                crate::filters::source::DeblockedWindow::extract(
                    &workspace,
                    start,
                    end,
                    super::super::STRIPE_WINDOW_MARGIN,
                )
                .unwrap(),
            )
            .is_err(),
        "an out-of-range stripe must fail closed"
    );

    let freezes = AtomicUsize::new(0);
    assert!(
        setup
            .finish(|frame| {
                freezes.fetch_add(1, Ordering::SeqCst);
                frame
            })
            .is_err(),
        "terminal freeze must reject missing stripes"
    );
    assert_eq!(freezes.load(Ordering::SeqCst), 0);
    drop(writer);
    assert!(slot.is_settled());
    assert_eq!(slot.published_luma_rows(), 0);
    assert!(slot.ready().is_err());
}

fn arc_owned_filter_setup() -> (
    Arc<OwnedFilterSetup<'static, 'static, u16>>,
    CurrentFrameWorkspace<u16>,
    Arc<crate::pipeline::frame_progress::FrameProgress<u16>>,
) {
    let core = Arc::new(switchable_core());
    let sink = final_filter_sink_10bit();
    let progress =
        Arc::new(crate::pipeline::frame_progress::FrameProgress::new(sink.frame_info()).unwrap());
    let (setup, workspace) = sink
        .into_owned_filter_setup_published(core, false, Arc::clone(&progress))
        .unwrap();
    let workspace = workspace.unwrap();
    (Arc::new(setup), workspace, progress)
}

#[test]
fn owned_setup_derives_lossless_grid_before_deblock_records_move()
-> core::result::Result<(), Box<dyn std::error::Error>> {
    let mut core = switchable_core();
    let lossless = core
        .lossless_info
        .as_mut()
        .ok_or("fixture must carry derived lossless facts")?;
    lossless.has_lossless_segment = true;

    let mut records = deblock_records();
    records[0].lossless = true;
    let expected_count = records.len();
    let mut sink = final_filter_sink_10bit();
    sink.filter_records.deblock_blocks = records;
    let progress =
        Arc::new(crate::pipeline::frame_progress::FrameProgress::new(sink.frame_info()).unwrap());
    let (mut setup, workspace) = sink
        .into_owned_filter_setup(Arc::new(core), false, Some(progress), None)
        .unwrap();
    let workspace = workspace.unwrap();

    assert!(
        setup
            .lossless_grid
            .as_ref()
            .is_some_and(|grid| grid.plane_sample_lossless(PlaneId::Y, 0, 0, 0, 0))
    );
    let detached = setup.detach_deblock_records();
    assert_eq!(detached.blocks.len(), expected_count);
    assert!(setup.filter_records.deblock_blocks.is_empty());
    assert!(
        setup
            .lossless_grid
            .as_ref()
            .is_some_and(|grid| grid.plane_sample_lossless(PlaneId::Y, 0, 0, 0, 0))
    );
    workspace.recycle_planes();
    Ok(())
}

fn owned_filter_jobs(
    setup: &Arc<OwnedFilterSetup<'static, 'static, u16>>,
    workspace: &CurrentFrameWorkspace<u16>,
    order: &[usize],
) -> Vec<OwnedFilterJob<u16>> {
    order
        .iter()
        .map(|&stripe| {
            let (start, end) = setup.stripe_ranges()[stripe];
            setup.owned_job(
                stripe,
                crate::filters::source::DeblockedWindow::extract(
                    workspace,
                    start,
                    end,
                    super::super::STRIPE_WINDOW_MARGIN,
                )
                .unwrap(),
            )
        })
        .collect()
}

#[test]
fn arc_owned_filter_jobs_join_out_of_order_restore_records_and_freeze_once() {
    let core = Arc::new(switchable_core());
    let (expected, _) = final_filter_sink_10bit()
        .into_filtered_frame_from_deblocked(
            core,
            false,
            crate::filters::deblock::DeblockQuantDeltas::ZERO,
            None,
            None,
            core::convert::identity,
        )
        .unwrap();
    let (setup, workspace, progress) = arc_owned_filter_setup();
    assert_eq!(setup.stripe_ranges().len(), 3);
    for job in owned_filter_jobs(&setup, &workspace, &[1, 0, 2]) {
        job.run().unwrap();
    }
    let restored = deblock_records();
    setup
        .restore_deblock_records(crate::filters::deblock::OwnedDeblockRecords {
            blocks: restored.clone(),
            chroma: crate::filters::deblock::ChromaDeblockRecords::default(),
        })
        .unwrap();
    workspace.recycle_planes();

    let freezes = AtomicUsize::new(0);
    let (actual, records) = setup
        .owned_finish()
        .finish(|frame| {
            freezes.fetch_add(1, Ordering::SeqCst);
            frame
        })
        .unwrap();

    assert_eq!(freezes.load(Ordering::SeqCst), 1);
    assert_eq!(
        records
            .deblock_blocks
            .iter()
            .map(|block| (block.r, block.c, block.n4w, block.n4h, block.qindex))
            .collect::<Vec<_>>(),
        restored
            .iter()
            .map(|block| (block.r, block.c, block.n4w, block.n4h, block.qindex))
            .collect::<Vec<_>>()
    );
    assert!(progress.read().is_none());
    assert_eq!(
        splot_recon::DecodedFrameHashInput::new(&actual).compute_hash(),
        splot_recon::DecodedFrameHashInput::new(&expected).compute_hash(),
    );
}

#[test]
fn arc_owned_filter_finish_rejects_missing_duplicate_and_shared_owners() {
    let (setup, workspace, _) = arc_owned_filter_setup();
    let mut duplicate = owned_filter_jobs(&setup, &workspace, &[0, 0]);
    duplicate.remove(0).run().unwrap();
    assert!(duplicate.remove(0).run().is_err());
    assert!(
        setup
            .owned_finish()
            .finish(core::convert::identity)
            .is_err(),
        "missing stripes must prevent terminal freeze"
    );
    workspace.recycle_planes();

    let (setup, workspace, _) = arc_owned_filter_setup();
    for job in owned_filter_jobs(&setup, &workspace, &[0, 1, 2]) {
        job.run().unwrap();
    }
    let lingering = Arc::clone(&setup);
    assert!(
        setup
            .owned_finish()
            .finish(core::convert::identity)
            .is_err(),
        "terminal freeze requires the sole Arc owner"
    );
    drop(lingering);
    workspace.recycle_planes();
}

#[test]
fn owned_filter_task_values_are_send_and_shared_setup_is_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    assert_send::<super::super::OwnedFilteredStripe<u16>>();
    assert_send::<super::super::OwnedFilterJob<u16>>();
    assert_send::<super::super::OwnedFilterFinish<u16>>();
    assert_sync::<super::super::OwnedFilterSetup<'static, 'static, u16>>();
}

fn luma_rect(samples: &[u8], x: usize) -> Vec<u8> {
    (0..4)
        .flat_map(|y| samples[y * 16 + x..y * 16 + x + 4].iter().copied())
        .collect()
}

fn apply_luma_lr(
    sink: &WienerNsLrReconSink<u8>,
    core: &FrameHeaderCore,
    blocks: &[WienerNsLrSourceBlock],
) -> Vec<u8> {
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(sink.frame_info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 16)]));
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let (target, chroma) = target.split([false, true, true]);
    drop(chroma);
    let cdef = crate::filters::cdef::cdef_stripe(
        crate::filters::source::DeblockedPlanes::frame(sink.workspace.as_ref().unwrap()).unwrap(),
        None,
        None,
        None,
        None,
        (4, 4),
        (1, 1),
        splot_recon::BitDepth::Eight,
        None,
        0,
        16,
    )
    .unwrap();
    let filtered = sink
        .stripe_chain()
        .apply_lr_stripe(
            core,
            cdef,
            &CdefOverlap::default(),
            [blocks, &[], &[]],
            &[],
            super::LrStripeOutput {
                active_planes: [true, false, false],
                direct_u8_planes: [false; 3],
                target,
            },
        )
        .unwrap()
        .into_filtered();
    let mut filtered = filtered;
    filtered.y.finish_direct().unwrap();
    drop(filtered);
    assert!(lease.submit());
    let frame = progress.freeze_workspace(core::convert::identity).unwrap();
    frame.y().samples().to_vec()
}

#[test]
fn inactive_filter_planes_reuse_cdef_storage() {
    let sink = lr_sink(&[0; 16 * 16]);
    let progress = Arc::new(
        crate::pipeline::frame_progress::FrameProgress::<u8>::new(sink.frame_info()).unwrap(),
    );
    assert!(progress.begin(&[(0, 16)]));
    let mut lease = progress.direct_stripe(0).unwrap();
    let target = lease.take_target().unwrap();
    let cdef = crate::filters::cdef::cdef_stripe(
        crate::filters::source::DeblockedPlanes::frame(sink.workspace.as_ref().unwrap()).unwrap(),
        None,
        None,
        None,
        None,
        (4, 4),
        (1, 1),
        splot_recon::BitDepth::Eight,
        None,
        0,
        16,
    )
    .unwrap();
    let cdef_ptr = cdef.filtered_y.samples().as_ptr();
    let filtered = sink
        .stripe_chain()
        .apply_lr_stripe(
            &switchable_core(),
            cdef,
            &CdefOverlap::default(),
            [&[], &[], &[]],
            &[],
            super::LrStripeOutput {
                active_planes: [false; 3],
                direct_u8_planes: [false; 3],
                target,
            },
        )
        .unwrap()
        .into_filtered();

    assert_eq!(filtered.y.as_u16().unwrap().samples().as_ptr(), cdef_ptr);
}

#[test]
fn merges_contiguous_row_blocks_and_splits_on_filter_visible_fields() {
    let mut stripe_split = block(0, 8, 0);
    stripe_split.luma_stripe_end_y = 7;
    let mut unit_split = block(0, 12, 4);
    unit_split.unit_col = 1;
    let blocks = [
        block(0, 4, 0),
        block(1, 0, 0),
        block(0, 0, 0),
        stripe_split,
        block(0, 12, 0),
        block(0, 0, 4),
        block(0, 4, 4),
        block(0, 8, 4),
        unit_split,
    ];

    let runs = coalesced_lr_source_rows(&blocks, 0);
    let shapes: Vec<_> = runs
        .iter()
        .map(|run| (run.x, run.y, run.width, run.height))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (0, 0, 8, 4),
            (8, 0, 4, 4),
            (12, 0, 4, 4),
            (0, 4, 12, 4),
            (12, 4, 4, 4)
        ],
        "runs must merge contiguous same-row blocks and split when any \
         filter-visible field differs"
    );
    assert!(runs.iter().all(|run| run.plane == 0));
}

#[test]
fn does_not_merge_across_row_gaps() {
    let blocks = [block(0, 0, 0), block(0, 8, 0)];
    let runs = coalesced_lr_source_rows(&blocks, 0);
    assert_eq!(runs.len(), 2);
}

#[test]
fn keeps_switchable_restoration_types_in_separate_runs() {
    let mut pc_wiener = block(0, 0, 0);
    pc_wiener.restoration_type = crate::bitstream::tile_payload::LrUnitRestorationType::PcWiener;
    let wiener_nonsep = block(0, 4, 0);

    let runs = coalesced_lr_source_rows(&[pc_wiener, wiener_nonsep], 0);

    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].restoration_type,
        crate::bitstream::tile_payload::LrUnitRestorationType::PcWiener
    );
    assert_eq!(
        runs[1].restoration_type,
        crate::bitstream::tile_payload::LrUnitRestorationType::WienerNonsep
    );
}

#[test]
fn terminal_chroma_wiener_requires_exact_full_plane_coverage() {
    let covered = [
        block(1, 0, 0),
        block(1, 4, 0),
        block(1, 0, 4),
        block(1, 4, 4),
    ];
    assert!(terminal_chroma_wiener_covers(&covered, 8, 0, 8));
    assert!(terminal_chroma_wiener_covers(&covered, 8, 2, 7));
    let mut later_stripe = covered.to_vec();
    later_stripe.extend([block(1, 0, 8), block(1, 4, 8)]);
    assert!(terminal_chroma_wiener_covers(&later_stripe, 8, 8, 12));

    assert!(!terminal_chroma_wiener_covers(&covered[..3], 8, 0, 8));
    let mut overlapping = covered.to_vec();
    overlapping.push(block(1, 0, 0));
    assert!(!terminal_chroma_wiener_covers(&overlapping, 8, 0, 8));

    let mut mixed = covered;
    mixed[3].restoration_type = crate::bitstream::tile_payload::LrUnitRestorationType::PcWiener;
    assert!(!terminal_chroma_wiener_covers(&mixed, 8, 0, 8));
    assert!(!terminal_chroma_wiener_covers(&covered, 0, 0, 8));
    assert!(!terminal_chroma_wiener_covers(&covered, 8, 8, 8));
}

#[test]
fn switchable_luma_dispatches_mixed_units_from_one_snapshot() {
    let snapshot: Vec<u8> = (0..256)
        .map(|index| 48 + ((index * 37 + index / 16 * 19) % 160) as u8)
        .collect();
    let mixed_core = switchable_core();
    assert_eq!(
        mixed_core.lr_params.as_ref().unwrap().planes[0].restoration_type,
        FrameRestorationType::Switchable
    );
    let mut pc_core = mixed_core.clone();
    pc_core.lr_params.as_mut().unwrap().planes[0].restoration_type = FrameRestorationType::PcWiener;
    let mut wiener_ns_core = mixed_core.clone();
    wiener_ns_core.lr_params.as_mut().unwrap().planes[0].restoration_type =
        FrameRestorationType::WienerNonsep;

    let mut pc_block = block(0, 0, 0);
    pc_block.restoration_type = crate::bitstream::tile_payload::LrUnitRestorationType::PcWiener;
    let wiener_ns_block = block(0, 8, 0);
    with_lr_source_scratch::<u8, _>(|scratch| {
        scratch.cell_subclasses.resize(32, usize::MAX);
    });
    let mixed_luma = apply_luma_lr(
        &lr_sink(&snapshot),
        &mixed_core,
        &[pc_block, wiener_ns_block],
    );
    let pc_luma = apply_luma_lr(&lr_sink(&snapshot), &pc_core, &[pc_block]);
    let wiener_ns_luma = apply_luma_lr(&lr_sink(&snapshot), &wiener_ns_core, &[wiener_ns_block]);

    assert_eq!(luma_rect(&mixed_luma, 0), luma_rect(&pc_luma, 0));
    assert_eq!(luma_rect(&mixed_luma, 8), luma_rect(&wiener_ns_luma, 8));
    assert_eq!(luma_rect(&mixed_luma, 4), luma_rect(&snapshot, 4));
    assert_ne!(luma_rect(&mixed_luma, 0), luma_rect(&snapshot, 0));
    assert_ne!(luma_rect(&mixed_luma, 8), luma_rect(&snapshot, 8));
}

#[test]
fn merges_compatible_adjacent_rows_into_rectangles() {
    let blocks = [
        block(0, 0, 0),
        block(0, 4, 0),
        block(0, 0, 4),
        block(0, 4, 4),
    ];
    let runs = coalesced_lr_source_rows(&blocks, 0);

    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].x, runs[0].y), (0, 0));
    assert_eq!((runs[0].width, runs[0].height), (8, 8));
}

#[test]
fn does_not_merge_rows_across_source_boundaries() {
    let top = block(0, 0, 0);
    let mut bottom = block(0, 0, 4);
    bottom.luma_stripe_start_y = 4;
    let runs = coalesced_lr_source_rows(&[top, bottom], 0);

    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].height, 4);
    assert_eq!(runs[1].height, 4);
}

#[test]
fn coalesces_all_planes_in_place_and_returns_ordered_partitions() {
    let blocks = vec![
        block(2, 4, 0),
        block(0, 4, 4),
        block(1, 0, 0),
        block(0, 0, 0),
        block(2, 0, 0),
        block(0, 4, 0),
        block(0, 0, 4),
    ];
    let allocation = blocks.as_ptr();
    let capacity = blocks.capacity();

    let (runs, plane_ends) = coalesced_lr_source_rows_all(blocks);

    assert_eq!(runs.as_ptr(), allocation);
    assert_eq!(runs.capacity(), capacity);
    assert_eq!(plane_ends, [1, 2]);
    assert_eq!(runs.len(), 3);
    assert_eq!(
        runs.iter()
            .map(|run| (run.plane, run.x, run.y, run.width, run.height))
            .collect::<Vec<_>>(),
        vec![(0, 0, 0, 8, 8), (1, 0, 0, 4, 4), (2, 0, 0, 8, 4)]
    );
}

#[test]
fn empty_and_out_of_range_planes_produce_empty_partitions() {
    let (runs, plane_ends) = coalesced_lr_source_rows_all(Vec::new());
    assert!(runs.is_empty());
    assert_eq!(plane_ends, [0, 0]);

    let (runs, plane_ends) =
        coalesced_lr_source_rows_all(vec![block(3, 0, 0), block(usize::MAX, 4, 0)]);
    assert!(runs.is_empty());
    assert_eq!(plane_ends, [0, 0]);
}

#[test]
fn checked_extent_boundaries_do_not_merge() {
    let mut first = block(0, 0, 0);
    first.width = usize::MAX;
    let second = block(0, usize::MAX, 0);

    let runs = coalesced_lr_source_rows(&[first, second], 0);

    assert_eq!(runs, vec![first, second]);
}

#[test]
fn keeps_tile_unit_and_stripe_domains_separate() {
    let first = block(0, 0, 0);
    let mut unit = block(0, 4, 0);
    unit.unit_row = 1;
    let mut tile = block(0, 4, 0);
    tile.tile_mi_col_end = 8;
    let mut stripe = block(0, 4, 0);
    stripe.luma_stripe_start_y = 1;

    for next in [unit, tile, stripe] {
        let runs = coalesced_lr_source_rows(&[first, next], 0);
        assert_eq!(runs, vec![first, next]);
    }
}

#[test]
fn wiener_ns_luma_worker_scratch_retention_is_bounded() {
    WIENER_NS_LUMA_SCRATCH.with(|slot| slot.set(None));

    with_wiener_ns_luma_scratch::<u16, _>(MAX_RETAINED_WIENER_NS_LUMA_SAMPLES + 1, |_| ());
    WIENER_NS_LUMA_SCRATCH.with(|slot| assert!(slot.take().is_none()));

    with_wiener_ns_luma_scratch::<u16, _>(MAX_RETAINED_WIENER_NS_LUMA_SAMPLES, |_| ());
    WIENER_NS_LUMA_SCRATCH.with(|slot| assert!(slot.take().is_some()));
}

#[test]
fn lr_source_window_resolves_in_stripe_rows_from_overlap_planes() {
    let bounds = LoopRestorationSourceBounds {
        luma_start_x: 0,
        luma_end_x: 7,
        luma_start_y: 0,
        luma_end_y: 7,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 7,
        subsampling_x: 0,
        subsampling_y: 0,
    };
    let curr_workspace = crate::test_support::yuv420_workspace(8, 8, 0);
    let curr = FramePlane::new(&curr_workspace, PlaneId::Y).unwrap();
    let mut cdef_workspace = crate::test_support::yuv420_workspace(8, 8, 0);
    for sample in 0..64 {
        cdef_workspace
            .set_reconstructed_sample(PlaneId::Y, sample % 8, sample / 8, sample as u8)
            .unwrap();
    }
    let cdef_source = FramePlane::new(&cdef_workspace, PlaneId::Y).unwrap();
    let band = StripePlane::copy_from(cdef_source, 0, 4).unwrap();
    let overlap = [StripePlane::copy_from(cdef_source, 4, 8).unwrap()];
    let mut storage = Vec::new();

    assert!(
        LrSourceWindow::<u8>::materialize(
            &mut storage,
            PlaneId::Y,
            curr,
            &band,
            &[],
            &bounds,
            2,
            2,
            4,
            4,
            (1, 1),
        )
        .is_err()
    );
    let window = LrSourceWindow::<u8>::materialize(
        &mut storage,
        PlaneId::Y,
        curr,
        &band,
        &overlap,
        &bounds,
        2,
        2,
        4,
        4,
        (1, 1),
    )
    .unwrap();
    assert_eq!(window.get_abs(3, 3), 27);
    assert_eq!(window.get_abs(3, 5), 43);
    assert_eq!(window.get_abs(4, 6), 52);
}

#[test]
fn lr_source_window_reuses_storage_after_an_error() {
    let bounds = LoopRestorationSourceBounds {
        luma_start_x: 0,
        luma_end_x: 7,
        luma_start_y: 0,
        luma_end_y: 7,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 7,
        subsampling_x: 0,
        subsampling_y: 0,
    };
    let curr_workspace = crate::test_support::yuv420_workspace(8, 8, 0);
    let curr = FramePlane::new(&curr_workspace, PlaneId::Y).unwrap();
    let mut cdef_workspace = crate::test_support::yuv420_workspace(8, 8, 0);
    for sample in 0..64 {
        cdef_workspace
            .set_reconstructed_sample(PlaneId::Y, sample % 8, sample / 8, sample as u8)
            .unwrap();
    }
    let cdef_source = FramePlane::new(&cdef_workspace, PlaneId::Y).unwrap();
    let cdef = StripePlane::copy_from(cdef_source, 0, 8).unwrap();
    let short_cdef = StripePlane::copy_from(cdef_source, 0, 1).unwrap();
    let mut storage = Vec::new();

    let window = LrSourceWindow::<u8>::materialize(
        &mut storage,
        PlaneId::Y,
        curr,
        &cdef,
        &[],
        &bounds,
        2,
        2,
        4,
        4,
        (1, 1),
    )
    .unwrap();
    assert_eq!(window.get_abs(2, 2), 18);
    let allocation = window.samples.as_ptr();

    assert!(
        LrSourceWindow::<u8>::materialize(
            &mut storage,
            PlaneId::Y,
            curr,
            &short_cdef,
            &[],
            &bounds,
            2,
            2,
            2,
            2,
            (1, 1),
        )
        .is_err()
    );
    let window = LrSourceWindow::<u8>::materialize(
        &mut storage,
        PlaneId::Y,
        curr,
        &cdef,
        &[],
        &bounds,
        2,
        2,
        2,
        2,
        (1, 1),
    )
    .unwrap();
    assert_eq!(window.samples.as_ptr(), allocation);
    assert_eq!(window.get_abs(3, 3), 27);
}

#[test]
fn lr_source_scratch_does_not_retain_oversized_buffers() {
    LR_SOURCE_SCRATCH.with(|slot| slot.set(None));
    with_lr_source_scratch::<u16, _>(|scratch| {
        scratch
            .primary
            .try_reserve_exact(MAX_RETAINED_LR_SCRATCH_ELEMENTS + 1)
            .unwrap();
    });
    LR_SOURCE_SCRATCH.with(|slot| assert!(slot.take().is_none()));

    let allocation = with_lr_source_scratch::<u16, _>(|scratch| {
        scratch.primary.try_reserve_exact(16).unwrap();
        scratch.primary.as_ptr()
    });
    with_lr_source_scratch::<u16, _>(|scratch| {
        assert_eq!(scratch.primary.as_ptr(), allocation);
    });
    LR_SOURCE_SCRATCH.with(|slot| slot.set(None));
}

#[test]
fn lr_block_output_lands_in_its_rectangle_for_both_storage_widths() {
    fn filtered_plane<T: ReconSample>() -> Vec<u16> {
        let mut plane = StripePlane::from_samples(8, 8, 4, vec![9u16; 32]).unwrap();
        let mut source = block(0, 2, 5);
        source.width = 3;
        source.height = 2;
        filter_lr_block_into::<T>(&mut plane, &source, |output, stride| {
            for row in 0..source.height {
                for col in 0..source.width {
                    output[row * stride + col] =
                        T::try_from_u16((row * source.width + col + 1) as u16).unwrap();
                }
            }
            Ok(())
        })
        .unwrap();
        plane.samples().to_vec()
    }

    let expected = vec![
        9, 9, 9, 9, 9, 9, 9, 9, // row 4
        9, 9, 1, 2, 3, 9, 9, 9, // row 5
        9, 9, 4, 5, 6, 9, 9, 9, // row 6
        9, 9, 9, 9, 9, 9, 9, 9, // row 7
    ];
    assert_eq!(filtered_plane::<u16>(), expected);
    assert_eq!(filtered_plane::<u8>(), expected);
}

#[test]
fn lr_block_output_refuses_a_rectangle_wider_than_the_stripe() {
    let mut plane = StripePlane::from_samples(8, 8, 4, vec![9u16; 32]).unwrap();
    let mut source = block(0, 6, 5);
    source.width = 3;
    source.height = 1;
    assert!(filter_lr_block_into::<u16>(&mut plane, &source, |_, _| Ok(())).is_err());
    assert!(plane.samples().iter().all(|sample| *sample == 9));
}

#[test]
fn lr_output_scratch_reuses_storage_after_an_error() {
    LR_OUTPUT_SCRATCH.with(|slot| slot.set(None));
    let allocation = with_lr_output_scratch::<u16, _>(|output| {
        output.try_reserve_exact(16).unwrap();
        Err::<(), _>(output.as_ptr())
    })
    .unwrap_err();
    with_lr_output_scratch::<u16, _>(|output| {
        assert_eq!(output.as_ptr(), allocation);
    });
    LR_OUTPUT_SCRATCH.with(|slot| slot.set(None));
}
