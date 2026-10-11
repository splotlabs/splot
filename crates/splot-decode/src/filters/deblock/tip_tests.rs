// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::tests::with_plane_ctx;
use super::*;

/// Filters the TIP unit edges of one 4:2:0 plane edge by edge, in the old
/// order: each row of 4-line vertical edges, then each column of 4-column
/// horizontal edges.
fn per_edge_tip_plane<T: ReconSample>(
    ws: &mut CurrentFrameWorkspace<T>,
    plane: usize,
    luma_unit_size: usize,
    bit_depth: BitDepth,
) {
    let sub = usize::from(plane != 0);
    let unit = luma_unit_size >> sub;
    let (q_thr, side) = adaptive_strength(q_clamped(200, 0, bit_depth), bit_depth);
    with_plane_ctx(ws, plane_index_to_id(plane), |ctx| {
        let (width, height) = (ctx.width, ctx.height);
        let max_widths = deblock_filter_max_width(unit, plane != 0, false);
        for y in (0..height).step_by(MI_SIZE) {
            for x in (unit..width).step_by(unit) {
                let lanes = (height - y).min(MI_SIZE);
                apply_tip_filter_edge(ctx, x, y, 1, 0, lanes, q_thr, side, max_widths, bit_depth)
                    .unwrap();
            }
        }
        for x in (0..width).step_by(MI_SIZE) {
            for y in (unit..height).step_by(unit) {
                let max_widths =
                    deblock_filter_max_width(unit, plane != 0, y.is_multiple_of(64 >> sub));
                let lanes = (width - x).min(MI_SIZE);
                apply_tip_filter_edge(ctx, x, y, 0, 1, lanes, q_thr, side, max_widths, bit_depth)
                    .unwrap();
            }
        }
    });
}

/// TIP deblocking runs match the per-edge filter for both sample types and
/// unit sizes, for edges near the plane border, partial last edges, and
/// planes too short or too narrow for one full edge.
fn tip_runs_match_per_edge_filter<T: ReconSample>(bit_depth: BitDepth) {
    let filled = |width, height| {
        let mut ws = crate::pipeline::reconstruct::new_general_intra_workspace::<T>(
            width,
            height,
            bit_depth,
            PixelFormat::Yuv420,
        )
        .unwrap();
        let mut state = 0x2545_f491_u32;
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            let (plane_width, plane_height) = coded_plane_dimensions(&ws, plane).unwrap();
            for y in 0..plane_height {
                for x in 0..plane_width {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let unit = ((x / 4 + y / 4) % 3) as u16 * 3;
                    let sample = (100 + unit + (state >> 29) as u16) << (bit_depth.bits() - 8);
                    ws.set_reconstructed_sample(plane, x, y, T::try_from_u16(sample).unwrap())
                        .unwrap();
                }
            }
        }
        ws
    };
    let samples = |ws: &CurrentFrameWorkspace<T>, plane| -> Vec<u16> {
        let samples = ws.plane(plane_index_to_id(plane)).unwrap().samples();
        samples.iter().map(|&sample| sample.to_u16()).collect()
    };
    for (width, height) in [(64, 48), (46, 38), (40, 6), (6, 40)] {
        for luma_unit_size in [8, 16] {
            let mut actual = filled(width, height);
            let original = samples(&actual, 0);
            deblock_tip_frame(
                &mut actual,
                luma_unit_size,
                QuantizationParams::inferred_tip(200, 0, 0),
                0,
                None,
                false,
                bit_depth,
            )
            .unwrap();
            let mut expected = filled(width, height);
            for plane in 0..3 {
                per_edge_tip_plane(&mut expected, plane, luma_unit_size, bit_depth);
                assert_eq!(
                    samples(&actual, plane),
                    samples(&expected, plane),
                    "{bit_depth:?} {width}x{height} unit {luma_unit_size} plane {plane}"
                );
            }
            assert_ne!(samples(&actual, 0), original);
        }
    }
}

#[test]
fn tip_deblocking_runs_match_per_edge_filter() {
    tip_runs_match_per_edge_filter::<u8>(BitDepth::Eight);
    tip_runs_match_per_edge_filter::<u16>(BitDepth::Ten);
}
