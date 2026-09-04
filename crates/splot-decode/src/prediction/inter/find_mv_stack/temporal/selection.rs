// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{
    MAX_FRAME_DISTANCE, MotionFieldLayout, TemporalMotionFieldMetadata, TemporalProjectionConfig,
    sorted_reference_hints,
};

/// One projection candidate list, bounded by [`MFMV_STACK_SIZE`].
pub(super) type ProjectionQueue = splot_core::tile::InlineVec<TemporalProjection, MFMV_STACK_SIZE>;

/// One entry per reference slot, which a sequence bounds at
/// [`splot_core::headers::sequence::MAX_REF_FRAMES`].
type RefSlots<T> =
    splot_core::tile::InlineVec<T, { splot_core::headers::sequence::MAX_REF_FRAMES }>;

const TIP_MFMV_STACK_SIZE: usize = 3;
const MFMV_STACK_SIZE: usize = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TemporalProjection {
    pub(super) ref_index: usize,
    pub(super) side: usize,
    pub(super) target_ref: Option<usize>,
}

fn relative_distance(a: u32, b: u32) -> i32 {
    super::super::super::get_relative_dist(
        i32::try_from(a).unwrap_or(i32::MAX),
        i32::try_from(b).unwrap_or(i32::MAX),
    )
}

fn reference_motion_field<'a>(
    ref_index: usize,
    ref_frame_idx: &[u32],
    ref_motion_fields: &'a [Option<TemporalMotionFieldMetadata>],
) -> Option<&'a TemporalMotionFieldMetadata> {
    let slot = *ref_frame_idx.get(ref_index)?;
    ref_motion_fields.get(slot as usize)?.as_ref()
}

fn topo_sort_reference(
    ref_index: usize,
    ref_frame_idx: &[u32],
    ref_order_hints: &[Option<u32>],
    ref_motion_fields: &[Option<TemporalMotionFieldMetadata>],
    overlays: &[bool],
    visited: &mut [bool],
    stack: &mut RefSlots<usize>,
) {
    if visited.get(ref_index).copied().unwrap_or(true) {
        return;
    }
    visited[ref_index] = true;
    let Some(source) = reference_motion_field(ref_index, ref_frame_idx, ref_motion_fields)
        .filter(|source| source.is_inter)
    else {
        let _ = stack.push(ref_index);
        return;
    };
    for target_hint in source.ref_order_hints.iter().flatten() {
        let Some(target) = ref_order_hints
            .iter()
            .position(|hint| hint.is_some_and(|hint| relative_distance(hint, *target_hint) == 0))
        else {
            continue;
        };
        if !overlays.get(target).copied().unwrap_or(false) {
            topo_sort_reference(
                target,
                ref_frame_idx,
                ref_order_hints,
                ref_motion_fields,
                overlays,
                visited,
                stack,
            );
        }
    }
    stack.push(ref_index);
}

fn has_reference_on_side(
    source: &TemporalMotionFieldMetadata,
    source_hint: u32,
    future: bool,
) -> bool {
    source.ref_order_hints.iter().flatten().any(|&hint| {
        let distance = relative_distance(hint, source_hint);
        if future { distance > 0 } else { distance < 0 }
    })
}

fn closest_interpolation_distance(
    source: &TemporalMotionFieldMetadata,
    source_hint: u32,
    current_hint: u32,
    forward: bool,
) -> i32 {
    source
        .ref_order_hints
        .iter()
        .flatten()
        .filter_map(|&hint| {
            let source_to_ref = relative_distance(source_hint, hint);
            let current_to_ref = relative_distance(current_hint, hint);
            let matching_side = if forward {
                source_to_ref > 0 && current_to_ref > 0
            } else {
                source_to_ref < 0 && current_to_ref < 0
            };
            matching_side.then_some(source_to_ref.abs())
        })
        .min()
        .unwrap_or(i32::MAX)
}

