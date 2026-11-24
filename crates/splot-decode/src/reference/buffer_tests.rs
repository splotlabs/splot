// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use crate::pipeline::frame_store::FrameStore;
use crate::pipeline::{PipelineDecodedFrame, PipelineFrame};
use crate::test_support::decoded_frame;
use splot_recon::SharedFrame;
use std::sync::Arc;

fn key_update() -> FrameRefUpdate<'static> {
    FrameRefUpdate {
        refresh_frame_flags: 0xFF,
        order_hint: 0,
        order_hint_lsb: 0,
        implicit_output_frame: false,
        immediate_output_frame: true,
        width: 64,
        height: 64,
        base_q_idx: 70,
        delta_q_u_ac: -2,
        delta_q_v_ac: 3,
        is_inter: false,
        is_intra_frame: true,
        num_total_refs: 0,
        saved_order_hints: [0; 7],
        saved_gm_params: [GlobalMotionRef::identity().gm_params; 7],
        lr_frame_filter_class_counts: [1, 0, 0],
        lr_banks: [None; 3],
        long_term_id: None,
        embedded_layer_id: splot_core::types::EmbeddedLayerId::from_bits(0),
        temporal_layer_id: splot_core::types::TemporalLayerId::BASE,
    }
}

fn inter_update() -> FrameRefUpdate<'static> {
    FrameRefUpdate {
        refresh_frame_flags: 1 << 1,
        order_hint: 1,
        order_hint_lsb: 1,
        implicit_output_frame: true,
        immediate_output_frame: false,
        width: 64,
        height: 64,
        base_q_idx: 109,
        delta_q_u_ac: 4,
        delta_q_v_ac: -1,
        is_inter: true,
        is_intra_frame: false,
        num_total_refs: 1,
        saved_order_hints: [0; 7],
        saved_gm_params: [GlobalMotionRef::identity().gm_params; 7],
        lr_frame_filter_class_counts: [0, 0, 0],
        lr_banks: [None; 3],
        long_term_id: None,
        embedded_layer_id: splot_core::types::EmbeddedLayerId::from_bits(0),
        temporal_layer_id: splot_core::types::TemporalLayerId::BASE,
    }
}

fn valid_count(buf: &RuntimeReferenceBuffer) -> usize {
    buf.slots.iter().filter(|s| s.valid).count()
}

#[test]
fn non_key_refresh_marks_every_refreshed_slot_valid() {
    let mut update = key_update();
    update.refresh_frame_flags = (1 << 2) | (1 << 5);
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();

    buf.update(0, &update, false);

    assert!(buf.slots[2].valid);
    assert!(buf.slots[5].valid);
    assert_eq!(valid_count(&buf), 2);
}

fn pipeline_frame(width: usize, height: usize) -> PipelineFrame {
    PipelineFrame {
        frame: crate::pipeline::inflight::PipelineFrameSlot::completed(
            PipelineDecodedFrame::Eight(SharedFrame::new(decoded_frame(width, height))),
        ),
        display_grain: None,
        output_effects: crate::pipeline::output_effects::FrameOutputEffects::empty(),
        frame_cdfs: crate::prediction::inter::FrameCdfHandle::settled(Arc::new(
            crate::bitstream::tile_payload::FrameCdfSubset::from_defaults(),
        )),
        motion_field: crate::prediction::inter::MotionFieldHandle::pending_with_layout(
            crate::prediction::inter::MotionFieldLayout::new(16, 16, 16)
                .expect("valid motion-field layout"),
        ),
        ccso_header: None,
        ccso_grid: crate::prediction::inter::CcsoGridHandle::settled(None),
        segment_ids: crate::prediction::inter::SegmentIdMapHandle::settled(Arc::new(
            crate::bitstream::tile_payload::FrameSegmentIdMap::new(
                height.div_ceil(4),
                width.div_ceil(4),
            )
            .unwrap(),
        )),
        frame_rate: crate::pipeline::PipelineFrameRate {
            numerator: 1,
            denominator: 1,
        },
        temporal_layer_id: splot_core::types::TemporalLayerId::BASE,
        props: crate::DataProps::default(),
    }
}

