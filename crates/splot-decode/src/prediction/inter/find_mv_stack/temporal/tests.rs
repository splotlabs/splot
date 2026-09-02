// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use splot_parallel::{ThreadCount, WorkerPool};

#[test]
fn temporal_motion_block_stays_compact() {
    assert_eq!(size_of::<TemporalMotionBlock>(), 120);
}

#[test]
fn temporal_grid_cell_count_fits_at_maximum_frame_size() {
    let max_frame_dimension = 1_usize << 16;
    let max_mi_dimension = 2 * ((max_frame_dimension + 7) >> 3);
    let (width8, height8, cells) =
        allocate_temporal_grid::<()>(max_mi_dimension, max_mi_dimension).unwrap();
    assert_eq!((width8, height8), (8192, 8192));
    assert_eq!(cells.len(), width8 * height8);
}

#[test]
fn temporal_grid_rejects_impossible_capacity() {
    assert!(allocate_temporal_grid::<u8>(1, usize::MAX).is_none());
}

#[test]
fn projected_temporal_reset_reports_allocation_failure() {
    let mut field = ProjectedTemporalMotionField::default();
    let result = field.reset(2, usize::MAX);
    assert!(matches!(
        &result,
        Err(crate::DecodeError::Reconstruction {
            source: splot_recon::ReconError::WorkspaceAllocationFailed {
                plane: splot_recon::PlaneId::Y,
                context: "inter projected temporal motion field"
            }
        })
    ));
}

#[test]
fn compact_temporal_motion_preserves_every_warp_shape() {
    let mvs = [Mv { row: 9, col: -7 }, Mv { row: -5, col: 3 }];
    let first = [1 << 16, 0, 2 << 16, 0, 1 << 16, -(1 << 16)];
    let second = [1 << 16, 0, -(3 << 16), 0, 1 << 16, 2 << 16];
    for warp_params in [
        [None, None],
        [Some(first), None],
        [None, Some(second)],
        [Some(first), Some(second)],
    ] {
        let motion = TemporalBlockMotion::new(mvs, warp_params);
        for list in 0..2 {
            let expected =
                warp_params[list].map_or(mvs[list], |params| warp_sub_mv_at(params, 4, 6, 8, 10));
            assert_eq!(motion.mv_at(list, 4, 6, 8, 10), expected);
        }
    }
}

fn tip_context(
    current_order_hint: u32,
    ref_order_hints: Vec<Option<u32>>,
    mi_rows: usize,
    mi_cols: usize,
) -> TemporalMvContext {
    TemporalMvContext {
        current_order_hint,
        ref_order_hints,
        field: ProjectedTemporalMotionField::new(mi_rows, mi_cols).unwrap(),
        projection_scratch: ProjectedTemporalMotionField::new(0, 0).unwrap(),
        average_scratch: ProjectedTemporalMotionField::new(0, 0).unwrap(),
        trajectories: None,
        trajectory_scratch: None,
        tip: None,
        banded: None,
    }
}

#[test]
fn whole_field_reuses_published_band_cells() {
    let mut field = TemporalMotionField::new(4, 4).unwrap();
    field.set_reference_metadata(true, (16, 16), &[Some(0)]);
    *field.cell_mut(0, 0).unwrap() = TemporalMotionCell {
        ref_indices: [0, INVALID_TEMPORAL_REF],
        mvs: [
            CompressedTemporalMv { row: 1, col: 2 },
            CompressedTemporalMv::ZERO,
        ],
    };
    let layout = field.layout();
    let metadata = field.metadata();
    let expected = field.cell(0, 0);
    let bands = field
        .into_bands()
        .into_iter()
        .map(Arc::new)
        .collect::<Vec<_>>();
    let first_band_cells = bands[0].cells.as_slice().as_ptr();

    let rebuilt = TemporalMotionField::from_bands(layout, &metadata, bands).unwrap();

    assert_eq!(rebuilt.cell(0, 0), expected);
    assert_eq!(rebuilt.row(0).unwrap().as_ptr(), first_band_cells);
}

