// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

fn block(plane: usize, x: usize, y: usize) -> WienerNsLrSourceBlock {
    WienerNsLrSourceBlock {
        restoration_type: crate::bitstream::tile_payload::LrUnitRestorationType::WienerNonsep,
        plane,
        row: y / 4,
        col: x / 4,
        unit_row: 0,
        unit_col: 0,
        tile_mi_row_start: 0,
        tile_mi_row_end: 4,
        tile_mi_col_start: 0,
        tile_mi_col_end: 4,
        x,
        y,
        width: 4,
        height: 4,
        luma_start_x: 0,
        luma_end_x: 15,
        luma_start_y: 0,
        luma_end_y: 15,
        frame_luma_end_y: 15,
        luma_stripe_start_y: 0,
        luma_stripe_end_y: 15,
    }
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

fn luma_rect(samples: &[u8], x: usize) -> Vec<u8> {
    (0..4)
        .flat_map(|y| samples[y * 16 + x..y * 16 + x + 4].iter().copied())
        .collect()
}

fn apply_luma_lr(
    sink: &mut WienerNsLrReconSink<u8>,
    core: &FrameHeaderCore,
    blocks: &[WienerNsLrSourceBlock],
) {
    let cdef = crate::filters::cdef::cdef_stripe(
        &sink.workspace,
        None,
        None,
        None,
        None,
        (4, 4),
        splot_recon::BitDepth::Eight,
        0,
        16,
    )
    .unwrap();
    let filtered = sink
        .apply_lr_stripe(core, ByteOffset::new(0), cdef, [blocks, &[], &[]], &[])
        .unwrap()
        .into_filtered();
    WienerNsLrReconSink::publish_filter_stripe_to(
        &mut sink.workspace,
        PlaneId::Y,
        &filtered.y,
        ByteOffset::new(0),
    )
    .unwrap();
}

#[test]
fn inactive_filter_planes_reuse_cdef_storage() {
    let sink = lr_sink(&[0; 16 * 16]);
    let cdef = crate::filters::cdef::cdef_stripe(
        &sink.workspace,
        None,
        None,
        None,
        None,
        (4, 4),
        splot_recon::BitDepth::Eight,
        0,
        16,
    )
    .unwrap();
    let cdef_ptr = cdef.filtered_y.samples().as_ptr();
    let filtered = sink
        .apply_lr_stripe(
            &switchable_core(),
            ByteOffset::new(0),
            cdef,
            [&[], &[], &[]],
            &[],
        )
        .unwrap()
        .into_filtered();

    assert_eq!(filtered.y.samples().as_ptr(), cdef_ptr);
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
    let mut mixed = lr_sink(&snapshot);
    apply_luma_lr(&mut mixed, &mixed_core, &[pc_block, wiener_ns_block]);
    let mut pc_only = lr_sink(&snapshot);
    apply_luma_lr(&mut pc_only, &pc_core, &[pc_block]);
    let mut wiener_ns_only = lr_sink(&snapshot);
    apply_luma_lr(&mut wiener_ns_only, &wiener_ns_core, &[wiener_ns_block]);

    let mixed_luma = mixed.workspace.samples(PlaneId::Y).unwrap();
    let pc_luma = pc_only.workspace.samples(PlaneId::Y).unwrap();
    let wiener_ns_luma = wiener_ns_only.workspace.samples(PlaneId::Y).unwrap();
    assert_eq!(luma_rect(mixed_luma, 0), luma_rect(pc_luma, 0));
    assert_eq!(luma_rect(mixed_luma, 8), luma_rect(wiener_ns_luma, 8));
    assert_eq!(luma_rect(mixed_luma, 4), luma_rect(&snapshot, 4));
    assert_ne!(luma_rect(mixed_luma, 0), luma_rect(&snapshot, 0));
    assert_ne!(luma_rect(mixed_luma, 8), luma_rect(&snapshot, 8));
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
        &bounds,
        2,
        2,
        4,
        4,
        1,
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
            &bounds,
            2,
            2,
            2,
            2,
            1,
        )
        .is_err()
    );
    let window = LrSourceWindow::<u8>::materialize(
        &mut storage,
        PlaneId::Y,
        curr,
        &cdef,
        &bounds,
        2,
        2,
        2,
        2,
        1,
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
