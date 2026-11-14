// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::expect_used)]

use super::super::blend::{
    blend_compound_average, blend_compound_diff_weighted, diff_weighted_mask_into,
};
use super::super::{CWP_EQUAL, CompoundBlend};
use super::*;
use crate::prediction::inter::mv_scaling::derive_plane_scaling;

const SIDES: [usize; 6] = [8, 16, 32, 64, 128, 256];
const SUBSAMPLINGS: [(u32, u32); 3] = [(0, 0), (1, 0), (1, 1)];

/// The kernel family under test: `i16` predictors into `u8`, or `i32`
/// predictors into `u8` (eight bits) or `u16` (ten bits).
#[derive(Clone, Copy, Debug)]
enum Depth {
    Narrow,
    WideEight,
    WideTen,
}

const DEPTHS: [Depth; 3] = [Depth::Narrow, Depth::WideEight, Depth::WideTen];

impl Depth {
    fn bit_depth(self) -> BitDepth {
        match self {
            Self::WideTen => BitDepth::Ten,
            _ => BitDepth::Eight,
        }
    }
}

/// Compound predictors of `depth`: random, with runs of the range ends and
/// equal pairs so every weight and the clip at both ends occur.
fn predictors(len: usize, seed: u32, depth: Depth) -> [Vec<i32>; 2] {
    let (low, high) = match depth {
        Depth::WideTen => (-20588, 36956),
        _ => (-5132, 9212),
    };
    let mut state = seed.wrapping_mul(2_654_435_761) | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    let span = |value: u32| low + (value % (high - low + 1) as u32) as i32;
    let mut preds = [Vec::with_capacity(len), Vec::with_capacity(len)];
    for i in 0..len {
        let (a, b) = match next() % 8 {
            0 => (low, high),
            1 => (high, low),
            2 => {
                let value = span(next());
                (value, value)
            }
            3 => {
                let value = span(next()).min(high - 1200);
                (value, value + (i as i32 % 1200))
            }
            _ => (span(next()), span(next())),
        };
        preds[0].push(a);
        preds[1].push(b);
    }
    preds
}

fn narrow(preds: &[Vec<i32>; 2]) -> [Vec<i16>; 2] {
    preds
        .each_ref()
        .map(|pred| pred.iter().map(|&value| value as i16).collect())
}

/// Where a plane lies for the oracle: its plane position, the reference
/// scalings, the frame plane size and the luma block size.
#[derive(Clone, Copy)]
struct Placement {
    position: (usize, usize),
    scalings: [PlaneScaling; 2],
    frame: (usize, usize),
    luma: (usize, usize),
}

fn placed(size: (usize, usize), luma: (usize, usize)) -> Placement {
    let scaling = derive_plane_scaling(0, 0, 0, 0, 0, 0, 64, 64, 64, 64);
    Placement {
        position: (0, 0),
        scalings: [scaling; 2],
        frame: size,
        luma,
    }
}

/// The scalar oracle blend of one plane into a `w + 3`-strided output.
fn scalar<T: ReconSample>(
    preds: &[Vec<i32>; 2],
    (w, h): (usize, usize),
    blend: CompoundBlend,
    at: Placement,
    luma_mask: Option<&[u16]>,
    (sub, bit_depth): ((u32, u32), BitDepth),
) -> Vec<u16> {
    let stride = w + 3;
    let mut output = vec![T::default(); (h - 1) * stride + w];
    let [pred0, pred1] = preds;
    let result = if matches!(blend, CompoundBlend::Average { .. }) {
        let ([s0, s1], (x, y), (fw, fh)) = (at.scalings, at.position, at.frame);
        blend_compound_average::<T>(
            pred0,
            pred1,
            bit_depth,
            w,
            h,
            blend,
            None,
            x,
            y,
            s0,
            s1,
            fw,
            fh,
            sub.0,
            sub.1,
            &mut output,
            stride,
        )
    } else {
        let (luma_w, luma_h) = at.luma;
        blend_compound_diff_weighted::<T>(
            pred0,
            pred1,
            bit_depth,
            w,
            h,
            blend,
            luma_w,
            luma_h,
            luma_mask,
            sub.0,
            sub.1,
            &mut output,
            stride,
        )
    };
    result.expect("scalar blend");
    output.iter().map(|sample| sample.to_u16()).collect()
}

fn oracle(
    depth: Depth,
    preds: &[Vec<i32>; 2],
    size: (usize, usize),
    blend: CompoundBlend,
    at: Placement,
    luma_mask: Option<&[u16]>,
    sub: (u32, u32),
) -> Vec<u16> {
    let shape = (sub, depth.bit_depth());
    match depth {
        Depth::Narrow | Depth::WideEight => scalar::<u8>(preds, size, blend, at, luma_mask, shape),
        Depth::WideTen => scalar::<u16>(preds, size, blend, at, luma_mask, shape),
    }
}