#[test]
fn spatial_derivation_uses_reference_trajectories() {
    let mut candidate = TrajectoryMotionField::new(2, 2).unwrap();
    candidate.set(
        0,
        0,
        Mv {
            row: -10,
            col: -192,
        },
    );
    let mut dst = TrajectoryMotionField::new(2, 2).unwrap();
    dst.set(0, 0, Mv { row: 12, col: 240 });
    let mut context = tip_context(4, vec![Some(0), Some(9)], 2, 2);
    context.trajectories = Some(TrajectoryState::from_fields(&[candidate, dst]));

    assert_eq!(
        context.derive_spatial_mv(1, 0, Mv { row: -12, col: -67 }, 0, 0),
        Some(Mv { row: 10, col: 365 })
    );
}

#[test]
fn compound_spatial_derivation_uses_both_reference_trajectories() {
    let mut candidate = TrajectoryMotionField::new(2, 2).unwrap();
    candidate.set(
        0,
        0,
        Mv {
            row: -10,
            col: -192,
        },
    );
    let mut dst0 = TrajectoryMotionField::new(2, 2).unwrap();
    dst0.set(0, 0, Mv { row: 12, col: 240 });
    let mut dst1 = TrajectoryMotionField::new(2, 2).unwrap();
    dst1.set(0, 0, Mv { row: -8, col: 100 });
    let mut context = tip_context(4, vec![Some(0), Some(9), Some(6)], 2, 2);
    context.trajectories = Some(TrajectoryState::from_fields(&[candidate, dst0, dst1]));

    assert_eq!(
        context.derive_compound_spatial_mvs([1, 2], 0, Mv { row: -12, col: -67 }, 0, 0,),
        Some([Mv { row: 10, col: 365 }, Mv { row: -10, col: 225 }])
    );
}

#[test]
fn spatial_derivation_projects_references_on_the_same_side() {
    let context = tip_context(10, vec![Some(8), Some(6)], 2, 2);

    assert_eq!(
        context.derive_spatial_mv(0, 1, Mv { row: 8, col: 12 }, 0, 0),
        Some(Mv { row: 4, col: 6 })
    );
}

#[test]
fn trajectory_derivation_clamps_to_the_motion_vector_domain() {
    assert_eq!(
        derive_mv_from_trajectories(
            Mv {
                row: MV_LIMIT,
                col: -MV_LIMIT,
            },
            Mv {
                row: MV_LIMIT,
                col: -MV_LIMIT,
            },
            Mv {
                row: -MV_LIMIT,
                col: MV_LIMIT,
            },
        ),
        Mv {
            row: MV_LIMIT,
            col: -MV_LIMIT,
        }
    );
}

#[test]
fn tip_reference_pair_uses_the_nearest_past_and_future_references() {
    let context = tip_context(10, vec![Some(6), Some(9), Some(12), Some(15)], 4, 4);

    assert_eq!(
        context.tip_reference_pair(),
        Some(TipReferencePair {
            past_ref: 1,
            future_ref: 2,
            past_offset: 1,
            future_offset: -2,
            ref_offset: 3,
        })
    );
}

#[test]
fn tip_reference_pair_matches_sort_ref_order_for_equal_future_hints() {
    let context = tip_context(
        8,
        vec![
            Some(10),
            Some(7),
            Some(6),
            Some(10),
            Some(5),
            Some(4),
            Some(3),
        ],
        4,
        4,
    );

    assert_eq!(
        context.tip_reference_pair(),
        Some(TipReferencePair {
            past_ref: 1,
            future_ref: 3,
            past_offset: 1,
            future_offset: -2,
            ref_offset: 3,
        })
    );
}

