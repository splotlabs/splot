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
    let (mut hidden, mut refs, mut outputs) = (Vec::new(), Vec::new(), Vec::new());
    let (mut coded_count, mut key_display, mut after_output) = (0, 0, true);
    let mut shown = Vec::new();
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
                    refs.clear();
                } else {
                    let valid = (0..REF_SLOTS)
                        .filter(|&slot| slots[slot].is_some())
                        .fold(0, |mask, slot| mask | 1 << slot);
                    assert_eq!(frame.ref_slots, valid, "{case}: frame {display}");
                    let closest_past = refs.iter().filter(|&&d| d < display).max();
                    let closest_future = refs.iter().filter(|&&d| d > display).min();
                    assert!(closest_past.is_some(), "{case}: frame {display}");
                    for closest in [closest_past, closest_future].into_iter().flatten() {
                        assert!(
                            slots.contains(&Some(*closest)),
                            "{case}: frame {display} lost reference {closest}"
                        );
                    }
                    let newest_shown = slots.iter().flatten().filter(|d| shown.contains(*d)).max();
                    assert!(
                        newest_shown.is_some_and(|&d| display.abs_diff(d) < 128),
                        "{case}: frame {display} cannot derive its OrderHint"
                    );
                    let leaf = frame.show_frame && frame.layer > 0;
                    let refreshed = frame.refresh_frame_flags.count_ones();
                    assert!(
                        refreshed == 1 || leaf && refreshed == 0,
                        "{case}: {display}"
                    );
                    let slot = frame.refresh_frame_flags.trailing_zeros() as usize;
                    if let Some(evicted) = slots.get(slot).copied().flatten() {
                        assert!(
                            !hidden.contains(&evicted),
                            "{case}: frame {display} evicts hidden frame {evicted}"
                        );
                    }
                    if refreshed == 1 {
                        slots[slot] = Some(display);
                    }
                }
                assert_eq!(u64::from(frame.order_hint), display - key_display, "{case}");
                if frame.refresh_frame_flags != 0 {
                    refs.push(display);
                }
                if frame.show_frame {
                    shown.push(display);
                } else {
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
    for mini_gop_size in 1..=8 {
        check(600, mini_gop_size, 520);
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
    planner.force_key(34);
    planner.force_key(37);
    planner.force_key(41);
    assert_eq!(planner.cuts, [34, 41], "a cut within a mini-GOP of a key");
    planner.force_key(52);
    assert_eq!(planner.cuts, [34, 41], "a cut 1 after the scheduled key 51");
}

/// The display indices of the coded frames of each wave with coded frames of
/// the mini-GOP after the key frame, and the display indices of its non-reference frames.
fn mini_gop_waves(mini_gop_size: u8) -> (Vec<Vec<u64>>, Vec<u64>) {
    let mut planner = GopPlanner::new(NonZeroU32::new(100).unwrap(), mini_gop_size);
    planner.plan(1).unwrap();
    let steps = planner.plan(usize::from(mini_gop_size)).unwrap();
    let frames = |wave: &[GopStep]| -> Vec<GopFrame> {
        wave.iter()
            .filter_map(|step| match *step {
                GopStep::Code(frame) => Some(frame),
                GopStep::ShowExisting { .. } => None,
            })
            .collect()
    };
    let leaves = frames(&steps)
        .into_iter()
        .filter(|frame| frame.refresh_frame_flags == 0)
        .map(|frame| {
            assert!(frame.show_frame && frame.layer > 0, "{frame:?}");
            frame.display_index
        })
        .collect();
    let waves = waves(&steps)
        .map(|wave| frames(wave).iter().map(|f| f.display_index).collect())
        .filter(|wave: &Vec<u64>| !wave.is_empty())
        .collect();
    (waves, leaves)
}

#[test]
fn leaf_frames_refresh_no_slot_and_code_in_waves() {
    let (waves, leaves) = mini_gop_waves(4);
    assert_eq!(waves, [vec![4], vec![2], vec![1, 3]]);
    assert_eq!(leaves, [1]);
    let (waves, leaves) = mini_gop_waves(8);
    assert_eq!(
        waves,
        [vec![8], vec![4], vec![2], vec![1, 3, 6], vec![5, 7]]
    );
    assert_eq!(leaves, [1, 3, 5]);
    let (waves, leaves) = mini_gop_waves(1);
    assert_eq!(waves, [vec![1]]);
    assert!(leaves.is_empty());
}

/// Each inter frame's primary reference is at or below its layer: the closest
/// layer, an inter frame over the key frame, then the closest display index;
/// key frames have none.
#[test]
fn primary_reference_is_the_closest_layer_at_or_below_the_frame() {
    let mut slots: [Option<u64>; REF_SLOTS] = [None; REF_SLOTS];
    let mut primaries = Vec::new();
    for step in run(100, 4, 7) {
        if let GopStep::Code(frame) = step {
            let primary = frame.primary_slot.map(|slot| slots[slot as usize].unwrap());
            primaries.push((frame.display_index, primary));
            for (slot, held) in slots.iter_mut().enumerate() {
                if frame.refresh_frame_flags & 1 << slot != 0 {
                    *held = Some(frame.display_index);
                }
            }
        }
    }
    assert_eq!(
        primaries,
        [
            (0, None),
            (4, Some(0)),
            (2, Some(4)),
            (1, Some(2)),
            (3, Some(2)),
            (6, Some(4)),
            (5, Some(2)),
        ]
    );
}
