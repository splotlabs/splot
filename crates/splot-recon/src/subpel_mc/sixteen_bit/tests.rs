// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The NEON word-lane kernels against the portable kernels.

use super::super::block::{Block, Sink, TAPS, Taps, stage_len};
use super::super::{NUM_FILTER_TYPES, portable};

const STRIDE: usize = 160;
const ROWS: usize = 48;

/// Planes of `max`-bounded samples: a mixed texture and an extreme-contrast
/// one that reaches the largest tap sums.
fn planes(max: u16) -> [Vec<u16>; 2] {
    [
        (0..STRIDE * ROWS)
            .map(|i| ((i * 73 + i / STRIDE * 29) % (usize::from(max) + 1)) as u16)
            .collect(),
        (0..STRIDE * ROWS)
            .map(|i| {
                if (i + i / STRIDE).is_multiple_of(3) {
                    max
                } else {
                    0
                }
            })
            .collect(),
    ]
}

fn taps(filter: usize, phase: usize) -> Option<&'static Taps> {
    TAPS[filter][phase].as_ref()
}

/// Column remaps of a window clamped on the left and on the right.
fn remaps() -> [Option<[u8; 16]>; 3] {
    [
        None,
        Some(core::array::from_fn(|lane| lane.saturating_sub(3) as u8)),
        Some(core::array::from_fn(|lane| lane.min(6) as u8)),
    ]
}

/// Every sink of the NEON `predict` against the portable kernels.
fn check_predict(block: &Block<'_, u16>, max: u16, label: &str) {
    let (w, h) = (block.w, block.h);
    let rows = h + block.vertical.map_or(0, |taps| taps.span() - 1);
    let len = stage_len(rows, w, block.vertical);
    let stride = w + 3;
    let run = |neon: bool, sink: &mut Sink<'_>| {
        let mut stage = vec![0x5a5a_u16 as i16; len];
        if neon {
            assert!(super::predict(block, sink, &mut stage), "declined {label}");
        } else {
            portable::predict(block, sink, &mut stage);
        }
    };
    let words = |neon: bool| {
        let mut output = vec![0xa5a5_u16; stride * h];
        run(
            neon,
            &mut Sink::PutWide {
                output: &mut output,
                stride,
                max,
            },
        );
        output
    };
    assert_eq!(words(true), words(false), "put wide {label}");
    let bytes = |neon: bool| {
        let mut output = vec![0xa5_u8; stride * h];
        run(
            neon,
            &mut Sink::Put {
                output: &mut output,
                stride,
            },
        );
        output
    };
    assert_eq!(bytes(true), bytes(false), "put {label}");
    let prep = |neon: bool| {
        let mut output = vec![i32::MIN; stride * h];
        run(
            neon,
            &mut Sink::Prep {
                output: &mut output,
                stride,
            },
        );
        output
    };
    let pred = prep(false);
    assert_eq!(prep(true), pred, "prep {label}");
    let pred0 = (0..w * h)
        .map(|i| pred[(w * h - 1 - i) / w * stride + (w * h - 1 - i) % w])
        .collect::<Vec<_>>();
    for forward in [8, 4, 11, 0, 16] {
        let average = |neon: bool| {
            let mut wide = vec![0xa5a5_u16; stride * h];
            run(
                neon,
                &mut Sink::AverageWide {
                    pred0: &pred0,
                    forward,
                    output: &mut wide,
                    stride,
                    max,
                },
            );
            let mut bytes = vec![0xa5_u8; stride * h];
            run(
                neon,
                &mut Sink::Average {
                    pred0: &pred0,
                    forward,
                    output: &mut bytes,
                    stride,
                },
            );
            (wide, bytes)
        };
        assert_eq!(average(true), average(false), "average {forward} {label}");
    }
}