#[test]
fn tip_reference_pair_uses_the_two_nearest_past_references() {
    let context = tip_context(10, vec![Some(2), Some(6), Some(9)], 4, 4);

    assert_eq!(
        context.tip_reference_pair(),
        Some(TipReferencePair {
            past_ref: 2,
            future_ref: 1,
            past_offset: 1,
            future_offset: 4,
            ref_offset: 3,
        })
    );
}

#[test]
fn reference_order_hints_exclude_invalid_slots() {
    assert_eq!(
        reference_order_hints(&[0, 1, 2], &[true, false, true], &[8, 9, 12]),
        vec![Some(8), None, Some(12)]
    );
}

#[test]
fn metadata_constructor_matches_delayed_reference_resolution() {
    let ref_order_hints = [Some(1), None, Some(3)];
    let block = TemporalMotionBlock::new(
        0,
        0,
        2,
        2,
        2,
        2,
        4,
        [Some(3), Some(1)],
        [Mv { row: 8, col: 16 }, Mv { row: 24, col: 32 }],
        [None; 2],
    );
    let mut delayed = TemporalMotionField::new(2, 2).unwrap();
    delayed.record_block(block);
    delayed.set_reference_metadata(true, (8, 8), &ref_order_hints);

    let mut immediate =
        TemporalMotionField::new_with_metadata(2, 2, true, (8, 8), &ref_order_hints).unwrap();
    immediate.record_block(block);

    assert_eq!(immediate, delayed);
}

#[test]
fn single_reference_motion_is_stored_in_both_slots() {
    for source_list in 0..2 {
        let mut field = TemporalMotionField::new(2, 2).unwrap();
        field.set_reference_metadata(true, (8, 8), &[Some(7)]);
        let mut ref_order_hints = [None; 2];
        let mut mvs = [Mv::ZERO; 2];
        ref_order_hints[source_list] = Some(7);
        mvs[source_list] = Mv { row: 8, col: -12 };

        field.record_block(TemporalMotionBlock::new(
            0,
            0,
            2,
            2,
            2,
            2,
            8,
            ref_order_hints,
            mvs,
            [None; 2],
        ));

        let cell = field.cell(0, 0).unwrap();
        assert_eq!(cell.ref_indices, [0; 2]);
        assert_eq!(
            cell.mvs.map(uncompress_tmvp_mv),
            [Mv { row: 8, col: -12 }; 2]
        );
    }
}

#[test]
fn compound_references_are_stored_in_temporal_slot_order() {
    let cases = [
        ([Some(1), Some(3)], [Some(3), Some(1)]),
        ([Some(5), Some(6)], [Some(6), Some(5)]),
        ([Some(6), Some(3)], [Some(3), Some(6)]),
        ([Some(3), Some(6)], [Some(3), Some(6)]),
    ];
    for (input, expected) in cases {
        let mut field = TemporalMotionField::new(2, 2).unwrap();
        field.set_reference_metadata(true, (8, 8), &[Some(1), Some(3), Some(5), Some(6)]);
        field.record_block(TemporalMotionBlock::new(
            0,
            0,
            2,
            2,
            2,
            2,
            4,
            input,
            [Mv { row: 8, col: 16 }, Mv { row: 24, col: 32 }],
            [None; 2],
        ));

        let cell = field.cell(0, 0).unwrap();
        assert_eq!(
            cell.ref_indices
                .map(|index| field.ref_order_hints[usize::from(index)]),
            expected
        );
        let swapped = input != expected;
        assert_eq!(
            uncompress_tmvp_mv(cell.mvs[0]).row,
            if swapped { 24 } else { 8 }
        );
        assert_eq!(
            uncompress_tmvp_mv(cell.mvs[1]).row,
            if swapped { 8 } else { 24 }
        );
    }
}

