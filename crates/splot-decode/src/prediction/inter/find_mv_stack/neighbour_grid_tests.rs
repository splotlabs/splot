// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! What [`super::NeighbourMvGrid`] records for one leaf's motion.

#![allow(clippy::unwrap_used)]

use super::super::MvBlockContext;
use super::{CWP_EQUAL, INTRABC_REF_FRAME, MotionMode, Mv, warp_sub_mv_at};
use super::{
    NON_INTER_FLAG_SYNTAX, NeighbourFlagSyntax, NeighbourMotionValues, NeighbourMvGrid,
    ZERO_NEIGHBOUR_MOTION_VALUES,
};
use crate::prediction::TileGridConstructionError;

#[test]
fn mv_grid_constructor_classifies_geometry_and_allocation() {
    let reversed = std::ops::Range { start: 5, end: 4 };
    let grid = NeighbourMvGrid::new_for_tile(4..8, 8..12).unwrap();
    assert_eq!((grid.origin_row, grid.origin_col), (4, 8));
    assert_eq!((grid.mi_rows, grid.mi_cols), (4, 4));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(4..4, 8..12),
        Err(TileGridConstructionError::EmptyDimensions)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(reversed, 8..12),
        Err(TileGridConstructionError::ReversedDimensions)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(0..usize::MAX, 0..2),
        Err(TileGridConstructionError::AreaOverflow)
    ));
    assert!(matches!(
        NeighbourMvGrid::new_for_tile(0..usize::MAX, 0..1),
        Err(TileGridConstructionError::Allocation)
    ));
}

/// A 4x4 leaf at the grid origin whose flag half is already published, since
/// the motion plane is only visible through a cell that carries both halves.
fn leaf_grid(motion_mode: MotionMode) -> NeighbourMvGrid {
    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    grid.record_flags(
        0,
        0,
        4,
        4,
        NeighbourFlagSyntax {
            is_inter: true,
            ref_frame0: 0,
            motion_mode,
            ..NON_INTER_FLAG_SYNTAX
        },
    );
    grid
}

/// AV2 § 7.13.3.19 writes `SubMvs` only when `useWarp` is 1 (local warp).
/// Global warp is `useWarp` 2 and writes none, so a `SINGLE_MODE_GLOBALMV`
/// leaf keeps the model its neighbours read while every cell keeps the uniform
/// block MV.
#[test]
fn a_globalmv_single_records_its_mode_without_splatting_sub_mvs() {
    let mv = Mv { row: -20, col: 36 };
    let mut grid = leaf_grid(MotionMode::Simple);

    grid.record_motion(
        0,
        0,
        4,
        4,
        NeighbourMotionValues {
            mv: [mv, Mv::ZERO],
            cwp_weight: CWP_EQUAL,
            stored_warp: None,
            global_mv: [true, false],
            splat_warp: [None, None],
        },
    );

    for (r, c) in [(0, 0), (0, 3), (3, 0), (3, 3)] {
        let cell = grid.get(r, c).unwrap();
        assert_eq!(
            cell.motion.warp_params(),
            None,
            "GLOBALMV does not duplicate frame state at ({r}, {c})"
        );
        assert!(cell.motion.is_global_mv(0));
        assert_eq!(
            cell.motion.sub_mv, mv,
            "no per-cell sub-MV is derived for global warp at ({r}, {c})"
        );
    }
}

/// The same grid does splat when a warp leaf asks it to, so the case above is
/// a deliberate suppression and not a splat that stopped working. The model's
/// linear terms are exaggerated, so two 8x8 units resolve to different sub-MVs
/// through the § 7.13.3.19 rounding.
#[test]
fn a_warp_leaf_still_splats_per_cell_sub_mvs() {
    let model = [131_072, 65_536, 69_632, 4_096, -4_096, 69_632];
    let mv = Mv { row: -20, col: 36 };
    let mut grid = leaf_grid(MotionMode::LocalWarp);

    grid.record_motion(
        0,
        0,
        4,
        4,
        NeighbourMotionValues {
            mv: [mv, Mv::ZERO],
            cwp_weight: CWP_EQUAL,
            stored_warp: Some(model),
            global_mv: [false, false],
            splat_warp: [Some(model), None],
        },
    );

    assert_eq!(
        grid.get(3, 3).unwrap().motion.sub_mv,
        warp_sub_mv_at(model, 0, 0, 3, 3)
    );
    assert_ne!(
        grid.get(0, 0).unwrap().motion.sub_mv,
        grid.get(3, 3).unwrap().motion.sub_mv,
        "a splatted leaf varies its sub-MV across § 7.13.3.19 8x8 units"
    );
}

