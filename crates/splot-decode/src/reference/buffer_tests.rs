// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::pipeline::{PipelineDecodedFrame, PipelineFrame};
use splot_recon::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane,
    PlaneRect, PlaneSize,
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
        lr_frame_filter_class_counts: [1, 0, 0],
        lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
        frame_cdfs: FrameCdfSubset::from_defaults(),
        ccso_params: None,
        ccso_grid: None,
        motion_field: TemporalMotionField::empty(),
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
        lr_frame_filter_class_counts: [0, 0, 0],
        lr_frame_filter_taps: [Vec::new(), Vec::new(), Vec::new()],
        frame_cdfs: FrameCdfSubset::from_defaults(),
        ccso_params: None,
        ccso_grid: None,
        motion_field: TemporalMotionField::empty(),
    }
}

fn valid_count(buf: &RuntimeReferenceBuffer) -> usize {
    buf.slots.iter().filter(|s| s.valid).count()
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
        frame: PipelineDecodedFrame::Eight(decoded_frame(width, height)),
        display_grain: None,
        frame_cdfs: FrameCdfSubset::from_defaults(),
        motion_field: TemporalMotionField::empty(),
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