#[test]
fn key_refresh_marks_only_first_slot_valid() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    assert_eq!(valid_count(&buf), 1);
    assert!(buf.slots[0].valid);
    assert!(!buf.slots[1].valid);
    assert_eq!(buf.slots[0].base_q_idx, 70);
    assert_eq!(buf.slots[0].counter, 0);
    assert_eq!(buf.slots[0].delta_q_u_ac, -2);
    assert_eq!(buf.slots[0].delta_q_v_ac, 3);
    assert_eq!(buf.slots[0].frame_index, Some(0));
    assert_eq!(buf.slots[0].lr_frame_filter_class_counts, [1, 0, 0]);
}

#[test]
fn inter_refresh_adds_a_second_valid_slot() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    buf.update(1, &inter_update(), false);
    assert_eq!(valid_count(&buf), 2);
    assert!(buf.slots[0].valid);
    assert!(buf.slots[1].valid);
    assert_eq!(buf.slots[1].order_hint, 1);
    assert_eq!(buf.slots[1].order_hint_lsb, 1);
    assert!(buf.slots[1].implicit_output_frame);
    assert!(!buf.slots[1].immediate_output_frame);
    assert_eq!(buf.slots[1].base_q_idx, 109);
    assert_eq!(buf.slots[1].counter, 1);
    assert_eq!(buf.slots[1].delta_q_u_ac, 4);
    assert_eq!(buf.slots[1].delta_q_v_ac, -1);
    assert_eq!(buf.slots[1].frame_index, Some(1));
    assert!(buf.slots[1].is_inter);
}

#[test]
fn first_regular_frame_after_olk_keeps_only_olk_and_long_term_slots() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    let mut old = key_update();
    old.refresh_frame_flags = 1;
    buf.update(0, &old, true);

    let mut long_term = key_update();
    long_term.refresh_frame_flags = 1 << 3;
    long_term.long_term_id = Some(7);
    buf.update(1, &long_term, true);

    let mut olk = key_update();
    olk.refresh_frame_flags = 1 << 1;
    buf.update(2, &olk, true);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk);

    let mut leading = inter_update();
    leading.refresh_frame_flags = 1 << 2;
    buf.update(3, &leading, false);
    buf.note_frame(ObuType::LeadingTileGroup, false, &leading);

    buf.prepare_for_frame(ObuType::RegularTileGroup, true);

    assert!(!buf.slots[0].valid);
    assert!(buf.slots[1].valid);
    assert!(!buf.slots[2].valid);
    assert!(buf.slots[3].valid);
}

#[test]
fn olk_co_vcl_refresh_in_same_tu_survives_first_regular_tu() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    let mut olk = key_update();
    olk.refresh_frame_flags = 1 << 1;
    buf.update(0, &olk, true);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk);

    let mut co_vcl = inter_update();
    co_vcl.refresh_frame_flags = 1 << 4;
    buf.update(1, &co_vcl, false);
    buf.note_frame(ObuType::RegularTileGroup, false, &co_vcl);

    let mut leading = inter_update();
    leading.refresh_frame_flags = 1 << 2;
    buf.update(2, &leading, false);
    buf.note_frame(ObuType::LeadingTileGroup, false, &leading);

    buf.prepare_for_frame(ObuType::RegularTileGroup, true);

    assert!(buf.slots[1].valid);
    assert!(!buf.slots[2].valid);
    assert!(buf.slots[4].valid);
}

#[test]
fn regular_frames_after_olk_in_its_tu_allow_only_tu_refreshed_slots() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    let mut olk = key_update();
    olk.refresh_frame_flags = 1 << 1;
    buf.prepare_for_frame(ObuType::OpenLoopKey, true);
    assert_eq!(buf.allowed_frames, None);
    buf.update(0, &olk, true);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk);

    buf.prepare_for_frame(ObuType::RegularTileGroup, false);
    assert_eq!(buf.allowed_frames, Some(1 << 1));
    let mut co_vcl = inter_update();
    co_vcl.refresh_frame_flags = 1 << 4;
    buf.update(1, &co_vcl, false);
    buf.note_frame(ObuType::RegularTileGroup, false, &co_vcl);

    buf.prepare_for_frame(ObuType::RegularSef, false);
    assert_eq!(buf.allowed_frames, Some((1 << 1) | (1 << 4)));
    buf.prepare_for_frame(ObuType::LeadingTileGroup, false);
    assert_eq!(buf.allowed_frames, None);

    buf.prepare_for_frame(ObuType::RegularTileGroup, true);
    assert_eq!(buf.allowed_frames, None);
}