/// Motion reaches a leaf only through the leaf's own first cell and geometry:
/// a mismatched publication leaves the leaf unresolved rather than attaching
/// motion to it. A list-1 splat model with no list-0 model still splats list 1
/// only, and stores no neighbour-facing model.
#[test]
fn motion_resolves_only_its_own_leaf_and_keeps_per_list_models() {
    let model1 = [131_072, 65_536, 69_632, 4_096, -4_096, 69_632];
    let mv = [Mv { row: -20, col: 36 }, Mv { row: 12, col: -8 }];
    let values = NeighbourMotionValues {
        mv,
        cwp_weight: CWP_EQUAL,
        stored_warp: None,
        global_mv: [false, true],
        splat_warp: [None, Some(model1)],
    };
    let mut grid = leaf_grid(MotionMode::Simple);

    grid.record_motion(0, 0, 2, 2, values);
    assert!(grid.flags_at(1, 1).is_some());
    assert!(
        grid.get(1, 1).is_none(),
        "a mismatched leaf stays unresolved"
    );

    grid.record_motion(0, 0, 4, 4, values);
    let cell = grid.get(3, 3).unwrap();
    assert_eq!(cell.motion.warp_params(), None);
    assert!(cell.motion.is_global_mv(1) && !cell.motion.is_global_mv(0));
    assert_eq!(cell.motion.sub_mv, mv[0]);
    assert_eq!(cell.motion.sub_mv1, warp_sub_mv_at(model1, 0, 0, 3, 3));
}

/// A tile taller than two superblock rows keeps only the current superblock
/// row and the one above it. A reused plane row reads as unpublished, and a
/// read of a row the window dropped fails the tile instead of reading absent.
#[test]
fn the_window_drops_superblock_rows_two_above_the_current_one() {
    let mut grid = NeighbourMvGrid::default();
    grid.reset_for_tile(0..12, 0..2, 4).unwrap();
    let inter = NeighbourFlagSyntax {
        is_inter: true,
        ref_frame0: 0,
        ..NON_INTER_FLAG_SYNTAX
    };
    grid.record_flags(0, 0, 2, 4, inter);
    grid.record_flags(4, 0, 2, 4, inter);
    assert!(grid.flags_at(0, 0).is_some() && grid.flags_at(7, 1).is_some());
    grid.record_flags(8, 0, 2, 2, inter);
    assert!(grid.flags_at(7, 1).is_some(), "the superblock row above");
    assert!(grid.flags_at(9, 1).is_some());
    assert!(grid.flags_at(10, 0).is_none(), "reused row is cleared");
    assert!(!grid.window_violated());
    assert!(grid.flags_at(3, 0).is_none(), "two superblock rows above");
    assert!(
        grid.window_violated(),
        "the dropped row is a fail-closed defect"
    );
}