#[test]
fn step_two_projection_samples_and_stores_on_the_even_grid() {
    let mut source = TemporalMotionField::new(8, 8).unwrap();
    source.set_reference_metadata(true, (32, 32), &[Some(0)]);
    for x8 in 0..source.width8 {
        *source.cell_mut(0, x8).unwrap() = TemporalMotionCell {
            ref_indices: [0, INVALID_TEMPORAL_REF],
            mvs: [
                compress_tmvp_mv(Mv { row: 0, col: -64 }),
                CompressedTemporalMv::ZERO,
            ],
        };
    }
    let mut other = TemporalMotionField::new(8, 8).unwrap();
    other.set_reference_metadata(true, (32, 32), &[]);

    let mut context = TemporalMvContext::from_references(
        (8, 8),
        2,
        TemporalProjectionConfig {
            frame_size: (32, 32),
            step: 2,
            unit_size8: 8,
            enable_tip: false,
            enable_trajectory: false,
            reduced: false,
        },
        &[0, 1],
        &[true, true],
        &[1, 3],
        &[Some(Arc::new(source)), Some(Arc::new(other))],
    )
    .unwrap();

    assert!(context.field.cell(0, 0).unwrap().valid);
    assert!(context.field.cell(0, 2).unwrap().valid);
    assert!(!context.field.cell(0, 1).unwrap().valid);
    assert!(!context.field.cell(0, 3).unwrap().valid);

    context.fill_sampling_gaps(2, 16);
    assert_eq!(context.field.cell(0, 1), context.field.cell(0, 0));
    assert_eq!(context.field.cell(0, 3), context.field.cell(0, 2));
}

#[test]
fn backward_projection_preserves_source_to_current_direction() {
    let project = |threads| {
        WorkerPool::new(ThreadCount::from(threads))
            .unwrap()
            .install(|| {
                let mut source = TemporalMotionField::new(18, 56).unwrap();
                source.set_reference_metadata(true, (56 * 8, 18 * 8), &[Some(9)]);
                *source.cell_mut(8, 26).unwrap() = TemporalMotionCell {
                    ref_indices: [INVALID_TEMPORAL_REF, 0],
                    mvs: [
                        CompressedTemporalMv::ZERO,
                        compress_tmvp_mv(Mv { row: 10, col: 232 }),
                    ],
                };
                let mut output = ProjectedTemporalMotionField::new(18, 56).unwrap();
                project_whole_temporal_motion_field(
                    &source,
                    4,
                    2,
                    1,
                    8,
                    0,
                    1,
                    None,
                    &[],
                    None,
                    &mut output,
                );
                output
            })
    };
    let output = project(1);

    assert_eq!(
        output.cell(8, 25),
        Some(ProjectedTemporalMotionCell {
            valid: true,
            mv: Mv {
                row: -10,
                col: -232,
            },
            ref_offset: 5,
        })
    );
    assert!(!output.cell(8, 27).unwrap().valid);
    assert_eq!(output, project(4));
}

#[test]
fn projection_records_zero_offset_reference() {
    let mut source = TemporalMotionField::new(4, 4).unwrap();
    source.set_reference_metadata(true, (16, 16), &[Some(4)]);
    *source.cell_mut(0, 0).unwrap() = TemporalMotionCell {
        ref_indices: [0, INVALID_TEMPORAL_REF],
        mvs: [CompressedTemporalMv::ZERO; 2],
    };
    let mut output = ProjectedTemporalMotionField::new(4, 4).unwrap();

    project_whole_temporal_motion_field(
        &source,
        4,
        2,
        1,
        8,
        0,
        0,
        None,
        &[Some(4)],
        None,
        &mut output,
    );

    assert_eq!(
        output.cell(0, 0),
        Some(ProjectedTemporalMotionCell {
            valid: true,
            mv: Mv::ZERO,
            ref_offset: 0,
        })
    );
}

#[test]
fn zero_offset_projection_uses_the_zero_divisor_multiplier() {
    assert_eq!(project_mv(Mv { row: 24, col: -40 }, 3, 0), Mv::ZERO);
}