#[test]
fn closed_loop_key_clears_open_loop_state_and_allowed_frames() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    let mut old = key_update();
    old.refresh_frame_flags = 1;
    buf.update(0, &old, true);

    let mut olk = key_update();
    olk.refresh_frame_flags = 1 << 1;
    buf.update(1, &olk, true);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk);

    buf.prepare_for_frame(ObuType::RegularTileGroup, false);
    assert_eq!(buf.allowed_frames, Some(1 << 1));

    buf.prepare_for_frame(ObuType::ClosedLoopKey, true);
    assert_eq!(buf.allowed_frames, None);

    buf.prepare_for_frame(ObuType::RegularTileGroup, false);
    assert_eq!(buf.allowed_frames, None);

    buf.prepare_for_frame(ObuType::RegularTileGroup, true);
    assert!(buf.slots[0].valid);
}

#[test]
fn reference_refresh_preserves_full_and_lsb_order_hints() {
    let mut update = inter_update();
    update.order_hint = 136;
    update.order_hint_lsb = 8;
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    buf.update(1, &update, false);

    assert_eq!(buf.slots[1].order_hint, 136);
    assert_eq!(buf.slots[1].order_hint_lsb, 8);
    let frames = vec![Some(pipeline_frame(64, 64)), Some(pipeline_frame(64, 64))];
    let metadata = buf.state::<u8>(&frames.into()).unwrap();
    assert_eq!(metadata.ref_order_hint[1], 136);
    assert_eq!(metadata.ref_order_hint_lsbs[1], 8);
    assert!(metadata.ref_implicit_output_frame[1]);
}

#[test]
fn reference_refresh_preserves_global_motion_predictor_state() {
    let mut update = inter_update();
    update.num_total_refs = 2;
    update.saved_order_hints[..2].copy_from_slice(&[7, 11]);
    update.saved_gm_params[0] = [131_072, 65_536, 65_600, 256, -128, 65_728];
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    buf.update(1, &update, false);

    let frames = vec![Some(pipeline_frame(64, 64)), Some(pipeline_frame(64, 64))];
    let metadata = buf.state::<u8>(&frames.into()).unwrap();
    assert_eq!(metadata.ref_num_total_refs[1], 2);
    assert_eq!(metadata.saved_global_motion_order_hints[1][..2], [7, 11]);
    assert_eq!(
        metadata.saved_global_motion_params[1][0],
        [131_072, 65_536, 65_600, 256, -128, 65_728]
    );
}