/// A checkpoint taken since an older one keeps its leaf records after a
/// rollback to the older one and another leaf's publication, as an encoder
/// search does when it restores the best of several trials.
#[test]
fn a_checkpoint_since_an_older_one_restores_its_own_leaf() {
    let publish = |grid: &mut NeighbourMvGrid, mv: Mv| {
        grid.record_flags(
            0,
            0,
            4,
            4,
            NeighbourFlagSyntax {
                is_inter: true,
                ref_frame0: 0,
                ..NON_INTER_FLAG_SYNTAX
            },
        );
        grid.record_motion(
            0,
            0,
            4,
            4,
            NeighbourMotionValues {
                mv: [mv, Mv::ZERO],
                cwp_weight: CWP_EQUAL,
                stored_warp: None,
                global_mv: [false, false],
                splat_warp: [None, None],
            },
        );
    };
    let (best, other) = (Mv { row: -20, col: 36 }, Mv { row: 8, col: -12 });
    let mut grid = NeighbourMvGrid::new(16, 16).unwrap();
    let start = grid.checkpoint((0, 0, 4, 4), None);
    publish(&mut grid, best);
    let end = grid.checkpoint((0, 0, 4, 4), Some(&start));
    grid.rollback(&start);
    assert!(grid.get(0, 0).is_none());
    publish(&mut grid, other);
    grid.rollback(&end);
    assert_eq!(grid.get(3, 3).unwrap().motion.sub_mv, best);
}

/// Publishes an inter leaf with motion `mv` and, when given, a stored warp
/// model of its own.
fn publish(
    grid: &mut NeighbourMvGrid,
    (r, c, n4w, n4h): (usize, usize, usize, usize),
    mv: Mv,
    model: Option<[i32; 6]>,
) {
    let motion_mode = if model.is_some() {
        MotionMode::LocalWarp
    } else {
        MotionMode::Simple
    };
    let syntax = NeighbourFlagSyntax {
        is_inter: true,
        ref_frame0: 1,
        motion_mode,
        ..NON_INTER_FLAG_SYNTAX
    };
    grid.record_flags(r, c, n4w, n4h, syntax);
    let values = NeighbourMotionValues {
        mv: [mv, Mv::ZERO],
        cwp_weight: CWP_EQUAL,
        stored_warp: model,
        global_mv: [false, false],
        splat_warp: [None, None],
    };
    grid.record_motion(r, c, n4w, n4h, values);
}

/// An edge taken past the tile's last column keeps only the tile's columns,
/// and its leaves and warp models land renumbered behind the importing
/// grid's own, with every other cell of that grid untouched.
#[test]
fn an_edge_imports_its_cells_leaves_and_models_into_another_grid() {
    let (first, second) = (
        [65_536, 0, 0, 65_536, 12, -4],
        [65_536, 8, -8, 65_536, 3, 7],
    );
    let mut source = NeighbourMvGrid::new_for_tile(0..8, 0..10).unwrap();
    publish(
        &mut source,
        (0, 0, 10, 4),
        Mv { row: 2, col: -6 },
        Some(first),
    );
    publish(&mut source, (4, 0, 4, 4), Mv { row: -3, col: 5 }, None);
    publish(
        &mut source,
        (4, 4, 6, 4),
        Mv { row: 9, col: 1 },
        Some(second),
    );
    let edge = source.edge(6..8, 4..16);
    let mut target = NeighbourMvGrid::new_for_tile(0..8, 0..10).unwrap();
    publish(
        &mut target,
        (0, 0, 4, 2),
        Mv { row: 1, col: 1 },
        Some(first),
    );
    target.import_edge(&edge);
    for r in 0..8 {
        for c in 0..10 {
            let expected = match (r, c) {
                (6..8, 4..10) => source.get(r, c),
                (0..2, 0..4) => target.get(0, 0),
                _ => None,
            };
            assert_eq!(target.get(r, c), expected, "({r}, {c})");
        }
    }
    assert_eq!(
        target.get(7, 9).and_then(|cell| cell.motion.warp_params()),
        Some(second)
    );
}

/// The direct index agrees with the window's own row mapping at every
/// position around a windowed tile that starts away from the frame origin.
#[test]
fn the_direct_index_matches_the_window_row_mapping() {
    let mut grid = NeighbourMvGrid::default();
    grid.reset_for_tile(8..40, 4..9, 8).unwrap();
    for sb_row in [8, 16, 24, 32] {
        grid.enter_sb_row(sb_row);
        for r in -2..44 {
            for c in -2..12 {
                let expected = usize::try_from(r)
                    .ok()
                    .and_then(|r| grid.plane_row(r))
                    .zip(usize::try_from(c - 4).ok().filter(|&col| col < 5))
                    .map(|(row, col)| row * 5 + col);
                assert_eq!(grid.index(r, c), expected, "({r}, {c}) in row {sb_row}");
            }
        }
    }
}