pub(super) fn projection_queue(
    mi_dimensions: (usize, usize),
    current_hint: u32,
    config: TemporalProjectionConfig,
    ref_frame_idx: &[u32],
    ref_order_hints: &[Option<u32>],
    ref_motion_fields: &[Option<TemporalMotionFieldMetadata>],
    ref_motion_layouts: &[Option<MotionFieldLayout>],
) -> ProjectionQueue {
    let mut sorted_buffer = [(0usize, 0i32); super::MAX_SORTED_REFS];
    let sorted_len = sorted_reference_hints(ref_order_hints, &mut sorted_buffer);
    let mut sorted_slots = [0usize; super::MAX_SORTED_REFS];
    for (slot, &(index, _)) in sorted_slots.iter_mut().zip(&sorted_buffer[..sorted_len]) {
        *slot = index;
    }
    let sorted = &sorted_slots[..sorted_len];
    let mut overlays = RefSlots::<bool>::default();
    overlays.extend_within((0..ref_order_hints.len()).map(|index| {
        let Some(hint) = ref_order_hints[index] else {
            return false;
        };
        reference_motion_field(index, ref_frame_idx, ref_motion_fields).is_some_and(|field| {
            field
                .ref_order_hints
                .iter()
                .flatten()
                .any(|&target| relative_distance(hint, target) == 0)
        })
    }));
    let mut visited = RefSlots::<bool>::default();
    visited.extend_within(core::iter::repeat_n(false, ref_order_hints.len()));
    let mut stack = RefSlots::<usize>::default();
    for index in 0..ref_order_hints.len() {
        topo_sort_reference(
            index,
            ref_frame_idx,
            ref_order_hints,
            ref_motion_fields,
            &overlays,
            &mut visited,
            &mut stack,
        );
    }
    if stack.len() < 2 {
        return ProjectionQueue::default();
    }

    let cur_idx = sorted
        .iter()
        .rposition(|&index| {
            ref_order_hints[index].is_some_and(|hint| relative_distance(hint, current_hint) < 0)
        })
        .map_or(-1, |index| index as isize);
    let mut checked = RefSlots::<[bool; 2]>::default();
    checked.extend_within(core::iter::repeat_n([false; 2], ref_order_hints.len()));
    let mut queue = ProjectionQueue::default();
    let mut add = |projection: TemporalProjection, max_check: usize| {
        let Some(source_hint) = ref_order_hints.get(projection.ref_index).copied().flatten() else {
            return;
        };
        let Some(source) =
            reference_motion_field(projection.ref_index, ref_frame_idx, ref_motion_fields)
        else {
            return;
        };
        let expected = (mi_dimensions.1.div_ceil(2), mi_dimensions.0.div_ceil(2));
        let source_layout = ref_frame_idx
            .get(projection.ref_index)
            .and_then(|slot| usize::try_from(*slot).ok())
            .and_then(|slot| ref_motion_layouts.get(slot))
            .and_then(Option::as_ref);
        if queue.len() >= max_check
            || checked[projection.ref_index][projection.side]
            || !source.is_inter
            || source.frame_size != Some(config.frame_size)
            || source_layout.is_none_or(|layout| (layout.width8(), layout.height8()) != expected)
            || relative_distance(source_hint, current_hint).abs() > MAX_FRAME_DISTANCE
        {
            return;
        }
        checked[projection.ref_index][projection.side] = true;
        let _ = queue.push(projection);
    };

    let past_count = sorted
        .iter()
        .filter(|&&index| {
            ref_order_hints[index].is_some_and(|hint| relative_distance(hint, current_hint) < 0)
        })
        .count();
    let future_count = sorted
        .iter()
        .filter(|&&index| {
            ref_order_hints[index].is_some_and(|hint| relative_distance(hint, current_hint) > 0)
        })
        .count();
    if config.enable_tip
        && ((past_count > 0 && future_count > 0) || past_count >= 2)
        && cur_idx >= 0
    {
        let past = sorted[cur_idx as usize];
        let future = if future_count > 0 {
            sorted[cur_idx as usize + 1]
        } else {
            sorted[cur_idx as usize - 1]
        };
        let past_depth = stack.iter().position(|&index| index == past).unwrap_or(0);
        let future_depth = stack.iter().position(|&index| index == future).unwrap_or(0);
        let (start, target) = if past_depth > future_depth {
            (past, future)
        } else {
            (future, past)
        };
        add(
            TemporalProjection {
                ref_index: start,
                side: usize::from(
                    relative_distance(
                        ref_order_hints[start].unwrap_or(0),
                        ref_order_hints[target].unwrap_or(0),
                    ) < 0,
                ),
                target_ref: Some(target),
            },
            TIP_MFMV_STACK_SIZE,
        );
    }

    for group in 0..2isize {
        let past = usize::try_from(cur_idx - group)
            .ok()
            .and_then(|position| sorted.get(position).copied())
            .filter(|&index| {
                let hint = ref_order_hints[index].unwrap_or(0);
                reference_motion_field(index, ref_frame_idx, ref_motion_fields)
                    .is_some_and(|field| has_reference_on_side(field, hint, true))
            });
        let future = usize::try_from(cur_idx + 1 + group)
            .ok()
            .and_then(|position| sorted.get(position).copied())
            .filter(|&index| {
                let hint = ref_order_hints[index].unwrap_or(0);
                reference_motion_field(index, ref_frame_idx, ref_motion_fields)
                    .is_some_and(|field| has_reference_on_side(field, hint, false))
            });
        let distance = |index: Option<usize>, forward| {
            index
                .and_then(|index| {
                    Some((
                        reference_motion_field(index, ref_frame_idx, ref_motion_fields)?,
                        ref_order_hints.get(index).copied().flatten()?,
                    ))
                })
                .map_or(-1, |(field, hint)| {
                    closest_interpolation_distance(field, hint, current_hint, forward)
                })
        };
        let mut candidates = [(past, 1), (future, 0)];
        if distance(future, true) < distance(past, false) {
            candidates.reverse();
        }
        for (candidate, side) in candidates {
            if let Some(ref_index) = candidate {
                add(
                    TemporalProjection {
                        ref_index,
                        side,
                        target_ref: None,
                    },
                    TIP_MFMV_STACK_SIZE,
                );
            }
        }
    }
    for position in [cur_idx, cur_idx - 1] {
        if let Some(ref_index) = usize::try_from(position)
            .ok()
            .and_then(|position| sorted.get(position).copied())
        {
            add(
                TemporalProjection {
                    ref_index,
                    side: 0,
                    target_ref: None,
                },
                TIP_MFMV_STACK_SIZE,
            );
        }
    }
    for &ref_index in stack.iter().skip(1).rev() {
        let side = usize::from(
            ref_order_hints[ref_index]
                .is_some_and(|hint| relative_distance(hint, current_hint) < 0),
        );
        for side in [side, 1 - side] {
            add(
                TemporalProjection {
                    ref_index,
                    side,
                    target_ref: None,
                },
                MFMV_STACK_SIZE,
            );
        }
    }
    if config.reduced {
        queue.truncate(1);
    }
    queue
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::TemporalMotionField;
    use super::*;

    fn reference_field(
        mi_rows: usize,
        mi_cols: usize,
        hints: &[u32],
    ) -> (TemporalMotionFieldMetadata, MotionFieldLayout) {
        let mut field = TemporalMotionField::new(mi_rows, mi_cols).unwrap();
        field.set_reference_metadata(
            true,
            (mi_cols * 4, mi_rows * 4),
            &hints.iter().copied().map(Some).collect::<Vec<_>>(),
        );
        (field.metadata(), field.layout())
    }

    #[test]
    fn tip_pair_leads_the_priority_queue() {
        let products = [
            Some(reference_field(4, 4, &[3])),
            Some(reference_field(4, 4, &[1])),
        ];
        let fields = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.0.clone()))
            .collect::<Vec<_>>();
        let layouts = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.1))
            .collect::<Vec<_>>();
        let config = TemporalProjectionConfig {
            frame_size: (16, 16),
            step: 1,
            unit_size8: 8,
            enable_tip: true,
            enable_trajectory: false,
            reduced: false,
        };

        assert_eq!(
            *projection_queue(
                (4, 4),
                2,
                config,
                &[0, 1],
                &[Some(1), Some(3)],
                &fields,
                &layouts,
            )
            .as_ref(),
            vec![
                TemporalProjection {
                    ref_index: 0,
                    side: 1,
                    target_ref: Some(1),
                },
                TemporalProjection {
                    ref_index: 1,
                    side: 0,
                    target_ref: None,
                },
                TemporalProjection {
                    ref_index: 0,
                    side: 0,
                    target_ref: None,
                },
            ]
        );
    }

    #[test]
    fn tip_projection_pair_matches_sort_ref_order_for_equal_future_hints() {
        let products = std::array::from_fn::<_, 7, _>(|_| Some(reference_field(4, 4, &[])));
        let fields = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.0.clone()))
            .collect::<Vec<_>>();
        let layouts = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.1))
            .collect::<Vec<_>>();
        let ref_order_hints = [
            Some(10),
            Some(7),
            Some(6),
            Some(10),
            Some(5),
            Some(4),
            Some(3),
        ];
        let queue = projection_queue(
            (4, 4),
            8,
            TemporalProjectionConfig {
                frame_size: (16, 16),
                step: 1,
                unit_size8: 8,
                enable_tip: true,
                enable_trajectory: false,
                reduced: false,
            },
            &[0, 1, 2, 3, 4, 5, 6],
            &ref_order_hints,
            &fields,
            &layouts,
        );

        assert_eq!(
            queue.first(),
            Some(&TemporalProjection {
                ref_index: 3,
                side: 0,
                target_ref: Some(1),
            })
        );
    }

    #[test]
    fn reduced_mode_keeps_only_the_highest_priority_projection() {
        let products = [
            Some(reference_field(4, 4, &[3])),
            Some(reference_field(4, 4, &[1])),
        ];
        let fields = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.0.clone()))
            .collect::<Vec<_>>();
        let layouts = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.1))
            .collect::<Vec<_>>();

        assert_eq!(
            *projection_queue(
                (4, 4),
                2,
                TemporalProjectionConfig {
                    frame_size: (16, 16),
                    step: 1,
                    unit_size8: 8,
                    enable_tip: true,
                    enable_trajectory: false,
                    reduced: true,
                },
                &[0, 1],
                &[Some(1), Some(3)],
                &fields,
                &layouts,
            )
            .as_ref(),
            vec![TemporalProjection {
                ref_index: 0,
                side: 1,
                target_ref: Some(1),
            }]
        );
    }

    #[test]
    fn current_reference_does_not_enable_a_tip_pair() {
        let products = [
            Some(reference_field(4, 4, &[2])),
            Some(reference_field(4, 4, &[1])),
        ];
        let fields = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.0.clone()))
            .collect::<Vec<_>>();
        let layouts = products
            .iter()
            .map(|product| product.as_ref().map(|product| product.1))
            .collect::<Vec<_>>();

        let queue = projection_queue(
            (4, 4),
            2,
            TemporalProjectionConfig {
                frame_size: (16, 16),
                step: 1,
                unit_size8: 8,
                enable_tip: true,
                enable_trajectory: false,
                reduced: false,
            },
            &[0, 1],
            &[Some(1), Some(2)],
            &fields,
            &layouts,
        );

        assert!(
            queue
                .iter()
                .all(|projection| projection.target_ref.is_none())
        );
    }
}