#[test]
fn projection_is_total_over_untrusted_integer_inputs() {
    for numerator in [
        i32::MIN,
        -MAX_FRAME_DISTANCE,
        0,
        MAX_FRAME_DISTANCE,
        i32::MAX,
    ] {
        for denominator in [i32::MIN, -1, 0, MAX_FRAME_DISTANCE, i32::MAX] {
            let projected = project_mv(
                Mv {
                    row: i32::MIN,
                    col: i32::MAX,
                },
                numerator,
                denominator,
            );
            assert!(projected.row.abs() <= MV_LIMIT && projected.col.abs() <= MV_LIMIT);
        }
    }
}

#[test]
fn tip_candidate_distinguishes_absent_from_present_invalid_cells() {
    let mut context = tip_context(4, vec![Some(2), Some(6)], 2, 2);
    context.tip = Some(TipReferencePair {
        past_ref: 0,
        future_ref: 1,
        past_offset: 2,
        future_offset: -2,
        ref_offset: 4,
    });
    let base = Mv { row: 3, col: -5 };
    assert_eq!(context.tip_candidate(0, 0, base), Some([base; 2]));

    context.field = ProjectedTemporalMotionField::default();
    assert_eq!(context.tip_candidate(0, 0, base), None);
}

#[test]
fn bounded_tmvp_projection_matches_wide_projection() {
    let components = [-REFMVS_LIMIT, -1024, -1, 0, 1, 1024, REFMVS_LIMIT];
    for &component in &components {
        let mv = Mv {
            row: component,
            col: -component,
        };
        for numerator in -MAX_FRAME_DISTANCE..=MAX_FRAME_DISTANCE {
            for denominator in 0..=MAX_FRAME_DISTANCE {
                assert_eq!(
                    project_tmvp_mv(mv, numerator, denominator),
                    project_mv(mv, numerator, denominator),
                );
            }
        }
    }
    let wide = Mv {
        row: REFMVS_LIMIT + 1,
        col: -(REFMVS_LIMIT + 1),
    };
    assert_eq!(project_tmvp_mv(wide, 31, 1), project_mv(wide, 31, 1));
}

#[test]
fn side_rejected_projection_still_extends_existing_trajectory() {
    let mut trajectories = TrajectoryState::new((112, 252), 6, 1, 8).unwrap();
    trajectories.whole_band().unwrap().observe_projection(
        0,
        Some(1),
        Some(1),
        54,
        125,
        Mv { row: 64, col: -256 },
        1,
        2,
        false,
    );
    let mut source = TemporalMotionField::new(112, 252).unwrap();
    source.set_reference_metadata(true, (1008, 448), &[Some(0)]);
    source.record_block(TemporalMotionBlock::new(
        110,
        242,
        2,
        2,
        112,
        252,
        4,
        [Some(0), None],
        [Mv { row: 36, col: -160 }, Mv::ZERO],
        [None; 2],
    ));
    let mut output = ProjectedTemporalMotionField::new(112, 252).unwrap();

    project_whole_temporal_motion_field(
        &source,
        4,
        5,
        1,
        8,
        1,
        1,
        None,
        &[Some(6), Some(4), None, None, None, Some(0)],
        Some(&mut trajectories),
        &mut output,
    );

    let fields = trajectories.into_fields();
    assert_eq!(fields[5].cell(54, 123), Some(Mv { row: 68, col: -288 }));
    assert!(output.cells.iter().all(|cell| !cell.valid));
}

#[test]
fn sampling_gap_does_not_average_across_tmvp_units() {
    let mut context = tip_context(2, vec![Some(0)], 2, 36);
    context.field.set(0, 14, Mv { row: 8, col: 16 }, 2, true);
    context.field.set(0, 16, Mv { row: 24, col: 80 }, 4, true);

    context.fill_sampling_gaps(2, 16);

    assert_eq!(context.field.cell(0, 15), context.field.cell(0, 14));
    assert_eq!(context.field.cell(0, 17), context.field.cell(0, 16));
}

