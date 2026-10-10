// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use std::sync::Arc;

use super::super::tests::{frame_for, workspace_for};
use super::*;
use crate::pipeline::frame_progress::FrameProgress;
use splot_recon::{BitDepth, DecodedFrame, PixelFormat};

const WIDTH: usize = 64;
const HEIGHT: usize = 48;

fn mv(row: i32, col: i32) -> Mv {
    Mv { row, col }
}

/// A reference whose luma is a textured base plus `edits` of `(x, y, delta)`.
fn reference<T: ReconSample>(
    bit_depth: BitDepth,
    edits: &[(usize, usize, u16)],
) -> DecodedFrame<T> {
    let max = usize::from(bit_depth.max_sample());
    let mut luma: Vec<u16> = (0..WIDTH * HEIGHT)
        .map(|i| ((i * 37 + i / WIDTH * 11) % (max / 2)) as u16)
        .collect();
    for &(x, y, delta) in edits {
        luma[y * WIDTH + x] += delta;
    }
    frame(bit_depth, luma)
}

/// A smooth luma ramp moved `dx` samples left, so optical flow sees motion.
fn ramp<T: ReconSample>(bit_depth: BitDepth, dx: usize) -> DecodedFrame<T> {
    let luma = (0..WIDTH * HEIGHT).map(|i| (2 * (i % WIDTH + dx) + i / WIDTH) as u16);
    frame(bit_depth, luma.collect())
}

fn frame<T: ReconSample>(bit_depth: BitDepth, luma: Vec<u16>) -> DecodedFrame<T> {
    let sample = |value: u16| T::try_from_u16(value).expect("sample");
    let chroma = vec![sample(64); (WIDTH / 2) * (HEIGHT / 2)];
    frame_for(
        bit_depth,
        PixelFormat::Yuv420,
        WIDTH,
        HEIGHT,
        luma.into_iter().map(sample).collect(),
        chroma.clone(),
        chroma,
    )
}

#[derive(Clone, Copy)]
struct Case {
    x: usize,
    mv: Mv,
    search: bool,
    refine: bool,
    threshold: Option<u32>,
    fast: bool,
}

fn check<T: ReconSample>(references: &[DecodedFrame<T>; 2], case: Case) {
    check_at(
        references.each_ref().map(ReferenceSamples::settled),
        16,
        case,
    );
}

