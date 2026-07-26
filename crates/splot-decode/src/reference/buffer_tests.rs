// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::pipeline::{PipelineDecodedFrame, PipelineFrame};
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane,
    PlaneRect, PlaneSize, SharedFrame,
};

fn key_update() -> FrameRefUpdate {
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
        is_key_or_switch: true,
        is_inter: false,
        adapted: false,
        num_total_refs: 0,
        saved_order_hints: [0; 7],
        saved_gm_params: [GlobalMotionRef::identity().gm_params; 7],
        lr_frame_filter_class_counts: [1, 0, 0],
        lr_frame_filter_taps: None,
        frame_cdfs: Arc::new(FrameCdfSubset::from_defaults()),
        ccso_params: None,
        ccso_grid: None,
        motion_field: Arc::new(TemporalMotionField::empty()),
        long_term_id: None,
        embedded_layer_id: splot_core::types::EmbeddedLayerId::from_bits(0),
    }
}

fn inter_update(adapted: bool) -> FrameRefUpdate {
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
        is_key_or_switch: false,
        is_inter: true,
        adapted,
        num_total_refs: 1,
        saved_order_hints: [0; 7],
        saved_gm_params: [GlobalMotionRef::identity().gm_params; 7],
        lr_frame_filter_class_counts: [0, 0, 0],
        lr_frame_filter_taps: None,
        frame_cdfs: Arc::new(FrameCdfSubset::from_defaults()),
        ccso_params: None,
        ccso_grid: None,
        motion_field: Arc::new(TemporalMotionField::empty()),
        long_term_id: None,
        embedded_layer_id: splot_core::types::EmbeddedLayerId::from_bits(0),
    }
}

fn valid_count(buf: &RuntimeReferenceBuffer) -> usize {
    buf.slots.iter().filter(|s| s.valid).count()
}

#[test]
fn bridge_overwrite_marks_every_refreshed_slot_valid() {
    let mut update = key_update();
    update.refresh_frame_flags = (1 << 2) | (1 << 5);
    update.is_key_or_switch = false;
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();

    buf.update(0, &update);

    assert!(buf.slots[2].valid);
    assert!(buf.slots[5].valid);
    assert_eq!(valid_count(&buf), 2);
}

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

fn pipeline_frame(width: usize, height: usize) -> PipelineFrame {
    PipelineFrame {
        frame: crate::pipeline::inflight::PipelineFrameSlot::completed(
            PipelineDecodedFrame::Eight(SharedFrame::new(decoded_frame(width, height))),
        ),
        display_grain: None,
        output_effects: crate::pipeline::output_effects::FrameOutputEffects::empty(),
        frame_cdfs: Arc::new(FrameCdfSubset::from_defaults()),
        motion_field: Arc::new(TemporalMotionField::empty()),
        ccso_params: None,
        ccso_grid: None,
        frame_rate_numerator: 1,
        frame_rate_denominator: 1,
    }
}

#[test]
fn key_refresh_marks_only_first_slot_valid() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
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
    buf.update(0, &key_update());
    buf.update(1, &inter_update(false));
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
    assert!(!buf.slots[1].adapted);
}

#[test]
fn first_regular_frame_after_olk_keeps_only_olk_and_listed_long_term_slots() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    let mut old = key_update();
    old.refresh_frame_flags = 1;
    buf.update(0, &old);

    let mut long_term = key_update();
    long_term.refresh_frame_flags = 1 << 3;
    long_term.long_term_id = Some(7);
    buf.update(1, &long_term);

    let mut olk = key_update();
    olk.refresh_frame_flags = 1 << 1;
    buf.update(2, &olk);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk, &[7]);

    let mut leading = inter_update(false);
    leading.refresh_frame_flags = 1 << 2;
    buf.update(3, &leading);
    buf.note_frame(ObuType::LeadingTileGroup, false, &leading, &[]);

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
    buf.update(0, &olk);
    buf.note_frame(ObuType::OpenLoopKey, true, &olk, &[]);

    let mut co_vcl = inter_update(false);
    co_vcl.refresh_frame_flags = 1 << 4;
    buf.update(1, &co_vcl);
    buf.note_frame(ObuType::RegularTileGroup, false, &co_vcl, &[]);

    let mut leading = inter_update(false);
    leading.refresh_frame_flags = 1 << 2;
    buf.update(2, &leading);
    buf.note_frame(ObuType::LeadingTileGroup, false, &leading, &[]);

    buf.prepare_for_frame(ObuType::RegularTileGroup, true);

    assert!(buf.slots[1].valid);
    assert!(!buf.slots[2].valid);
    assert!(buf.slots[4].valid);
}