#[test]
fn reference_products_come_from_the_retained_pipeline_frame() {
    let mut update = inter_update();
    update.refresh_frame_flags = 1 << 2;
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &update, false);

    let mut frame = pipeline_frame(64, 64);
    let frame_cdfs = Arc::new(crate::bitstream::tile_payload::FrameCdfSubset::from_defaults());
    frame.frame_cdfs = crate::prediction::inter::FrameCdfHandle::settled(Arc::clone(&frame_cdfs));

    let (_, header) =
        crate::prediction::inter::test_support::fixture_sequence_and_key_core(include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
        ));
    assert!(header.ccso_params.is_some());
    let ccso_header = Arc::new(header);
    frame.ccso_header = Some(Arc::clone(&ccso_header));

    let ccso_grid = Arc::new(
        crate::filters::ccso::CcsoUnitGrid::new(
            false,
            0,
            [false; 3],
            std::array::from_fn(|_| Vec::new()),
            0,
            0,
        )
        .unwrap(),
    );
    frame.ccso_grid =
        crate::prediction::inter::CcsoGridHandle::settled(Some(Arc::clone(&ccso_grid)));

    let segment_ids =
        Arc::new(crate::bitstream::tile_payload::FrameSegmentIdMap::new(16, 16).unwrap());
    frame.segment_ids =
        crate::prediction::inter::SegmentIdMapHandle::settled(Arc::clone(&segment_ids));

    let motion_field = crate::prediction::inter::TemporalMotionField::new(16, 16).unwrap();
    frame.motion_field = crate::prediction::inter::MotionFieldHandle::settled(motion_field);
    let motion_field = Arc::clone(frame.motion_field.field().unwrap());

    let frames = vec![Some(frame)];
    let metadata = buf.state::<u8>(&frames.into()).unwrap();

    let stored_cdfs = metadata.ref_frame_cdfs[2]
        .as_ref()
        .and_then(crate::prediction::inter::FrameCdfHandle::product)
        .unwrap();
    assert!(Arc::ptr_eq(stored_cdfs, &frame_cdfs));
    assert!(Arc::ptr_eq(
        metadata.ref_ccso_headers[2].as_ref().unwrap(),
        &ccso_header
    ));
    let stored_grid = metadata.ref_ccso_unit_grids[2]
        .as_ref()
        .and_then(crate::prediction::inter::CcsoGridHandle::product)
        .and_then(Option::as_ref)
        .unwrap();
    assert!(Arc::ptr_eq(stored_grid, &ccso_grid));
    let stored_segment_ids = metadata.ref_segment_ids[2]
        .as_ref()
        .and_then(crate::prediction::inter::SegmentIdMapHandle::product)
        .unwrap();
    assert!(Arc::ptr_eq(stored_segment_ids, &segment_ids));
    assert!(Arc::ptr_eq(
        metadata.ref_motion_fields[2]
            .as_ref()
            .and_then(crate::prediction::inter::MotionFieldHandle::field)
            .unwrap(),
        &motion_field
    ));
    assert!(metadata.ref_frame_cdfs[0].is_none());
    assert!(metadata.ref_ccso_headers[0].is_none());
    assert!(metadata.ref_ccso_unit_grids[0].is_none());
    assert!(metadata.ref_segment_ids[0].is_none());
    assert!(metadata.ref_motion_fields[0].is_none());
}

#[test]
fn frame_index_is_retained_until_last_slot_is_overwritten() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    let mut update = inter_update();
    update.refresh_frame_flags = (1 << 1) | (1 << 2);
    buf.update(1, &update, false);

    assert!(buf.retains(0));
    assert!(buf.retains(1));

    update.refresh_frame_flags = 1 << 1;
    buf.update(2, &update, false);
    assert!(buf.retains(1));

    update.refresh_frame_flags = 1 << 2;
    buf.update(3, &update, false);
    assert!(!buf.retains(1));
}

#[test]
fn zero_slots_rejected() {
    let Err(error) = RuntimeReferenceBuffer::new(0) else {
        panic!("zero reference slots must be rejected");
    };

    assert!(matches!(
        error,
        crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::InvalidReferenceStoreCapacity {
                capacity: 0,
                max_slots: ReferenceSlot::MAX_SLOTS
            }
        }
    ));
}

#[test]
fn frame_index_lookup_rejects_out_of_range_and_missing_slots() {
    let buf = RuntimeReferenceBuffer::new(8).unwrap();

    assert!(matches!(
        buf.frame_index_for_slot(8),
        Err(crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::SlotOutOfRange {
                slot: 8,
                slot_count: 8,
            },
        })
    ));
    assert!(matches!(
        buf.frame_index_for_slot(0),
        Err(crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot: 0 },
        })
    ));
}

#[test]
fn valid_slot_without_frame_index_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.slots[0].valid = true;

    let Err(error) = buf.state::<u8>(&Vec::new().into()) else {
        panic!("missing decoded-frame index must be rejected");
    };

    assert!(matches!(
        error,
        crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot: 0 },
        }
    ));
}

#[test]
fn valid_slot_with_out_of_range_frame_index_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.slots[0].valid = true;
    buf.slots[0].frame_index = Some(3);

    let Err(error) = buf.state::<u8>(&Vec::new().into()) else {
        panic!("out-of-range decoded-frame index must be rejected");
    };

    assert!(matches!(
        error,
        crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::FrameIndexOutOfRange {
                slot: 0,
                frame_index: 3,
                frame_count: 0,
            },
        }
    ));
}

#[test]
fn valid_slot_with_missing_retained_frame_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);

    let Err(error) = buf.state::<u8>(&vec![None].into()) else {
        panic!("missing retained frame must be rejected");
    };

    assert!(matches!(
        error,
        crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::MissingFrame { slot: 0 },
        }
    ));
}