#[test]
fn temporal_projection_stays_within_the_vertical_tmvp_unit() {
    assert!(!tmvp_position_is_near(52, 107, 47, 107, 2, 16));
    assert!(tmvp_position_is_near(47, 107, 47, 107, 2, 16));
}

#[test]
fn step_one_projection_uses_64_pixel_tmvp_unit_bounds() {
    assert!(!tmvp_position_is_near(52, 107, 47, 107, 1, 8));
    assert!(tmvp_position_is_near(47, 107, 47, 107, 1, 8));
    assert!(tmvp_position_is_near(47, 100, 47, 107, 1, 8));
    assert!(!tmvp_position_is_near(47, 99, 47, 107, 1, 8));
    assert!(tmvp_position_is_near(47, 115, 47, 107, 1, 8));
    assert!(!tmvp_position_is_near(47, 116, 47, 107, 1, 8));
}

#[test]
fn tip_projection_fills_unsampled_units_and_adds_the_block_mv() {
    let mut context = tip_context(10, vec![Some(8), Some(12)], 4, 4);
    context.field.set(0, 0, Mv { row: 8, col: -16 }, 4, true);

    let references = context.tip_reference_pair().unwrap();
    assert!(context.prepare_tip(references, 2, 2, false).is_ok());
    assert_eq!(context.tip_references(), context.tip_reference_pair());
    let expected = [Mv { row: 5, col: -6 }, Mv { row: -3, col: 10 }];
    for y8 in 0..2 {
        for x8 in 0..2 {
            assert_eq!(
                context.tip_candidate(y8, x8, Mv { row: 1, col: 2 }),
                Some(expected)
            );
        }
    }
}

#[test]
fn tip_temporal_scaling_clamps_to_the_reference_mv_domain() {
    let mut context = tip_context(10, vec![Some(6), Some(15)], 2, 2);
    context.field.set(
        0,
        0,
        Mv {
            row: -256,
            col: 256,
        },
        1,
        true,
    );

    let references = context.tip_reference_pair().unwrap();
    assert!(context.prepare_tip(references, 1, 8, false).is_ok());
    assert_eq!(
        context.field.cell(0, 0),
        Some(ProjectedTemporalMotionCell {
            valid: true,
            mv: Mv {
                row: -REFMVS_LIMIT,
                col: REFMVS_LIMIT,
            },
            ref_offset: 9,
        })
    );
}

#[test]
fn tip_step_one_hole_fill_stays_within_64_pixel_tmvp_units() {
    let mut context = tip_context(10, vec![Some(8), Some(12)], 2, 32);
    context.field.set(0, 7, Mv { row: 8, col: -16 }, 4, true);

    let references = context.tip_reference_pair().unwrap();
    assert!(context.prepare_tip(references, 1, 16, true).is_ok());
    assert_eq!(context.tip_candidate(0, 8, Mv::ZERO), Some([Mv::ZERO; 2]));
}

#[test]
fn tip_candidate_clamps_motion_field_coordinates_to_the_frame() {
    let references = TipReferencePair {
        past_ref: 0,
        future_ref: 1,
        past_offset: -1,
        future_offset: 1,
        ref_offset: 1,
    };
    let context =
        TemporalMvContext::with_tip_sample(4, 4, references, 1, 1, Mv { row: 16, col: 32 })
            .unwrap();

    assert_eq!(
        context.tip_candidate(usize::MAX, usize::MAX, Mv::ZERO),
        context.tip_candidate(1, 1, Mv::ZERO)
    );
}

#[test]
fn tip_newly_averaged_sample_keeps_the_scaled_reference_offset() {
    let mut context = tip_context(10, vec![Some(6), Some(15)], 2, 32);
    context.field.set(0, 14, Mv { row: 18, col: -36 }, 9, true);

    let references = context.tip_reference_pair().unwrap();
    assert!(context.prepare_tip(references, 2, 16, true).is_ok());
    let cell = context.field.cell(0, 11).unwrap();
    assert_eq!(
        cell,
        ProjectedTemporalMotionCell {
            valid: true,
            mv: Mv { row: 18, col: -36 },
            ref_offset: 9,
        }
    );
}