#[test]
fn reference_refresh_preserves_full_and_lsb_order_hints() {
    let mut update = inter_update(false);
    update.order_hint = 136;
    update.order_hint_lsb = 8;
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    buf.update(1, &update);

    assert_eq!(buf.slots[1].order_hint, 136);
    assert_eq!(buf.slots[1].order_hint_lsb, 8);
    let frames = vec![Some(pipeline_frame(64, 64)), Some(pipeline_frame(64, 64))];
    let metadata = buf.build_store_eight(&frames).unwrap().1;
    assert_eq!(metadata.ref_order_hint[1], 136);
    assert_eq!(metadata.ref_order_hint_lsbs[1], 8);
    assert!(metadata.ref_implicit_output_frame[1]);
}

#[test]
fn reference_refresh_preserves_global_motion_predictor_state() {
    let mut update = inter_update(false);
    update.num_total_refs = 2;
    update.saved_order_hints[..2].copy_from_slice(&[7, 11]);
    update.saved_gm_params[0] = [131_072, 65_536, 65_600, 256, -128, 65_728];
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    buf.update(1, &update);

    let frames = vec![Some(pipeline_frame(64, 64)), Some(pipeline_frame(64, 64))];
    let metadata = buf.build_store_eight(&frames).unwrap().1;
    assert_eq!(metadata.ref_num_total_refs[1], 2);
    assert_eq!(metadata.saved_global_motion_order_hints[1][..2], [7, 11]);
    assert_eq!(
        metadata.saved_global_motion_params[1][0],
        [131_072, 65_536, 65_600, 256, -128, 65_728]
    );
}

#[test]
fn per_slot_adaptation_is_tracked_independently() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    buf.update(1, &inter_update(true));
    assert!(!buf.slots[0].adapted);
    assert!(buf.slots[1].adapted);
    assert!(!buf.slots[0].is_inter);
    assert!(buf.slots[1].is_inter);
}

#[test]
fn frame_index_is_retained_until_last_slot_is_overwritten() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    let mut update = inter_update(false);
    update.refresh_frame_flags = (1 << 1) | (1 << 2);
    buf.update(1, &update);

    assert!(buf.retains(0));
    assert!(buf.retains(1));

    update.refresh_frame_flags = 1 << 1;
    buf.update(2, &update);
    assert!(buf.retains(1));

    update.refresh_frame_flags = 1 << 2;
    buf.update(3, &update);
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
fn valid_slot_without_frame_index_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.slots[0].valid = true;

    let Err(error) = buf.build_store_eight(&[]) else {
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

    let Err(error) = buf.build_store_eight(&[]) else {
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
fn valid_slot_with_mismatched_frame_size_is_reference_state_error() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    let frames = vec![Some(pipeline_frame(32, 64))];

    let Err(error) = buf.build_store_eight(&frames) else {
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
    buf.update(0, &key_update());
    assert!(matches!(
        buf.mark_sef_derive_output(0, false),
        Err(crate::DecodeError::ReferenceState {
            source: crate::DecodeReferenceStateError::ShowExistingFrameIneligible { slot: 0 }
        })
    ));

    let mut hidden = inter_update(false);
    hidden.refresh_frame_flags = 1 << 1;
    hidden.implicit_output_frame = false;
    hidden.immediate_output_frame = false;
    buf.update(1, &hidden);
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
    buf.update(0, &key_update());
    buf.note_show_existing();
    let update = inter_update(false);
    buf.update(1, &update);
    assert_eq!(buf.slots[1].counter, 2);
}

#[test]
fn restricted_switch_marks_dependency_layer_order_hints() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    let current = splot_core::types::EmbeddedLayerId::from_bits(0);
    let presence =
        splot_core::headers::sequence::MLayerDependencyMap::default_for(current).presence_map();
    let restricted = buf.restrict_references_for_switch(current, &presence);
    assert_eq!(restricted, (0..8).collect::<Vec<_>>());
    assert!(buf.slots.iter().all(|slot| slot.order_hint == u32::MAX));
}