/// Every word-lane shape, filter, phase pair, column remap and row clamp of
/// 10- and 12-bit samples.
#[test]
fn neon_predict_matches_portable_for_every_shape_phase_and_sink() {
    let shapes = [
        (4, 2),
        (4, 4),
        (4, 8),
        (4, 16),
        (8, 2),
        (8, 4),
        (8, 8),
        (8, 32),
        (16, 4),
        (16, 16),
        (24, 4),
        (32, 8),
        (64, 4),
        (128, 2),
    ];
    let phases = [0, 1, 4, 7, 8, 9, 15];
    for max in [1023, 4095] {
        for plane in &planes(max) {
            for (case, &(w, h)) in shapes.iter().enumerate() {
                for (index, &(h_phase, v_phase)) in phases
                    .iter()
                    .flat_map(|&x| phases.iter().map(move |&y| (x, y)))
                    .collect::<Vec<_>>()
                    .iter()
                    .enumerate()
                {
                    let filter = (index + case) % NUM_FILTER_TYPES;
                    let horizontal = taps(filter, h_phase);
                    let remaps = if w <= 8 && horizontal.is_some() {
                        &remaps()[..]
                    } else {
                        &remaps()[..1]
                    };
                    for &remap in remaps {
                        for (top, rows) in [(2, [0, ROWS as isize - 1]), (-5, [0, 7]), (3, [4, 9])]
                        {
                            let block = Block {
                                source: plane,
                                stride: STRIDE,
                                origin: 5 + case,
                                top,
                                rows,
                                w,
                                h,
                                horizontal,
                                vertical: taps(filter, v_phase),
                                remap,
                            };
                            let label = format!(
                                "{max} {w}x{h} filter {filter} phases {h_phase},{v_phase} \
                                 remap {remap:?} top {top}"
                            );
                            check_predict(&block, max, &label);
                        }
                    }
                }
            }
        }
    }
}

/// Compound cells of at most 64 samples, and full-pel cells of any size,
/// against the portable compound.
#[test]
fn neon_compound_matches_portable() {
    let shapes = [(4, 2), (4, 4), (4, 16), (8, 2), (8, 8), (16, 4), (32, 32)];
    let phases = [0, 1, 7, 8, 15];
    for max in [1023, 4095] {
        let planes = planes(max);
        for (case, &(w, h)) in shapes.iter().enumerate() {
            for (index, &(h_phase, v_phase)) in phases
                .iter()
                .flat_map(|&x| phases.iter().map(move |&y| (x, y)))
                .collect::<Vec<_>>()
                .iter()
                .enumerate()
            {
                let filter = (index + case) % NUM_FILTER_TYPES;
                let block = |plane: usize, origin: usize, top: isize, fullpel: bool| Block {
                    source: &planes[plane][..],
                    stride: STRIDE,
                    origin,
                    top,
                    rows: [0, ROWS as isize - 1],
                    w,
                    h,
                    horizontal: if fullpel { None } else { taps(filter, h_phase) },
                    vertical: if fullpel { None } else { taps(filter, v_phase) },
                    remap: None,
                };
                let fullpel = w * h > 64;
                let first = block(0, 3, 1, fullpel);
                let second = block(1, 7 + case, 4, fullpel || index % 3 == 0);
                let rows = h + 7;
                let len =
                    stage_len(rows, w, first.vertical).max(stage_len(rows, w, second.vertical));
                let stride = w + 5;
                for forward in [8, 4, 11, 0, 16] {
                    let run = |neon: bool| {
                        let mut stage = vec![0x5a5a_u16 as i16; len];
                        let mut output = vec![0xa5a5_u16; stride * h];
                        let blocks = [&first, &second];
                        if neon {
                            assert!(super::compound(
                                blocks,
                                forward,
                                max,
                                &mut stage,
                                &mut output,
                                stride
                            ));
                        } else {
                            portable::compound_u16(
                                blocks,
                                forward,
                                max,
                                &mut stage,
                                &mut output,
                                stride,
                            );
                        }
                        output
                    };
                    assert_eq!(
                        run(true),
                        run(false),
                        "{max} {w}x{h} filter {filter} phases {h_phase},{v_phase} forward {forward}"
                    );
                }
            }
        }
    }
}