/// An edges-only grid reads the same immediate and
/// superblock-border probes as a grid that keeps the plane, for leaves of
/// every shape published in partition order, odd columns on a superblock
/// row's top included.
#[test]
fn edge_lines_read_the_probes_of_the_plane() {
    let mut plane = NeighbourMvGrid::default();
    plane.reset_for_tile(0..16, 0..16, 8).unwrap();
    let mut edges = NeighbourMvGrid::default();
    edges.reset_for_tile(0..16, 0..16, 8).unwrap();
    edges.track_edges().unwrap();
    edges.keep_edges_only();
    let leaves = [
        (0, 0, 4, 4),
        (0, 4, 4, 4),
        (4, 0, 4, 4),
        (4, 4, 2, 2),
        (4, 6, 2, 2),
        (6, 4, 2, 2),
        (6, 6, 2, 2),
        (0, 8, 4, 8),
        (0, 12, 4, 8),
        (8, 0, 1, 1),
        (8, 1, 1, 1),
        (9, 0, 1, 1),
        (9, 1, 1, 1),
        (8, 2, 2, 2),
        (10, 0, 2, 2),
        (10, 2, 2, 2),
        (8, 4, 4, 4),
        (12, 0, 8, 4),
        (8, 8, 1, 4),
        (8, 9, 1, 4),
        (8, 10, 2, 4),
        (8, 12, 4, 4),
        (12, 8, 8, 4),
    ];
    let motion_modes = [
        MotionMode::Simple,
        MotionMode::InterIntra,
        MotionMode::LocalWarp,
        MotionMode::DeltaWarp,
        MotionMode::ExtendWarp,
    ];
    for (index, (r, c, n4w, n4h)) in leaves.into_iter().enumerate() {
        assert_eq!(
            plane.immediate_flags(r, c, n4w, n4h),
            edges.immediate_flags(r, c, n4w, n4h),
            "immediate probes of leaf {index}"
        );
        if r % 8 == 0 {
            let up = r.wrapping_sub(1);
            for cols in [
                [c.wrapping_sub(1), c],
                [c + n4w - 1, c + n4w.saturating_sub(2)],
            ] {
                assert_eq!(
                    plane.sb_border_flags(c, up, cols),
                    edges.sb_border_flags(c, up, cols),
                    "border probes of leaf {index}"
                );
            }
        }
        let syntax = NeighbourFlagSyntax {
            is_inter: index % 4 != 3,
            ref_frame0: (index % 7) as i8,
            ref_frame1: (index % 3 == 0).then_some(6),
            skip: index % 2 == 0,
            interp_filter: (index % 3) as u8,
            motion_mode: motion_modes[index % 5],
            ..NON_INTER_FLAG_SYNTAX
        };
        plane.record_flags(r, c, n4w, n4h, syntax);
        edges.record_flags(r, c, n4w, n4h, syntax);
    }
    assert!(!plane.window_violated() && !edges.window_violated());
}

/// Partition-order leaves of one `size` square: whole, two halves, or four
/// recursive quarters, chosen from `seed`.
fn partition_leaves(r: usize, c: usize, size: usize, seed: &mut u32, out: &mut Vec<[usize; 4]>) {
    *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    let half = size / 2;
    match (size, (*seed >> 16) % 4) {
        (1, _) | (_, 0) => out.push([r, c, size, size]),
        (_, 1) => out.extend([[r, c, size, half], [r + half, c, size, half]]),
        (_, 2) => out.extend([[r, c, half, size], [r, c + half, half, size]]),
        _ => {
            for (dr, dc) in [(0, 0), (0, half), (half, 0), (half, half)] {
                partition_leaves(r + dr, c + dc, half, seed, out);
            }
        }
    }
}