#[test]
fn valid_slot_with_mismatched_storage_depth_keeps_its_diagnostic() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    let frames = vec![Some(pipeline_frame(64, 64))];

    let Err(error) = buf.state::<u16>(&frames.into()) else {
        panic!("8-bit retained frame must not enter a 10-bit reference store");
    };

    assert!(matches!(
        error,
        crate::DecodeError::UnsupportedFeature { unsupported }
            if unsupported.reason() == "unsupported_8bit_reference_for_10bit_decode"
    ));
}

#[test]
fn valid_slot_with_mismatched_frame_size_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    let frames = vec![Some(pipeline_frame(32, 64))];

    let Err(error) = buf.state::<u8>(&frames.into()) else {
        panic!("mismatched retained-frame size must be rejected");
    };

    assert!(matches!(
        error,
        crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::FrameSizeMismatch {
                slot: 0,
                frame_index: 0,
                expected_width: 64,
                expected_height: 64,
                actual_width: 32,
                actual_height: 64,
            },
        }
    ));
}

#[test]
fn sef_derive_requires_a_hidden_not_previously_shown_reference() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    assert!(matches!(
        buf.mark_sef_derive_output(0, false),
        Err(crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ShowExistingFrameIneligible { slot: 0 }
        })
    ));

    let mut hidden = inter_update();
    hidden.refresh_frame_flags = 1 << 1;
    hidden.implicit_output_frame = false;
    hidden.immediate_output_frame = false;
    buf.update(1, &hidden, false);
    buf.mark_sef_derive_output(1, false).unwrap();
    assert!(matches!(
        buf.mark_sef_derive_output(1, false),
        Err(crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ShowExistingFrameIneligible { slot: 1 }
        })
    ));
}

#[test]
fn show_existing_advances_the_reference_frame_counter() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    buf.note_show_existing();
    let update = inter_update();
    buf.update(1, &update, false);
    assert_eq!(buf.slots[1].counter, 2);
}

#[test]
fn restricted_switch_marks_dependency_layer_order_hints() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    let current = splot_core::types::EmbeddedLayerId::from_bits(0);
    let presence =
        splot_core::headers::sequence::MLayerDependencyMap::default_for(current).presence_map();
    let restricted = buf.restrict_references_for_switch(current, &presence);
    assert_eq!(restricted, (0..8).collect::<Vec<_>>());
    assert!(buf.slots.iter().all(|slot| slot.order_hint == u32::MAX));
}

#[test]
fn a_held_snapshot_keeps_its_frame_filter_taps_across_a_refresh() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    assert!(buf.lr_frame_filter_taps.is_none());
    let coeffs = |tap: i16| InlineVec::from_iter_checked([tap]).unwrap();
    let banks = [5, 9, 7].map(|tap| WienerNsFrameFilterBank {
        classes: InlineVec::from_iter_checked([
            splot_core::headers::frame::WienerNsFrameFilterClass {
                coeffs: coeffs(tap),
                ..Default::default()
            },
        ])
        .unwrap(),
    });
    let mut update = inter_update();
    update.lr_banks[0] = Some(&banks[0]);
    buf.update(1, &update, false);
    let frames = (0..=4)
        .map(|_| Some(pipeline_frame(64, 64)))
        .collect::<Vec<_>>()
        .into();
    let held = buf.state::<u8>(&frames).unwrap();

    update.lr_banks[0] = Some(&banks[1]);
    buf.update(2, &update, false);
    let fresh = buf.state::<u8>(&frames).unwrap();

    let taps = |meta: &InterReferenceState<u8>, slot: usize| {
        meta.lr_frame_filter_taps.as_ref().unwrap()[slot]
    };
    assert_eq!(taps(&held, 1)[0][0], coeffs(5));
    assert_eq!(taps(&fresh, 1)[0][0], coeffs(9));
    assert!(taps(&fresh, 0)[0].is_empty());

    drop(held);
    update.lr_banks[0] = Some(&banks[2]);
    buf.update(3, &update, false);
    let reused = buf.state::<u8>(&frames).unwrap();
    assert_eq!(taps(&fresh, 1)[0][0], coeffs(9));
    assert_eq!(taps(&reused, 1)[0][0], coeffs(7));
    assert_eq!(buf.lr_frame_filter_spares.len(), 1);

    update.lr_banks[0] = None;
    buf.update(4, &update, false);
    let cleared = buf.state::<u8>(&frames).unwrap();
    assert!(taps(&cleared, 1)[0].is_empty());
    assert_eq!(taps(&reused, 1)[0][0], coeffs(7));
}

