// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

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
    let mut slots: [Option<u64>; DEFAULT_REF_SLOTS] = [None; DEFAULT_REF_SLOTS];
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
                    (key_display, slots) = (display, [None; DEFAULT_REF_SLOTS]);
                    slots[0] = Some(display);
                    refs.clear();
                } else {
                    let valid = (0..DEFAULT_REF_SLOTS)
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
                ..
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
    for keyint in [1, 2, 3, 5, 7, 8, 9, 16, 17, 30, 64] {
        for mini_gop_size in (1..=8).chain([15, 16, 31, 32]) {
            for frames in 1..=70 {
                check(keyint, mini_gop_size, frames);
            }
        }
    }
    for mini_gop_size in (1..=8).chain([16, 32]) {
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
    let mut slots: [Option<u64>; DEFAULT_REF_SLOTS] = [None; DEFAULT_REF_SLOTS];
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

/// With 3 slots the planner refreshes only slots 0..3 and a key frame
/// refreshes all three; a mini-GOP of 4 needs more than 1 slot, and IPPP
/// codes over 1 slot.
#[test]
fn planners_use_only_their_slots() {
    let keyint = NonZeroU32::new(30).unwrap();
    let mut planner = GopPlanner::new(keyint, 4).with_ref_slots(3);
    for _ in 0..4 {
        let len = planner.next_len(usize::MAX, true).unwrap();
        for step in planner.plan(len).unwrap() {
            if let GopStep::Code(frame) = step {
                assert_eq!(frame.refresh_frame_flags >> 3, 0, "{frame:?}");
                assert_eq!(frame.ref_slots >> 3, 0, "{frame:?}");
                if frame.frame_type == FrameType::Key {
                    assert_eq!(frame.refresh_frame_flags, 0b111);
                }
            }
        }
    }
    let mut small = GopPlanner::new(keyint, 4).with_ref_slots(1);
    let len = small.next_len(usize::MAX, true).unwrap();
    small.plan(len).unwrap();
    let len = small.next_len(usize::MAX, true).unwrap();
    assert!(small.plan(len).is_err());
    let mut ippp = GopPlanner::new(keyint, 1).with_ref_slots(1);
    for _ in 0..4 {
        let len = ippp.next_len(usize::MAX, true).unwrap();
        assert!(
            matches!(&ippp.plan(len).unwrap()[..], [GopStep::Code(frame)] if frame.refresh_frame_flags == 1)
        );
    }
}

/// The fewest slots each mini-GOP size plans over: 1 slot for size 1, 2 for
/// 2, 3 for 4 and 4 for 8; one slot less fails.
#[test]
fn mini_gops_plan_over_their_fewest_slots() {
    let fits = |slots: u8, mini_gop: u8| {
        let mut planner =
            GopPlanner::new(NonZeroU32::new(30).unwrap(), mini_gop).with_ref_slots(slots);
        (0..4).all(|_| {
            let len = planner.next_len(usize::MAX, true).unwrap_or(1);
            planner.plan(len).is_ok()
        })
    };
    for (slots, mini_gop) in [(1, 1), (2, 2), (3, 4), (4, 8)] {
        assert!(fits(slots, mini_gop), "{slots} slots, mini-GOP {mini_gop}");
        if slots > 1 {
            assert!(
                !fits(slots - 1, mini_gop),
                "{} slots, mini-GOP {mini_gop}",
                slots - 1
            );
        }
    }
}

/// The steps of `frames` input frames, queued all at once, with `repeats`
/// marked as repeats in display order.
fn run_repeats(keyint: u32, mini_gop_size: u8, frames: u64, repeats: &[u64]) -> Vec<GopStep> {
    let mut planner =
        GopPlanner::new(NonZeroU32::new(keyint).unwrap(), mini_gop_size).with_sef_repeat(true);
    for &repeat in repeats {
        planner.repeat(repeat);
    }
    let mut steps = Vec::new();
    let mut queued = frames as usize;
    while let Some(len) = planner.next_len(queued, true) {
        steps.extend(planner.plan(len).unwrap());
        queued -= len;
    }
    steps
}

/// A repeat is a show-existing frame with the display order hint of the slot
/// that holds the newest coded frame: right after a key frame, inside the
/// keyint (the mini-GOP stops before it), before a scheduled key frame and
/// at the end of the input. A repeat at a scheduled key frame is coded as
/// the key frame. Every input frame gets one output.
#[test]
fn repeats_show_the_newest_coded_frame_with_an_explicit_order_hint() {
    let steps = run_repeats(10, 4, 13, &[1, 2, 7, 9, 10, 12]);
    let mut slots = [None; MAX_REF_SLOTS];
    let (mut outputs, mut repeats) = (Vec::new(), Vec::new());
    for step in &steps {
        match *step {
            GopStep::Code(frame) => {
                assert_ne!(frame.display_index, 12);
                for slot in 0..MAX_REF_SLOTS {
                    if frame.refresh_frame_flags & 1 << slot != 0 {
                        slots[slot] = Some(frame.display_index);
                    }
                }
            }
            GopStep::ShowExisting {
                slot,
                display_index,
                order_hint: Some(hint),
                ..
            } => {
                let newest = slots.iter().flatten().max().copied();
                assert_eq!(slots[slot as usize], newest, "{display_index}");
                assert_eq!(
                    newest,
                    Some(display_index - 1 - u64::from(display_index == 2))
                );
                assert_eq!(u64::from(hint), display_index % 10, "{display_index}");
                repeats.push(display_index);
            }
            GopStep::ShowExisting { .. } => {}
        }
        outputs.extend(step.output());
    }
    assert_eq!(repeats, [1, 2, 7, 9, 12]);
    assert_eq!(outputs, (0..13).collect::<Vec<_>>());
    let key =
        |step: &GopStep| matches!(step, GopStep::Code(frame) if frame.frame_type == FrameType::Key);
    assert_eq!(steps.iter().filter(|step| key(step)).count(), 2);
}

/// A run of repeats stops where the next coded frame would leave half the
/// order-hint window of the frame it repeats; that frame is coded, and the
/// frames after it repeat it.
#[test]
fn a_repeat_run_stops_inside_half_the_order_hint_window() {
    let steps = run_repeats(300, 4, 140, &(1..140).collect::<Vec<_>>());
    let coded: Vec<u64> = steps
        .iter()
        .filter_map(|step| match step {
            GopStep::Code(frame) => Some(frame.display_index),
            GopStep::ShowExisting { .. } => None,
        })
        .collect();
    assert_eq!(coded, [0, 125]);
    assert_eq!(steps.len(), 140);
    let mut off = GopPlanner::new(NonZeroU32::new(300).unwrap(), 4);
    off.repeat(1);
    assert_eq!(off.next_len(3, true), Some(1));
    off.plan(1).unwrap();
    assert_eq!(off.next_len(3, true), Some(3));
}

/// With `open_loop_key = 1` each periodic key frame is the hidden first frame
/// of the mini-GOP that ends at it, coded as an open-loop key frame that
/// refreshes one slot; the frames before it in that mini-GOP and their
/// show-existing frames are leading; after it only its slot stays valid. The
/// first frame and a scene cut stay closed-loop.
#[test]
fn periodic_key_frames_open_their_mini_gops() {
    let mut planner = GopPlanner::new(NonZeroU32::new(12).unwrap(), 4).with_open_loop_key(1);
    planner.force_key(30);
    let mut steps = Vec::new();
    while let Some(len) = planner.next_len(usize::MAX, true) {
        steps.extend(planner.plan(len).unwrap());
        if planner.next_display() > 44 {
            break;
        }
    }
    let frames: Vec<GopFrame> = steps
        .iter()
        .filter_map(|step| match *step {
            GopStep::Code(frame) => Some(frame),
            GopStep::ShowExisting { .. } => None,
        })
        .collect();
    let keys: Vec<(u64, bool)> = frames
        .iter()
        .filter(|frame| frame.frame_type == FrameType::Key)
        .map(|frame| (frame.display_index, frame.open_loop))
        .collect();
    assert_eq!(
        keys,
        [(0, false), (12, true), (24, true), (30, false), (42, true)]
    );
    for olk in frames.iter().filter(|frame| frame.open_loop) {
        let key = olk.display_index;
        assert!(!olk.show_frame && olk.layer == 0 && olk.primary_slot.is_none());
        assert_eq!(olk.refresh_frame_flags.count_ones(), 1);
        let closed_loop = if key > 30 { 30 } else { 0 };
        assert_eq!(
            u64::from(olk.order_hint),
            key - closed_loop,
            "{key}: order hint"
        );
        let at = frames.iter().position(|frame| frame == olk).unwrap();
        let leading: Vec<u64> = frames[at + 1..]
            .iter()
            .take_while(|frame| frame.display_index < key)
            .map(|frame| frame.display_index)
            .collect();
        assert_eq!(leading, [key - 2, key - 3, key - 1]);
        assert!(frames[at + 1..at + 4].iter().all(|frame| frame.leading));
        let regular = frames[at + 4];
        assert!(!regular.leading && regular.display_index > key);
        assert_eq!(regular.ref_slots, olk.refresh_frame_flags);
    }
    let sefs: Vec<(u64, bool)> = steps
        .iter()
        .filter_map(|step| match *step {
            GopStep::ShowExisting {
                display_index,
                leading,
                ..
            } => Some((display_index, leading)),
            GopStep::Code(_) => None,
        })
        .filter(|&(display, _)| (9..=24).contains(&display))
        .collect();
    assert_eq!(
        sefs,
        [
            (10, true),
            (12, false),
            (14, false),
            (16, false),
            (18, false),
            (20, false),
            (22, true),
            (24, false)
        ]
    );
}

/// An open-loop key frame without a leading frame stays closed-loop: with
/// mini-GOPs of one frame every periodic key frame is a closed-loop key frame.
#[test]
fn key_frames_without_leading_frames_stay_closed_loop() {
    let mut planner = GopPlanner::new(NonZeroU32::new(5).unwrap(), 1).with_open_loop_key(1);
    let mut keys = Vec::new();
    while planner.next_display() < 16 {
        let len = planner.next_len(usize::MAX, true).unwrap();
        for step in planner.plan(len).unwrap() {
            if let GopStep::Code(frame) = step {
                assert!(!frame.open_loop && !frame.leading);
                if frame.frame_type == FrameType::Key {
                    keys.push(frame.display_index);
                }
            }
        }
    }
    assert_eq!(keys, [0, 5, 10, 15]);
}

/// Rule R of [`GopPlanner::with_max_tlayer_id`]: for each operating point
/// `n`, a decoder that drops the frames above temporal layer `n` sees, in
/// every frame it keeps, the same slots of that frame's layer or below as the
/// full decoder (§ 6.4.1: the same order hint and reference map). The
/// combinations that `PresetParams::validate` refuses are skipped.
#[test]
fn temporal_layers_keep_the_slots_of_each_operating_point() {
    for max_tlayer_id in 1..=3u8 {
        for mini_gop_size in 1..=32u8 {
            for (slots_used, keyint) in [(8u8, 255u32), (4, 100), (16, 64)] {
                let case = format!(
                    "max_tlayer_id {max_tlayer_id} mini-GOP {mini_gop_size} slots {slots_used}"
                );
                let params = crate::preset::PresetParams {
                    mini_gop_size,
                    bitrate_mini_gop_size: mini_gop_size,
                    lookahead_frames: 32,
                    num_ref_frames: slots_used,
                    max_tlayer_id,
                    implicit_output: true,
                    ..*crate::preset::Preset::default().params()
                };
                if params.validate().is_err() {
                    eprintln!("refused: {case}");
                    continue;
                }
                let mut planner = GopPlanner::new(NonZeroU32::new(keyint).unwrap(), mini_gop_size)
                    .with_ref_slots(slots_used)
                    .with_implicit_output(true)
                    .with_max_tlayer_id(max_tlayer_id);
                let mut steps = Vec::new();
                let mut queued = 0;
                for ended in core::iter::repeat_n(false, 300).chain([true]) {
                    queued += usize::from(!ended);
                    while let Some(len) = planner.next_len(queued, ended) {
                        let planned = planner.plan(len);
                        let Ok(planned) = planned else {
                            panic!("{case}: {planned:?} at display {}", planner.next_display());
                        };
                        steps.extend(planned);
                        queued -= len;
                    }
                }
                let mut decoders = [[None::<(u64, u8)>; 16]; 4];
                let mut coded_layers = [0usize; 4];
                for step in &steps {
                    let GopStep::Code(frame) = *step else {
                        continue;
                    };
                    let tlayer = frame.tlayer.get();
                    assert_eq!(tlayer, frame.layer.min(max_tlayer_id), "{case}");
                    coded_layers[usize::from(tlayer)] += 1;
                    let visible = |slots: &[Option<(u64, u8)>; 16]| {
                        slots.map(|held| held.filter(|&(_, layer)| layer <= tlayer))
                    };
                    let full = visible(&decoders[3]);
                    for (n, slots) in decoders.iter_mut().enumerate().skip(usize::from(tlayer)) {
                        assert_eq!(
                            visible(slots),
                            full,
                            "{case}: operating point {n}, {frame:?}"
                        );
                        if frame.frame_type == FrameType::Key && !frame.open_loop {
                            *slots = [None; 16];
                            slots[0] = Some((frame.display_index, 0));
                        } else {
                            for (slot, held) in slots.iter_mut().enumerate() {
                                if frame.refresh_frame_flags >> slot & 1 == 1 {
                                    *held = Some((frame.display_index, tlayer));
                                }
                            }
                        }
                    }
                }
                let layers = (mini_gop_size.max(1) - 1)
                    .checked_ilog2()
                    .map_or(0, |log| log + 1);
                for layer in 0..=usize::from(max_tlayer_id).min(layers as usize) {
                    assert!(coded_layers[layer] > 0, "{case}: no frame in layer {layer}");
                }
            }
        }
    }
}