/// The inter-frame `use_intrabc` / `skip` contexts read from the parse's edge
/// lines, and the spatial IntraBC probes the resolve pass captures over the
/// leaves it has resolved, match what the IntraBC prelude grid reads.
#[test]
fn neighbour_grid_reads_the_intrabc_prelude_neighbours() {
    use crate::filters::wienerns_lr::intrabc_records::{
        IntrabcBlockContext, IntrabcBlockGeometry, IntrabcBlockPrelude, IntrabcUseSkip,
        TileIntrabcPreludeState,
    };

    let mut sequence = splot_core::headers::frame::build_minimal_intra_sequence_header().unwrap();
    let partition = sequence.partition.as_mut().unwrap();
    partition.use_128x128_superblock = false;
    partition.use_256x256_superblock = false;
    let (sb4, rows, cols) = (16, 32, 48);
    let mut state = TileIntrabcPreludeState::new_for_tile(
        (rows, cols),
        0..rows,
        0..cols,
        &sequence,
        false,
        true,
    )
    .unwrap();
    let mut edges = NeighbourMvGrid::default();
    edges.reset_for_tile(0..rows, 0..cols, sb4).unwrap();
    edges.track_edges().unwrap();
    edges.keep_edges_only();
    let mut plane = NeighbourMvGrid::default();
    plane.reset_for_tile(0..rows, 0..cols, sb4).unwrap();
    let mut seed = 7;
    for (sb_r, sb_c) in (0..rows / sb4).flat_map(|r| (0..cols / sb4).map(move |c| (r, c))) {
        let mut leaves = Vec::new();
        partition_leaves(sb_r * sb4, sb_c * sb4, sb4, &mut seed, &mut leaves);
        let syntax = |index: usize| NeighbourFlagSyntax {
            is_inter: !index.is_multiple_of(4),
            ref_frame0: if index % 4 == 1 {
                INTRABC_REF_FRAME
            } else {
                (index % 7) as i8
            },
            skip: !index.is_multiple_of(3),
            ..NON_INTER_FLAG_SYNTAX
        };
        let base = seed as usize;
        for (index, &[r, c, n4w, n4h]) in leaves.iter().enumerate() {
            plane.record_flags(r, c, n4w, n4h, syntax(base + index));
        }
        for (index, [r, c, n4w, n4h]) in leaves.into_iter().enumerate() {
            let flags = syntax(base + index);
            let block = MvBlockContext {
                mi_row: r,
                mi_col: c,
                bw4: n4w,
                bh4: n4h,
                sb_h4: sb4,
                ref_frame0: 0,
                ref_frame1: None,
                mi_rows: rows,
                mi_cols: cols,
            };
            let ctx = super::super::block_neighbour_ctx(&edges, &block);
            assert_eq!(
                ctx.intrabc_ctx(),
                state.intrabc_ctx(r, c, n4w, n4h).unwrap()
            );
            assert_eq!(ctx.skip_ctx, state.skip_ctx(r, c, n4w, n4h).unwrap());
            let geometry =
                IntrabcBlockGeometry::new(IntrabcBlockContext::new(r, c, 0, false), n4w, n4h);
            assert_eq!(
                plane.capture_spatial_intrabc_probes(&block),
                state.capture_spatial_intrabc_probes(geometry),
                "probes of leaf ({r}, {c}) {n4w}x{n4h}"
            );
            let mut prelude = IntrabcBlockPrelude::from_use_skip(IntrabcUseSkip {
                use_intrabc: flags.ref_frame0 == INTRABC_REF_FRAME,
                skip_flag: flags.skip,
            });
            prelude.is_inter = flags.is_inter;
            state.record_block(r, c, n4w, n4h, prelude).unwrap();
            edges.record_flags(r, c, n4w, n4h, flags);
            plane.record_motion(r, c, n4w, n4h, ZERO_NEIGHBOUR_MOTION_VALUES);
        }
    }
    assert!(!state.window_violated() && !plane.window_violated());
}