/// The mask kernel of `depth` into a `w + 3`-strided output.
fn kernel_mask(
    depth: Depth,
    preds: &[Vec<i32>; 2],
    (w, h): (usize, usize),
    mask: &[u8],
) -> Vec<u16> {
    let stride = w + 3;
    let len = (h - 1) * stride + w;
    let refs = [preds[0].as_slice(), preds[1].as_slice()];
    match depth {
        Depth::Narrow => {
            let narrow = narrow(preds);
            let mut output = vec![0u8; len];
            let refs = [narrow[0].as_slice(), narrow[1].as_slice()];
            blend_mask_u8(&mut output, stride, refs, w, (w, h), mask, w).expect("mask blend");
            output.into_iter().map(u16::from).collect()
        }
        Depth::WideEight => {
            let mut output = vec![0u8; len];
            blend_mask_wide(
                &mut output,
                stride,
                refs,
                w,
                (w, h),
                (mask, w),
                BitDepth::Eight,
            )
            .expect("wide mask blend");
            output.into_iter().map(u16::from).collect()
        }
        Depth::WideTen => {
            let mut output = vec![0u16; len];
            blend_mask_wide(
                &mut output,
                stride,
                refs,
                w,
                (w, h),
                (mask, w),
                BitDepth::Ten,
            )
            .expect("wide mask blend");
            output
        }
    }
}

/// The difference-weight kernel of `depth` into a `w + 3`-strided output.
fn kernel_diff(
    depth: Depth,
    preds: &[Vec<i32>; 2],
    (w, h): (usize, usize),
    inverse: bool,
    chroma: (&mut [u8], usize, (u32, u32)),
) -> Vec<u16> {
    let stride = w + 3;
    let len = (h - 1) * stride + w;
    let refs = [preds[0].as_slice(), preds[1].as_slice()];
    let shape = (inverse, depth.bit_depth());
    match depth {
        Depth::Narrow => {
            let narrow = narrow(preds);
            let mut output = vec![0u8; len];
            let refs = [narrow[0].as_slice(), narrow[1].as_slice()];
            blend_diff_weighted_u8(&mut output, stride, refs, w, (w, h), inverse, Some(chroma))
                .expect("diff-weighted blend");
            output.into_iter().map(u16::from).collect()
        }
        Depth::WideEight => {
            let mut output = vec![0u8; len];
            blend_diff_weighted_wide(&mut output, stride, refs, w, (w, h), shape, Some(chroma))
                .expect("wide diff-weighted blend");
            output.into_iter().map(u16::from).collect()
        }
        Depth::WideTen => {
            let mut output = vec![0u16; len];
            blend_diff_weighted_wide(&mut output, stride, refs, w, (w, h), shape, Some(chroma))
                .expect("wide diff-weighted blend");
            output
        }
    }
}

