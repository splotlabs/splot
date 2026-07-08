// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn key_update() -> FrameRefUpdate {
    FrameRefUpdate {
        refresh_frame_flags: 0xFF,
        order_hint: 0,
        width: 64,
        height: 64,
        base_q_idx: 70,
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
        width: 64,
        height: 64,
        base_q_idx: 109,
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

#[test]
fn key_refresh_marks_only_first_slot_valid() {
    let mut buf = RuntimeReferenceBuffer::new(8).unwrap();
    buf.update(0, &key_update());
    assert_eq!(valid_count(&buf), 1);
    assert!(buf.slots[0].valid);
    assert!(!buf.slots[1].valid);
    assert_eq!(buf.slots[0].base_q_idx, 70);
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
    assert_eq!(buf.slots[1].base_q_idx, 109);
    assert_eq!(buf.slots[1].frame_index, Some(1));
    assert!(buf.slots[1].is_inter);
    assert!(!buf.slots[1].adapted);
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
fn zero_slots_rejected() {
    assert!(RuntimeReferenceBuffer::new(0).is_err());
}