#[test]
fn tip_averaging_never_inherits_the_previous_frame_between_sampled_cells() {
    let mut context = tip_context(10, vec![Some(6), Some(15)], 4, 8);
    for y8 in 0..2 {
        for x8 in 0..4 {
            context.field.set(y8, x8, Mv { row: 18, col: -36 }, 9, true);
        }
    }
    let references = context.tip_reference_pair().unwrap();
    assert!(context.prepare_tip(references, 1, 8, true).is_ok());

    context.field.reset(4, 8).unwrap();
    assert!(context.prepare_tip(references, 2, 16, true).is_ok());

    for y8 in 0..2 {
        for x8 in 0..4 {
            assert_eq!(
                context.field.cell(y8, x8),
                Some(ProjectedTemporalMotionCell::default()),
                "cell ({y8}, {x8}) kept the previous frame's TIP motion"
            );
        }
    }
}

#[test]
fn refresh_reuses_projected_and_trajectory_storage() {
    let config = TemporalProjectionConfig {
        frame_size: (64, 64),
        step: 1,
        unit_size8: 8,
        enable_tip: false,
        enable_trajectory: true,
        reduced: false,
    };
    let ref_frame_idx = [0];
    let ref_valid = [false];
    let ref_order_hint = [u32::MAX];
    let ref_motion_fields = [None];
    let mut context = TemporalMvContext::from_references(
        (16, 16),
        0,
        config,
        &ref_frame_idx,
        &ref_valid,
        &ref_order_hint,
        &ref_motion_fields,
    )
    .unwrap();
    let field_ptr = context.field.cells.as_ptr();
    let trajectories = context.trajectories.as_ref().unwrap();
    let trajectory_ptr = trajectories.cells.as_ptr();
    let positions_ptr = trajectories.positions[0].as_ptr();
    let offsets_ptr = trajectories.projection_offsets.as_ptr();

    context
        .refresh_from_references(
            (16, 16),
            1,
            config,
            &ref_frame_idx,
            &ref_valid,
            &ref_order_hint,
            &ref_motion_fields,
        )
        .unwrap();

    let trajectories = context.trajectories.as_ref().unwrap();
    assert_eq!(context.field.cells.as_ptr(), field_ptr);
    assert_eq!(trajectories.cells.as_ptr(), trajectory_ptr);
    assert_eq!(trajectories.positions[0].as_ptr(), positions_ptr);
    assert_eq!(trajectories.projection_offsets.as_ptr(), offsets_ptr);
}

#[test]
fn refresh_rejects_a_malformed_selected_projection_source() {
    let mut past = TemporalMotionField::empty();
    past.set_reference_metadata(true, (0, 0), &[Some(12)]);
    let mut future = TemporalMotionField::empty();
    future.set_reference_metadata(true, (0, 0), &[Some(8)]);
    let mut context = TemporalMvContext::empty();

    let result = context.refresh_from_references(
        (0, 0),
        10,
        TemporalProjectionConfig {
            frame_size: (0, 0),
            step: 1,
            unit_size8: 8,
            enable_tip: false,
            enable_trajectory: false,
            reduced: false,
        },
        &[0, 1],
        &[true, true],
        &[8, 12],
        &[Some(Arc::new(past)), Some(Arc::new(future))],
    );
    assert!(matches!(
        result,
        Err(crate::DecodeError::HeaderState {
            source: crate::DecodeHeaderStateError::InvalidInterTemporalMotionState
        })
    ));
    if let Err(error) = result {
        assert!(crate::DecodeDiagnosticReport::from_decode_error(&error).is_none());
    }
}
