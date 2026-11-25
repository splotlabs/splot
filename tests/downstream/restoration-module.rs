// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Standalone downstream oracle and timing driver for the § 7.20.3 luma run.

use std::hint::black_box;
use std::time::Instant;

use splot_core::simd::{ResolvedBackend, SimdBackend, SimdPolicy, resolve_backend};
use splot_lr_kernel::{Classes, LaneClass};

// Each coefficient multiplies the samples at (dy, dx) and (-dy, -dx).
const MIN: [i16; 16] = [
    -24, -24, -14, -14, -16, -16, -8, -8, -8, -8, -8, -8, -8, -8, -8, -8,
];
const BITS: [u32; 16] = [6, 6, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4];
const TAPS: [(isize, isize); 16] = [
    (1, 0),
    (0, 1),
    (2, 0),
    (0, 2),
    (1, 1),
    (-1, 1),
    (2, 1),
    (2, -1),
    (1, 2),
    (1, -2),
    (3, 0),
    (0, 3),
    (4, 0),
    (0, 4),
    (3, 3),
    (3, -3),
];

struct Case {
    samples: Vec<u16>,
    pitch: usize,
    rows: usize,
    span: (usize, usize),
    stride: usize,
    max: u16,
    classes: [LaneClass; 4],
    ids: Vec<u8>,
    uniform: bool,
}

impl Case {
    fn new(seed: usize) -> Self {
        let groups = 1 + seed % 7;
        let rows = 1 + (seed / 7) % 8;
        let start = seed % 4;
        let width = 16 * groups;
        let end = start + width + seed % 5;
        let pitch = end + 8 + seed % 3;
        let max = if seed % 5 == 0 { 255 } else { 1023 };
        let mut state = seed as u32 + 1;
        let mut sample = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 12) as u16) & max
        };
        let samples = (0..pitch * (rows + 8)).map(|_| sample()).collect();
        let classes = std::array::from_fn(|class| {
            let active = 1 + (seed + 3 * class) % TAPS.len();
            let coefficients = std::array::from_fn(|i| {
                if i < active {
                    match (seed + class) % 3 {
                        0 => MIN[i],
                        1 => MIN[i] + (1 << BITS[i]) - 1,
                        _ => MIN[i] + (sample() % (1 << BITS[i])) as i16,
                    }
                } else {
                    0
                }
            });
            (coefficients, 128 - 2 * coefficients.iter().sum::<i16>())
        });
        let ids = (0..groups * 4).map(|_| sample() as u8).collect();
        Self {
            samples,
            pitch,
            rows,
            span: (start, end),
            stride: end + 3,
            max,
            classes,
            ids,
            uniform: seed % 2 == 0,
        }
    }

    fn weights(&self) -> Classes<'_> {
        if self.uniform {
            Classes::Uniform(&self.classes[0])
        } else {
            Classes::Cells {
                cells: &self.ids,
                table: &self.classes,
            }
        }
    }

    fn oracle(&self, output: &mut [u16]) {
        let sample = |y: usize, x: usize| i32::from(self.samples[y * self.pitch + x]);
        let (start, end) = self.span;
        for y in 0..self.rows {
            for x in start..start + ((end - start) / 16) * 16 {
                let class = if self.uniform {
                    0
                } else {
                    usize::from(self.ids[(x - start) / 4]) % self.classes.len()
                };
                let center = sample(y + 4, x + 4);
                let mut total = center * 128;
                for (&(dy, dx), &coeff) in TAPS.iter().zip(&self.classes[class].0) {
                    let at = |dy, dx| {
                        sample(
                            (y + 4).wrapping_add_signed(dy),
                            (x + 4).wrapping_add_signed(dx),
                        )
                    };
                    total += i32::from(coeff) * (at(dy, dx) + at(-dy, -dx) - 2 * center);
                }
                output[y * self.stride + x] =
                    ((total + 64) >> 7).clamp(0, i32::from(self.max)) as u16;
            }
        }
    }
}

fn main() {
    let iterations: usize = std::env::args()
        .nth(1)
        .map_or(0, |text| text.parse().unwrap());
    let ResolvedBackend::Avx2(backend) = resolve_backend(SimdPolicy::Avx2).unwrap() else {
        panic!("expected the requested AVX2 backend");
    };
    let token = backend.token();
    let cases: Vec<_> = (0..64).map(Case::new).collect();
    let windows: Vec<Vec<_>> = cases
        .iter()
        .map(|c| c.samples.chunks_exact(c.pitch).collect())
        .collect();
    let mut scratch = vec![0_u16; cases.iter().map(|c| c.stride * c.rows).max().unwrap()];
    for (case, window) in cases.iter().zip(&windows) {
        scratch.fill(0xa55a);
        let mut expected = scratch.clone();
        case.oracle(&mut expected);
        assert!(splot_lr_avx2::filter_luma_run(
            token,
            window,
            case.rows,
            case.span,
            case.weights(),
            (&mut scratch, case.stride),
            case.max
        ));
        assert_eq!(scratch, expected);
        assert!(!splot_lr_avx2::filter_luma_run(
            token,
            &window[..window.len() - 1],
            case.rows,
            case.span,
            case.weights(),
            (&mut scratch, case.stride),
            case.max
        ));
    }
    let first = &cases[0];
    scratch.fill(0xa55a);
    assert!(splot_lr_avx2::filter_luma_run(
        token,
        &[],
        0,
        (0, 16),
        first.weights(),
        (&mut scratch, 0),
        first.max
    ));
    assert!(splot_lr_avx2::filter_luma_run(
        token,
        &[],
        2,
        (3, 15),
        first.weights(),
        (&mut scratch, 0),
        first.max
    ));
    assert!(scratch.iter().all(|&word| word == 0xa55a));
    assert!(!splot_lr_avx2::filter_luma_run(
        token,
        &windows[0],
        1,
        (0, 16),
        Classes::Cells {
            cells: &[0; 4],
            table: &[]
        },
        (&mut scratch, first.stride),
        first.max
    ));
    println!("64 scalar-oracle cases and shape checks passed");
    if iterations == 0 {
        return;
    }
    let mut sum = 0_u64;
    let start = Instant::now();
    for i in 0..iterations {
        let id = i % cases.len();
        let case = &cases[id];
        assert!(splot_lr_avx2::filter_luma_run(
            token,
            black_box(&windows[id]),
            case.rows,
            case.span,
            case.weights(),
            (black_box(&mut scratch), case.stride),
            case.max
        ));
        sum = sum.wrapping_add(u64::from(black_box(scratch[case.span.0])));
    }
    println!(
        "ns/run {:.3} checksum {sum}",
        start.elapsed().as_secs_f64() * 1e9 / iterations as f64
    );
}