/// Compares the fast cell of the unit at (`case.x`, `y`) with the full path,
/// and checks whether the fast path decided it.
fn check_at<T: ReconSample>(references: [ReferenceSamples<'_, T>; 2], y: usize, case: Case) {
    let bit_depth = references[0].info().bit_depth();
    let mut workspace = workspace_for::<T>(bit_depth, PixelFormat::Yuv420, WIDTH, HEIGHT);
    let sink = WorkspaceSink::Frame(&mut workspace);
    let offset = ByteOffset::new(0);
    let rect = McBlockRect::from_luma_rect(case.x, y, 8, 8);
    let mvs = [case.mv; 2];
    let block = unit_block(references, rect, mvs, case);
    let fast = TipFullpelViews::new(&sink, &block, (rect, mvs), offset)
        .expect("views")
        .motion_cell(&sink, &block, (rect, mvs), 8)
        .expect("fast cell");
    let full = if block.search_refinemv && search_range_allowed(mvs) {
        tip_refinemv_optflow_motion_cell(&sink, block, offset, [false; 2], &mut [[0; 256]; 2])
            .expect("refined cell")
            .expect("searched unit")
    } else {
        super::super::optflow::tip_unit_motion_cell(&sink, block, 8, offset).expect("unit cell")
    };
    assert_eq!(
        fast.is_some(),
        case.fast,
        "{bit_depth:?} x {} {:?}",
        case.x,
        case.mv
    );
    if let Some(fast) = fast {
        assert_eq!(fast, full, "{bit_depth:?} x {} {:?}", case.x, case.mv);
    }
}

fn unit_block<T: ReconSample>(
    references: [ReferenceSamples<'_, T>; 2],
    rect: McBlockRect,
    mvs: [Mv; 2],
    case: Case,
) -> CompoundMcBlock<'_, T> {
    InterBlockParams::compound_average(
        references[0],
        references[1],
        rect,
        mvs[0],
        mvs[1],
        InterpolationFilter::EightTap,
        CompoundBlend::default(),
    )
    .with_optflow_distances(Some([1, -1]))
    .with_optflow_sad_threshold(case.threshold)
    .with_refinemv(case.refine)
    .with_refinemv_search(case.search)
    .into_compound()
    .expect("compound block")
}

/// Covers both paths at plane and refine-window edges, a subpel candidate,
/// no SAD threshold, an optical-flow SAD on each side of the threshold (one
/// sample of row `Y + 1`, outside the centre rows) and a centre SAD on each
/// side of 288 (row `Y - 2`, outside the 8x8 rows; 328 keeps the candidates).
fn fast_cells_match_the_full_path<T: ReconSample>(bit_depth: BitDepth) {
    let shift = u16::from(bit_depth.bits() - 8);
    let base = |search, refine, threshold, fast| Case {
        x: 16,
        mv: mv(0, 0),
        search,
        refine,
        threshold,
        fast,
    };
    let still = [
        reference::<T>(bit_depth, &[]),
        reference::<T>(bit_depth, &[]),
    ];
    for case in [
        base(true, true, Some(6), true),
        base(false, true, Some(6), true),
        base(false, false, Some(6), true),
        base(true, true, None, false),
        base(false, true, None, true),
        base(false, false, None, true),
        Case {
            mv: mv(0, 4),
            ..base(true, true, Some(6), false)
        },
        Case {
            mv: mv(0, 4),
            ..base(false, false, Some(6), false)
        },
        Case {
            mv: mv(-8, -14 * 8),
            ..base(true, true, Some(6), true)
        },
        Case {
            mv: mv(-8, -15 * 8),
            ..base(true, true, Some(6), false)
        },
        Case {
            mv: mv(8, -16 * 8),
            ..base(false, true, Some(6), true)
        },
        Case {
            mv: mv(8, -17 * 8),
            ..base(false, true, Some(6), false)
        },
        Case {
            mv: mv(-2 * 8, 0),
            ..base(false, false, Some(6), true)
        },
        Case {
            mv: mv(23 * 8, 38 * 8),
            ..base(true, true, Some(6), true)
        },
        Case {
            mv: mv(23 * 8, 39 * 8),
            ..base(true, true, Some(6), false)
        },
        Case {
            mv: mv(24 * 8, 38 * 8),
            ..base(true, true, Some(6), false)
        },
        Case {
            mv: mv(0, 40 * 8),
            ..base(false, true, Some(6), true)
        },
        Case {
            mv: mv(0, 41 * 8),
            ..base(false, false, Some(6), false)
        },
        Case {
            mv: mv(28 * 8, 0),
            ..base(false, true, Some(6), true)
        },
        Case {
            mv: mv(-19 * 8, 0),
            ..base(false, false, None, true)
        },
    ] {
        check(&still, case);
    }
    for (delta, fast) in [(5u16, true), (6, false)] {
        let refs = [
            reference::<T>(bit_depth, &[]),
            reference::<T>(bit_depth, &[(19, 17, delta << shift)]),
        ];
        check(&refs, base(true, true, Some(6), fast));
        check(&refs, base(false, true, Some(6), true));
    }
    let refs = [ramp::<T>(bit_depth, 0), ramp::<T>(bit_depth, 1)];
    for (refine, threshold) in [(true, Some(6)), (false, Some(6)), (false, None)] {
        for row in [0, -19 * 8, 28 * 8] {
            check(
                &refs,
                Case {
                    mv: mv(row, 0),
                    ..base(false, refine, threshold, true)
                },
            );
        }
    }
    for (total, fast) in [(328u16, true), (329, false)] {
        let edits = [
            (14, 14, 128 << shift),
            (16, 14, 128 << shift),
            (18, 14, (total - 256) << shift),
        ];
        let refs = [
            reference::<T>(bit_depth, &[]),
            reference::<T>(bit_depth, &edits),
        ];
        check(&refs, base(true, true, Some(6), fast));
    }
}

#[test]
fn tip_fullpel_cells_match_the_full_path() {
    fast_cells_match_the_full_path::<u8>(BitDepth::Eight);
    fast_cells_match_the_full_path::<u16>(BitDepth::Ten);
}

/// The texture of [`reference`] with only its first `rows` rows published,
/// as a frame-parallel reference still filtering below them.
fn partly_published(rows: usize) -> Arc<FrameProgress<u8>> {
    let texture = reference::<u8>(BitDepth::Eight, &[]);
    let progress = Arc::new(FrameProgress::new(texture.info()).expect("progress"));
    progress
        .begin(&[(0, rows), (rows, HEIGHT)])
        .expect("stripes");
    let mut lease = progress.direct_stripe(0).expect("stripe lease");
    let mut target = lease.take_target().expect("stripe target");
    for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
        let source = texture.plane(plane).expect("plane").samples();
        let mut destination = target.take(plane).expect("plane target");
        let samples = destination.u8_samples_mut().expect("8-bit stripe");
        samples.copy_from_slice(&source[..samples.len()]);
    }
    assert!(lease.submit());
    progress
}

/// Units inside a 24-row published prefix match the full path; a unit
/// whose rows pass it falls back, through views built for an earlier unit.
#[test]
fn tip_fullpel_cells_read_only_the_published_prefix() {
    let progress = partly_published(24);
    let published = progress.read().expect("published frame");
    let references = [ReferenceSamples::publishing(&published).expect("reference"); 2];
    let case = |mv, search, fast| Case {
        x: 16,
        mv,
        search,
        refine: true,
        threshold: Some(6),
        fast,
    };
    check_at(references, 8, case(mv(0, 0), true, true));
    check_at(references, 8, case(mv(0, 0), false, true));
    let mut workspace = workspace_for::<u8>(BitDepth::Eight, PixelFormat::Yuv420, WIDTH, HEIGHT);
    let sink = WorkspaceSink::Frame(&mut workspace);
    let first = McBlockRect::from_luma_rect(16, 8, 8, 8);
    let past = McBlockRect::from_luma_rect(16, 16, 8, 8);
    let still = [mv(0, 0); 2];
    let block = unit_block(references, first, still, case(mv(0, 0), false, true));
    let views =
        TipFullpelViews::new(&sink, &block, (first, still), ByteOffset::new(0)).expect("views");
    for mvs in [[mv(8, 0); 2], [mv(0, 0), mv(8, 0)]] {
        let fast = views.motion_cell(&sink, &block, (past, mvs), 8);
        assert_eq!(fast.expect("fast cell"), None, "{mvs:?}");
    }
}