fn unit_grid(seed: u8) -> Arc<crate::filters::ccso::CcsoUnitGrid> {
    let (rows, cols) = (2, 3);
    let blocks = std::array::from_fn(|plane| {
        (0..rows * cols)
            .map(|cell| seed.wrapping_add(plane as u8 * 7).wrapping_add(cell as u8))
            .collect()
    });
    Arc::new(
        crate::filters::ccso::CcsoUnitGrid::new(true, 2, [true; 3], blocks, rows, cols).unwrap(),
    )
}

struct CcsoFrame {
    header: Arc<splot_core::headers::frame::FrameHeaderCore>,
    grid: Arc<crate::filters::ccso::CcsoUnitGrid>,
}

fn ccso_frame(
    header: &splot_core::headers::frame::FrameHeaderCore,
    seed: u8,
) -> (PipelineFrame, CcsoFrame) {
    let mut header = header.clone();
    for plane in &mut header.ccso_params.as_mut().unwrap().planes {
        plane.ccso_scale_idx = Some(seed & 3);
    }
    let owned = CcsoFrame {
        header: Arc::new(header),
        grid: unit_grid(seed),
    };
    let mut frame = pipeline_frame(64, 64);
    frame.ccso_header = Some(Arc::clone(&owned.header));
    frame.ccso_grid =
        crate::prediction::inter::CcsoGridHandle::settled(Some(Arc::clone(&owned.grid)));
    (frame, owned)
}

fn reuse_header(
    base: &splot_core::headers::frame::FrameHeaderCore,
    ref_frame_idx: [u32; 3],
    plane_refs: [Option<u32>; 3],
) -> splot_core::headers::frame::FrameHeaderCore {
    let mut core = base.clone();
    let mut inter = splot_core::headers::frame::InterControl::default();
    inter.ref_frame_idx = ref_frame_idx.into_iter().collect();
    core.inter = Some(inter);
    let ccso = core.ccso_params.as_mut().unwrap();
    ccso.ccso_frame_flag = Some(true);
    ccso.planes = splot_core::tile::InlineVec::from_iter_checked(plane_refs.map(|reference| {
        splot_core::headers::frame::CcsoPlaneParams {
            ccso_planes: true,
            sb_reuse_ccso: reference.is_some(),
            ccso_ref_idx: reference,
            ..Default::default()
        }
    }))
    .unwrap();
    core
}

fn reused_product(
    handle: Option<&crate::prediction::inter::CcsoGridHandle>,
) -> Option<&Arc<crate::filters::ccso::CcsoUnitGrid>> {
    handle
        .and_then(crate::prediction::inter::CcsoGridHandle::product)
        .and_then(Option::as_ref)
}

const CCSO_FIXTURE: &[u8] = include_bytes!(
    "../../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
);

fn ccso_sequence_and_core() -> (
    splot_core::headers::sequence::SequenceHeader,
    splot_core::headers::frame::FrameHeaderCore,
) {
    let (mut sequence, core) =
        crate::prediction::inter::test_support::fixture_sequence_and_key_core(CCSO_FIXTURE);
    sequence.filter.as_mut().unwrap().enable_ccso = true;
    (sequence, core)
}