#[test]
fn diff_weighted_kernels_match_the_scalar_blend() {
    let sizes = SIDES
        .iter()
        .flat_map(|&w| SIDES.iter().map(move |&h| (w, h)));
    for (seed, (w, h)) in sizes.enumerate() {
        for depth in DEPTHS {
            let preds = predictors(w * h, seed as u32, depth);
            for inverse in [false, true] {
                let blend = CompoundBlend::DiffWeighted { inverse };
                let mut luma_mask = Vec::new();
                let [pred0, pred1] = &preds;
                diff_weighted_mask_into(
                    pred0,
                    pred1,
                    depth.bit_depth(),
                    w,
                    h,
                    inverse,
                    &mut luma_mask,
                )
                .expect("luma mask");
                let want = oracle(
                    depth,
                    &preds,
                    (w, h),
                    blend,
                    placed((w, h), (w, h)),
                    Some(&luma_mask),
                    (0, 0),
                );
                for sub @ (sub_x, sub_y) in SUBSAMPLINGS {
                    let (cw, ch) = (w >> sub_x, h >> sub_y);
                    let mut chroma_mask = vec![0u8; cw * ch];
                    let luma =
                        kernel_diff(depth, &preds, (w, h), inverse, (&mut chroma_mask, cw, sub));
                    assert_eq!(
                        luma, want,
                        "luma {depth:?} {w}x{h} inverse {inverse} sub {sub:?}"
                    );
                    let chroma_preds = predictors(cw * ch, !(seed as u32), depth);
                    let at = placed((cw, ch), (w, h));
                    let chroma_want = oracle(
                        depth,
                        &chroma_preds,
                        (cw, ch),
                        blend,
                        at,
                        Some(&luma_mask),
                        sub,
                    );
                    let chroma = kernel_mask(depth, &chroma_preds, (cw, ch), &chroma_mask);
                    assert_eq!(
                        chroma, chroma_want,
                        "chroma {depth:?} {w}x{h} {inverse} {sub:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn wedge_mask_blend_matches_the_scalar_blend() {
    let sizes = [
        (8, 8),
        (8, 16),
        (16, 8),
        (16, 16),
        (8, 32),
        (32, 8),
        (16, 32),
        (32, 16),
        (32, 32),
    ];
    for (seed, (w, h)) in sizes.into_iter().enumerate() {
        for index in 0u8..68 {
            for sign in [false, true] {
                let blend = CompoundBlend::Wedge { index, sign };
                for sub @ (sub_x, sub_y) in SUBSAMPLINGS {
                    let depth = DEPTHS[usize::from(index) % 3];
                    let (pw, ph) = (w >> sub_x, h >> sub_y);
                    let preds = predictors(pw * ph, seed as u32 * 131 + u32::from(index), depth);
                    let want = oracle(
                        depth,
                        &preds,
                        (pw, ph),
                        blend,
                        placed((pw, ph), (w, h)),
                        None,
                        sub,
                    );
                    let mut mask = vec![0; pw * ph];
                    wedge_mask_map(&mut mask, pw, (w, h), (index, sign), sub).expect("wedge mask");
                    assert!(mask.iter().any(|&m| m != mask[0]), "wedge {index} {w}x{h}");
                    let got = kernel_mask(depth, &preds, (pw, ph), &mask);
                    assert_eq!(got, want, "wedge {index} sign {sign} {w}x{h} sub {sub:?}");
                }
            }
        }
    }
}

#[test]
fn implicit_mask_blend_matches_the_scalar_blend() {
    let sizes = [
        (2, 8),
        (4, 16),
        (16, 4),
        (8, 8),
        (12, 6),
        (16, 16),
        (64, 32),
        (128, 128),
    ];
    let (frame_w, frame_h) = (160usize, 144usize);
    let mvs = [-1200, -700, -130, -9, 0, 7, 300, 900];
    let (mut seed, mut edge_masks) = (0, 0);
    for (w, h) in sizes {
        let edges = [0, 8, frame_w - w, frame_w.saturating_sub(w / 2)];
        for (&plane_x, &plane_y) in edges.iter().zip(edges.iter().rev()) {
            let plane_y = plane_y.min(frame_h - h / 2);
            for (&mv0, &mv1) in mvs.iter().zip(mvs.iter().rev().cycle().skip(seed % 3)) {
                seed += 1;
                let depth = DEPTHS[seed % 3];
                let scaling = |row: i32, col: i32| {
                    let (frame_w, frame_h) = (frame_w as i32, frame_h as i32);
                    let (x, y) = (plane_x as i32, plane_y as i32);
                    derive_plane_scaling(x, y, row, col, 0, 0, frame_w, frame_h, frame_w, frame_h)
                };
                let scalings = [scaling(mv0, mv1), scaling(mv1 / 3, mv0)];
                let preds = predictors(w * h, seed as u32, depth);
                let blend = CompoundBlend::Average {
                    implicit_mask: true,
                    cwp_weight: CWP_EQUAL,
                };
                let at = Placement {
                    position: (plane_x, plane_y),
                    scalings,
                    frame: (frame_w, frame_h),
                    luma: (w, h),
                };
                let want = oracle(depth, &preds, (w, h), blend, at, None, (0, 0));
                let mut mask = vec![0; w * h];
                implicit_mask_map(&mut mask, w, scalings, (frame_w, frame_h));
                edge_masks += usize::from(mask.iter().any(|&m| m != 32));
                let got = kernel_mask(depth, &preds, (w, h), &mask);
                assert_eq!(
                    got, want,
                    "{depth:?} {w}x{h} at {plane_x},{plane_y} mvs {mv0} {mv1}"
                );
            }
        }
    }
    assert!(
        edge_masks > seed / 2,
        "{edge_masks} of {seed} masks are not uniform"
    );
}

#[test]
fn mask_kernels_reject_short_buffers_and_bad_shapes() {
    let preds = [vec![0i16; 64], vec![0i16; 64]];
    let refs = [preds[0].as_slice(), preds[1].as_slice()];
    let mut output = vec![0u8; 64];
    assert!(blend_mask_u8(&mut output, 8, refs, 8, (8, 8), &[0; 63], 8).is_err());
    assert!(blend_mask_u8(&mut output, 7, refs, 8, (8, 8), &[0; 64], 8).is_err());
    assert!(blend_mask_u8(&mut output, 8, refs, 8, (3, 8), &[0; 64], 8).is_err());
    assert!(blend_diff_weighted_u8(&mut output, 8, refs, 4, (4, 8), false, None).is_err());
    let mut mask = vec![0u8; 16];
    let chroma = Some((mask.as_mut_slice(), 4, (0, 1)));
    assert!(blend_diff_weighted_u8(&mut output, 8, refs, 8, (8, 8), false, chroma).is_err());
    let chroma = Some((mask.as_mut_slice(), 4, (1, 1)));
    assert!(blend_diff_weighted_u8(&mut output, 8, refs, 8, (8, 7), false, chroma).is_err());
    assert_eq!(output, vec![0u8; 64]);
}
