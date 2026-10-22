// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

/// Plans `frames` input frames that arrive one at a time.
fn run(keyint: u32, mini_gop_size: u8, frames: u64) -> Vec<GopStep> {
    let mut planner = GopPlanner::new(NonZeroU32::new(keyint).unwrap(), mini_gop_size);
    let mut steps = Vec::new();
    let mut queued = 0;
    let mut drain = |queued: &mut usize, ended: bool| {
        while let Some(len) = planner.next_len(*queued, ended) {
            steps.extend(planner.plan(len).unwrap());
            *queued -= len;
        }
    };
    for _ in 0..frames {
        queued += 1;
        drain(&mut queued, false);
    }
    drain(&mut queued, true);
    steps
}

/// Replays `steps` on a model of the reference slots and checks the plan.
fn check(keyint: u32, mini_gop_size: u8, frames: u64) {
    let case = format!("keyint {keyint} mini-GOP {mini_gop_size} frames {frames}");
    let steps = run(keyint, mini_gop_size, frames);
    let mut slots: [Option<u64>; REF_SLOTS] = [None; REF_SLOTS];
    let (mut hidden, mut coded, mut outputs) = (Vec::new(), Vec::new(), Vec::new());
    let (mut coded_count, mut key_display, mut after_output) = (0, 0, true);
    for step in &steps {
        match *step {
            GopStep::Code(frame) => {
                let display = frame.display_index;
                assert_eq!(frame.frame_index, coded_count, "{case}");
                assert_eq!(frame.opens_temporal_unit, after_output, "{case}");
                assert_eq!(
                    frame.frame_type == FrameType::Key,
                    display % u64::from(keyint) == 0,
                    "{case}: frame {display}"
                );
                if frame.frame_type == FrameType::Key {
                    assert!(frame.show_frame && hidden.is_empty(), "{case}");
                    assert_eq!(frame.refresh_frame_flags, 0xff, "{case}");
                    (key_display, slots) = (display, [None; REF_SLOTS]);
                    slots[0] = Some(display);
                    coded.clear();
                } else {
                    let valid = (0..REF_SLOTS)
                        .filter(|&slot| slots[slot].is_some())
                        .fold(0, |mask, slot| mask | 1 << slot);
                    assert_eq!(frame.ref_slots, valid, "{case}: frame {display}");
                    let closest_past = coded.iter().filter(|&&d| d < display).max();
                    let closest_future = coded.iter().filter(|&&d| d > display).min();
                    assert!(closest_past.is_some(), "{case}: frame {display}");
                    for closest in [closest_past, closest_future].into_iter().flatten() {
                        assert!(
                            slots.contains(&Some(*closest)),
                            "{case}: frame {display} lost reference {closest}"
                        );
                    }
                    assert_eq!(frame.refresh_frame_flags.count_ones(), 1, "{case}");
                    let slot = frame.refresh_frame_flags.trailing_zeros() as usize;
                    if let Some(evicted) = slots[slot] {
                        assert!(
                            !hidden.contains(&evicted),
                            "{case}: frame {display} evicts hidden frame {evicted}"
                        );
                    }
                    slots[slot] = Some(display);
                }
                assert_eq!(u64::from(frame.order_hint), display - key_display, "{case}");
                coded.push(display);
                if !frame.show_frame {
                    hidden.push(display);
                }
                coded_count += 1;
            }
            GopStep::ShowExisting {
                slot,
                display_index,
            } => {
                assert!(after_output, "{case}: a show-existing frame opens its unit");
                assert_eq!(slots[slot as usize], Some(display_index), "{case}");
                let at = hidden.iter().position(|&d| d == display_index);
                assert!(at.is_some(), "{case}: {display_index} is not waiting");
                hidden.retain(|&d| d != display_index);
            }
        }
        if let Some(display) = step.output() {
            outputs.push(display);
        }
        after_output = step.output().is_some();
    }
    assert!(hidden.is_empty() && after_output, "{case}");
    assert_eq!(outputs, (0..frames).collect::<Vec<_>>(), "{case}");
    assert_eq!(coded_count, frames, "{case}");
}

#[test]
fn every_plan_shows_frames_in_input_order_from_the_right_slots() {
    for keyint in [1, 2, 3, 5, 7, 8, 9, 16, 17, 30] {
        for mini_gop_size in 1..=8 {
            for frames in 1..=40 {
                check(keyint, mini_gop_size, frames);
            }
        }
    }
}

#[test]
fn plans_a_hierarchical_mini_gop_and_a_short_last_one() {
    let order: Vec<_> = run(100, 4, 7)
        .iter()
        .map(|step| match *step {
            GopStep::Code(frame) => (frame.display_index, frame.show_frame),
            GopStep::ShowExisting { display_index, .. } => (display_index, true),
        })
        .collect();
    assert_eq!(
        order,
        [
            (0, true),
            (4, false),
            (2, false),
            (1, true),
            (2, true),
            (3, true),
            (4, true),
            (6, false),
            (5, true),
            (6, true),
        ]
    );
}

#[test]
fn waits_for_a_full_mini_gop_unless_the_input_ended() {
    let mut planner = GopPlanner::new(NonZeroU32::new(10).unwrap(), 4);
    assert_eq!(planner.next_len(0, true), None);
    assert_eq!(planner.next_len(1, false), Some(1));
    planner.plan(1).unwrap();
    assert_eq!(planner.next_len(3, false), None);
    assert_eq!(planner.next_len(3, true), Some(3));
    assert_eq!(planner.next_len(9, false), Some(4));
    planner.plan(4).unwrap();
    planner.plan(4).unwrap();
    assert_eq!(planner.next_display(), 9);
    assert_eq!(
        planner.next_len(5, false),
        Some(1),
        "stops before the key frame"
    );
}

#[test]
fn plans_ippp_with_key_frames_every_keyint() {
    let keyint = NonZeroU32::MIN.saturating_add(2);
    let frames: Vec<_> = (0..7)
        .map(|index| {
            let frame = plan(index, keyint);
            (frame.frame_type, frame.order_hint, frame.show_frame)
        })
        .collect();
    assert_eq!(
        frames,
        [
            (FrameType::Key, 0, true),
            (FrameType::Inter, 1, true),
            (FrameType::Inter, 2, true),
            (FrameType::Key, 0, true),
            (FrameType::Inter, 1, true),
            (FrameType::Inter, 2, true),
            (FrameType::Key, 0, true),
        ]
    );
    assert!((0..5).all(|index| plan(index, NonZeroU32::MIN).frame_type == FrameType::Key));
}

#[test]
fn scene_cuts_start_key_frames_and_restart_the_keyint_count() {
    let mut planner = GopPlanner::new(NonZeroU32::new(10).unwrap(), 4);
    planner.force_key(6);
    let mut coded = Vec::new();
    for _ in 0..8 {
        let len = planner.next_len(100, false).unwrap();
        for step in planner.plan(len).unwrap() {
            if let GopStep::Code(frame) = step {
                coded.push((frame.display_index, frame.frame_type, frame.layer));
            }
        }
    }
    let keys: Vec<u64> = coded
        .iter()
        .filter(|frame| frame.1 == FrameType::Key)
        .map(|frame| frame.0)
        .collect();
    assert_eq!(keys, [0, 6, 16]);
    let layers: Vec<(u64, u8)> = coded[1..5].iter().map(|f| (f.0, f.2)).collect();
    assert_eq!(layers, [(4, 0), (2, 1), (1, 2), (3, 2)]);
    planner.force_key(3);
    assert_eq!(
        planner.next_key(),
        planner.scheduled_key,
        "a planned cut is ignored"
    );
}