#[test]
fn sb_reuse_resolves_the_referenced_frames_grid_after_a_key_frame_is_shown_by_sef() {
    let (sequence, base) = ccso_sequence_and_core();
    let (key_frame, key) = ccso_frame(&base, 1);
    let (inter_frame, inter) = ccso_frame(&base, 2);

    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update(), true);
    let mut update = inter_update();
    update.refresh_frame_flags = 1 << 1;
    buf.update(1, &update, false);

    let mut sef_frame = pipeline_frame(64, 64);
    sef_frame.ccso_header.clone_from(&key_frame.ccso_header);
    sef_frame.ccso_grid = key_frame.ccso_grid.clone();
    let frames: FrameStore = vec![Some(key_frame), Some(inter_frame), Some(sef_frame)].into();
    let reuse = reuse_header(&base, [1, 0, 0], [Some(0), Some(1), None]);
    let before = buf.state::<u8>(&frames).unwrap();
    let grids_before = crate::prediction::inter::reused_ccso_grids(&reuse, &sequence, &before);

    buf.note_show_existing();
    let after = buf.state::<u8>(&frames).unwrap();
    let grids_after = crate::prediction::inter::reused_ccso_grids(&reuse, &sequence, &after);

    assert_eq!(after.ref_valid.iter().filter(|valid| **valid).count(), 2);
    assert!(Arc::ptr_eq(
        after.ref_ccso_headers[0].as_ref().unwrap(),
        &key.header
    ));
    assert!(Arc::ptr_eq(
        after.ref_ccso_headers[1].as_ref().unwrap(),
        &inter.header
    ));
    assert!(Arc::ptr_eq(
        reused_product(grids_after[0].as_ref()).unwrap(),
        &inter.grid
    ));
    assert!(Arc::ptr_eq(
        reused_product(grids_after[1].as_ref()).unwrap(),
        &key.grid
    ));
    assert!(grids_after[2].is_none());
    for (plane, expected) in [(0, &inter.grid), (1, &key.grid)] {
        let reused = reused_product(grids_after[plane].as_ref()).unwrap();
        assert_eq!(**reused, **expected);
        assert_eq!(reused.plane_blocks(plane), expected.plane_blocks(plane));
        assert!(Arc::ptr_eq(
            reused_product(grids_before[plane].as_ref()).unwrap(),
            reused
        ));
    }
    assert_ne!(*key.grid, *inter.grid);
}

#[test]
fn sb_reuse_cannot_reach_an_old_sequences_grid_after_a_new_sequence_starts() {
    let (sequence, base) = ccso_sequence_and_core();
    let (old_key_frame, old_key) = ccso_frame(&base, 1);
    let (old_inter_frame, old_inter) = ccso_frame(&base, 2);
    let (new_key_frame, new_key) = ccso_frame(&base, 3);

    let mut old = RuntimeReferenceBuffer::new(8).unwrap();
    old.update(0, &key_update(), true);
    let mut update = inter_update();
    update.refresh_frame_flags = 1 << 1;
    old.update(1, &update, false);
    let frames: FrameStore = vec![
        Some(old_key_frame),
        Some(old_inter_frame),
        Some(new_key_frame),
    ]
    .into();
    let reuse = reuse_header(&base, [1, 0, 0], [Some(0), Some(1), Some(2)]);
    let populated = old.state::<u8>(&frames).unwrap();
    let grids = crate::prediction::inter::reused_ccso_grids(&reuse, &sequence, &populated);
    assert!(Arc::ptr_eq(
        reused_product(grids[0].as_ref()).unwrap(),
        &old_inter.grid
    ));
    assert!(Arc::ptr_eq(
        reused_product(grids[1].as_ref()).unwrap(),
        &old_key.grid
    ));

    let mut fresh = RuntimeReferenceBuffer::new(8).unwrap();
    fresh.update(2, &key_update(), true);
    let state = fresh.state::<u8>(&frames).unwrap();
    let grids = crate::prediction::inter::reused_ccso_grids(&reuse, &sequence, &state);

    assert!(!fresh.retains(0) && !fresh.retains(1) && fresh.retains(2));
    assert!(Arc::ptr_eq(
        state.ref_ccso_headers[0].as_ref().unwrap(),
        &new_key.header
    ));
    assert!(state.ref_ccso_headers[1..].iter().all(Option::is_none));
    assert!(state.ref_ccso_unit_grids[1..].iter().all(Option::is_none));
    assert!(
        grids[0].is_none(),
        "slot 1 must not resolve to the old inter frame"
    );
    assert!(Arc::ptr_eq(
        reused_product(grids[1].as_ref()).unwrap(),
        &new_key.grid
    ));
    assert!(Arc::ptr_eq(
        reused_product(grids[2].as_ref()).unwrap(),
        &new_key.grid
    ));
    for grid in grids
        .iter()
        .filter_map(|handle| reused_product(handle.as_ref()))
    {
        assert!(!Arc::ptr_eq(grid, &old_key.grid) && !Arc::ptr_eq(grid, &old_inter.grid));
    }
}
