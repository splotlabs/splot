// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{NUM_TAPS, horizontal_8tap_row_u16, horizontal_8tap_row_u16_reference};

/// Shapes of real `Subpel_Filters` rows: even coefficients summing to 128, with
/// the sign pattern and magnitude range § 7.13.3.18 uses.
const ROWS: [[i32; NUM_TAPS]; 4] = [
    [-2, 6, -12, 84, 68, -14, 6, -8],
    [0, 2, -10, 52, 100, -16, 2, -2],
    [2, -4, 8, 118, 12, -6, 4, -6],
    [-4, 10, -20, 64, 88, -24, 12, 2],
];

/// The block widths the AV2 partition tree produces, plus widths that leave a
/// four-lane group and a scalar tail.
const WIDTHS: [usize; 12] = [2, 3, 4, 5, 6, 7, 8, 12, 16, 32, 64, 128];

fn taps_for(row: usize, tap_start: usize, tap_end: usize) -> [i32; NUM_TAPS] {
    let mut taps = [0i32; NUM_TAPS];
    taps[tap_start..tap_end].copy_from_slice(&ROWS[row % ROWS.len()][tap_start..tap_end]);
    taps
}

fn source(len: usize, bit_depth: u32, seed: u32) -> Vec<u16> {
    let max = (1u32 << bit_depth) - 1;
    (0..len as u32)
        .map(|index| {
            let mixed = index.wrapping_mul(2_654_435_761).wrapping_add(seed);
            match index % 5 {
                0 => 0,
                1 => max as u16,
                _ => (mixed % (max + 1)) as u16,
            }
        })
        .collect()
}

#[test]
fn horizontal_matches_the_reference_over_every_span_width_and_depth() {
    let mut taken = 0usize;
    for bit_depth in [8u32, 10] {
        for (row, &width) in WIDTHS.iter().enumerate() {
            for tap_start in 0..=NUM_TAPS - 2 {
                for tap_end in tap_start + 2..=NUM_TAPS {
                    let taps = taps_for(row + tap_start, tap_start, tap_end);
                    let window = source(width + NUM_TAPS + 24, bit_depth, tap_end as u32);
                    let mut expected = vec![0i16; width];
                    horizontal_8tap_row_u16_reference(
                        &window,
                        &taps,
                        tap_start,
                        tap_end,
                        3,
                        &mut expected,
                    );
                    let mut actual = vec![i16::MIN; width];
                    if horizontal_8tap_row_u16(&window, &taps, tap_start, tap_end, 3, &mut actual) {
                        taken += 1;
                        assert_eq!(
                            actual, expected,
                            "depth {bit_depth} width {width} span {tap_start}..{tap_end}"
                        );
                    }
                }
            }
        }
    }
    assert_eq!(taken, expected_dispatch_count(), "dispatch coverage");
}

#[test]
fn the_horizontal_kernel_refuses_shapes_it_cannot_serve() {
    let taps = taps_for(0, 0, NUM_TAPS);
    let window = source(16, 10, 0);
    let mut row_out = [0i16; 8];
    assert!(!horizontal_8tap_row_u16(
        &window[..14],
        &taps,
        0,
        NUM_TAPS,
        3,
        &mut row_out
    ));
    assert!(!horizontal_8tap_row_u16(
        &window,
        &taps,
        0,
        NUM_TAPS,
        4,
        &mut row_out
    ));
    assert!(!horizontal_8tap_row_u16(
        &window,
        &taps,
        0,
        NUM_TAPS + 1,
        3,
        &mut row_out
    ));
    assert_eq!(row_out, [0i16; 8], "a refused shape writes nothing");
}

/// The number of `(width, span)` combinations the dispatch is expected to take
/// on this build. Zero off the hand-scheduled targets, where the caller keeps
/// its own portable path; every combination on them, so a kernel that quietly
/// stops being reached fails the differential tests instead of passing them
/// vacuously.
fn expected_dispatch_count() -> usize {
    if cfg!(all(target_arch = "aarch64", target_feature = "neon")) {
        2 * WIDTHS.len()
            * (2..=NUM_TAPS)
                .map(|span| NUM_TAPS + 1 - span)
                .sum::<usize>()
    } else {
        0
    }
}
