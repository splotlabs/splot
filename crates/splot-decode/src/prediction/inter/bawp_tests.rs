// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, FramePlanes,
    InterpolationFilter, OutputIndex, PixelFormat, Plane, PlaneId, PlaneRect, PlaneSize,
    ReconSample, ReferencePlaneView,
};

use super::{
    ReferenceSamples, TemplateStats, apply_bawp, apply_intrabc_morph_pred,
    bawp_reference_luma_rows, bawp_template_counts,
};
use crate::pipeline::frame_progress::PublishedFrame;
use crate::pipeline::inflight::RefFrameSlot;
use crate::prediction::inter::{BawpSyntax, InterBlock, Mv, PlacedInterBlock, mc::CompoundBlend};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn template_counts_follow_the_clamped_size_table() {
    for (case, expected) in [
        ((16, 16, true, true, true), (16, 16, 16, 16)),
        ((12, 16, true, true, true), (8, 16, 8, 8)),
        ((16, 12, true, true, true), (16, 8, 8, 8)),
        ((4, 4, true, true, true), (4, 4, 4, 4)),
        ((16, 4, true, true, true), (16, 4, 16, 0)),
        ((4, 16, true, true, true), (4, 16, 0, 16)),
        ((32, 8, true, true, false), (16, 8, 16, 0)),
        ((8, 32, true, false, true), (8, 16, 0, 16)),
        ((32, 32, false, true, true), (8, 8, 8, 8)),
        ((12, 12, false, true, true), (8, 8, 8, 8)),
        ((64, 64, true, false, false), (16, 16, 0, 0)),
    ] {
        let (bw, bh, luma, up, left) = case;
        assert_eq!(
            bawp_template_counts(bw, bh, luma, up, left),
            expected,
            "bw={bw} bh={bh} luma={luma} up={up} left={left}"
        );
    }
}

/// A § 7.13.3.25 sample pattern: noise, all-max, or 0 / max alternation.
fn pattern(kind: usize, max: u16, seed: usize) -> impl Fn(usize, usize) -> u16 {
    move |x: usize, y: usize| {
        let k = (x * 131 + y * 7919 + seed).wrapping_mul(2_654_435_761) >> 7;
        match kind {
            0 => (k % (usize::from(max) + 1)) as u16,
            1 => max,
            _ => [0, max][(x + y + seed) % 2],
        }
    }
}

/// The spec loops of § 7.13.3.25 over `cur(x, y)` and `reference(x, y)`
/// sample readers: `[sumX, sumY, sumXX, sumXY]`.
fn spec_sums(
    ((width, num_up), (height, num_left)): ((usize, usize), (usize, usize)),
    cur: impl Fn(usize, usize) -> i32,
    reference: impl Fn(usize, usize) -> i32,
    ((x, y), (ref_x, ref_y)): ((usize, usize), (usize, usize)),
) -> [i32; 4] {
    let mut sums = [0; 4];
    let mut add = |recon: i32, sample: i32| {
        sums[0] += sample;
        sums[1] += recon;
        sums[2] += sample * sample;
        sums[3] += sample * recon;
    };
    if let Some(step) = width.checked_div(num_up) {
        let mut i = step >> 1;
        while i < width {
            add(cur(x + i, y - 1), reference(ref_x + i, ref_y - 1));
            i += step;
        }
    }
    if let Some(step) = height.checked_div(num_left) {
        let mut i = step >> 1;
        while i < height {
            add(cur(x - 1, y + i), reference(ref_x - 1, ref_y + i));
            i += step;
        }
    }
    sums
}

/// Every template shape [`bawp_template_counts`] admits, at both bit depths,
/// above only / left only / both, inside the frame and cut by its right and
/// bottom storage edge (reads past it repeat the last stored sample), with
/// noise, all-max and 0 / max samples: the in-register sums equal the spec
/// loops, and a template whose arm leaves the frame or the reference is
/// `None`.
#[test]
fn template_sums_match_the_spec_loops() -> TestResult {
    check_template_sums::<u8>(BitDepth::Eight)?;
    check_template_sums::<u16>(BitDepth::Ten)
}

/// The sums [`CurrentFrameWorkspace::apply_bawp_template`] fits, through an
/// empty block that scales nothing.
fn template_sums<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: Option<&ReferencePlaneView<'_, T>>,
    at: ((usize, usize), (usize, usize)),
    arms: ((usize, usize), (usize, usize)),
    products: bool,
) -> TestResult<Option<[i32; 4]>> {
    let mut sums = None;
    workspace.apply_bawp_template(PlaneId::Y, reference, at, arms, products, (0, 0), |fit| {
        sums = Some(fit);
        (256, 0)
    })?;
    Ok(sums)
}

fn check_template_sums<T: ReconSample>(bit_depth: BitDepth) -> TestResult {
    let (frame_w, frame_h) = (44, 36);
    let max = bit_depth.max_sample();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        PixelFormat::Monochrome,
        PlaneSize::new(frame_w, frame_h)?,
        PlaneRect::new(0, 0, frame_w, frame_h)?,
    )?;
    let sides = [4, 8, 12, 16, 32, 64];
    for kind in 0..3 {
        let (cur, refs) = (pattern(kind, max, 1), pattern(kind, max, 5));
        let mut workspace = CurrentFrameWorkspace::<T>::new(info, T::default())?;
        let samples: Vec<T> = (0..frame_w * frame_h)
            .map(|i| T::try_from_u16(cur(i % frame_w, i / frame_w)))
            .collect::<std::result::Result<_, _>>()?;
        let whole = PlaneRect::new(0, 0, frame_w, frame_h)?;
        workspace.write_rect(PlaneId::Y, whole, &samples, frame_w)?;
        let reference: Vec<T> = (0..frame_w * frame_h)
            .map(|i| T::try_from_u16(refs(i % frame_w, i / frame_w)))
            .collect::<std::result::Result<_, _>>()?;
        let view = ReferencePlaneView::new(&reference, frame_w, frame_h)?;
        let shapes = sides.into_iter().flat_map(|bw| {
            sides
                .into_iter()
                .flat_map(move |bh| [(bw, bh, true), (bw, bh, false)])
        });
        for (bw, bh, luma) in shapes {
            for (up, left) in [(true, true), (true, false), (false, true)] {
                let (width, height, num_up, num_left) =
                    bawp_template_counts(bw, bh, luma, up, left);
                if !(matches!(width, 4 | 8 | 16) && matches!(height, 4 | 8 | 16)) {
                    continue;
                }
                let arms = ((width, num_up), (height, num_left));
                let at = (3, 2);
                // Inside, cut by the right edge, by the bottom edge, by both.
                for (x, y) in [
                    (5, 3),
                    (frame_w - width / 2, 9),
                    (7, frame_h - height / 2),
                    (frame_w - 3, frame_h - 1),
                ] {
                    let last_col = (x + width).min(frame_w) - x - 1;
                    let last_row = (y + height).min(frame_h) - y - 1;
                    let cur_at = |cx: usize, cy: usize| {
                        i32::from(cur(cx.min(x + last_col), cy.min(y + last_row)))
                    };
                    let ref_at = |rx: usize, ry: usize| view.sample(ry, rx);
                    let want = spec_sums(arms, cur_at, ref_at, ((x, y), at));
                    for products in [true, false] {
                        let got = template_sums(
                            &mut workspace,
                            Some(&view),
                            ((x, y), at),
                            arms,
                            products,
                        )?;
                        let want = if products {
                            want
                        } else {
                            [want[0], want[1], 0, 0]
                        };
                        assert_eq!(
                            got,
                            Some(want),
                            "{bw}x{bh} {luma} {up} {left} at {x},{y} kind {kind} {bit_depth:?}"
                        );
                    }
                    let own_at = |rx: usize, ry: usize| i32::from(cur(rx, ry));
                    let own = spec_sums(arms, cur_at, own_at, ((x, y), at));
                    assert_eq!(
                        template_sums(&mut workspace, None, ((x, y), at), arms, true)?,
                        Some(own),
                        "own {bw}x{bh} {luma} {up} {left} at {x},{y}"
                    );
                }
                let mut sums = |block: (usize, usize), at: (usize, usize)| {
                    template_sums(&mut workspace, Some(&view), (block, at), arms, true)
                };
                assert_eq!(sums((5, 3), (0, 2))?, None);
                assert_eq!(sums((5, 3), (3, 0))?, None);
                assert_eq!(sums((5, 3), (frame_w - width + 1, 2))?, None);
                assert_eq!(sums((5, 3), (3, frame_h - height + 1))?, None);
                if num_up > 0 {
                    assert_eq!(sums((5, 0), at)?, None);
                }
                if num_left > 0 {
                    assert_eq!(sums((0, 3), at)?, None);
                }
            }
        }
    }
    Ok(())
}

/// The shifts of [`TemplateStats`] against the spec's `/`, which truncates
/// toward zero (§ 4), over negative and positive `beta` numerators, all-max
/// and 0 / max templates and every power-of-two count.
#[test]
fn template_stats_shift_like_the_spec_division() {
    let spec_alpha = |[x, y, xx, xy]: [i32; 4], count: i32| {
        let nor = xy - x * y / count;
        let der = xx - x * x / count;
        if der != 0 && nor != 0 {
            match splot_recon::math::resolve_division(nor, der, 8) {
                0 => 256,
                alpha => alpha,
            }
        } else {
            256
        }
    };
    let mut seed = 0x9e37_79b9u32;
    let mut next = |bound: i32| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as i32 % bound
    };
    for count in [4, 8, 16, 32] {
        for case in 0..4000 {
            let max = if case % 2 == 0 { 255 } else { 1023 };
            let samples: Vec<(i32, i32)> = (0..count)
                .map(|i| match case % 5 {
                    0 => (max, max),
                    1 => ((i % 2) * max, ((i + 1) % 2) * max),
                    _ => (next(max + 1), next(max + 1)),
                })
                .collect();
            let sums = samples.iter().fold([0; 4], |[sx, sy, sxx, sxy], &(x, y)| {
                [sx + x, sy + y, sxx + x * x, sxy + x * y]
            });
            let stats = TemplateStats::new(sums, count as usize);
            let alpha = stats.implicit_alpha();
            assert_eq!(alpha, spec_alpha(sums, count), "{sums:?} {count}");
            let random = next(1023) as i16 - 511;
            for alpha in [alpha, -511, -48, 192, 320, 511, random] {
                let numerator = (sums[1] << 8) - sums[0] * i32::from(alpha);
                assert_eq!(
                    stats.beta(alpha),
                    numerator / count,
                    "{sums:?} {count} {alpha}"
                );
            }
        }
    }
    let empty = TemplateStats::new([0; 4], 0);
    assert_eq!((empty.implicit_alpha(), empty.beta(300)), (256, -128));
    let full = TemplateStats::new([4 * 1023, 0, 4 * 1023 * 1023, 0], 4);
    assert_eq!(full.beta(511), -(1023 * 511), "exact negative");
    let small = TemplateStats::new([1, 0, 1, 0], 4);
    assert_eq!(
        (small.beta(3), small.beta(-3)),
        (0, 0),
        "+-3 / 4 truncate to 0"
    );
    let seven = TemplateStats::new([7, 0, 49, 0], 4);
    assert_eq!(seven.beta(1), -1, "-7 / 4 truncates to -1");
}

#[test]
fn intrabc_morph_pred_skips_unavailable_top_left_template() -> TestResult {
    let mut workspace = workspace(8, 8, 77)?;
    let target = PlaneRect::new(0, 0, 4, 4)?;

    apply_intrabc_morph_pred(
        &mut workspace,
        target,
        (false, false),
        Mv { row: 0, col: 0 },
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 0, 0)?, 77);
    Ok(())
}

#[test]
fn inter_bawp_skips_unavailable_top_left_reference_template() -> TestResult {
    let mut workspace = workspace(8, 8, 91)?;
    let reference = frame(8, 8, vec![12; 64], vec![34; 16], vec![56; 16])?;
    let placed = placed_luma_block(0, 0, 4, 4);

    apply_bawp(
        &mut workspace,
        ReferenceSamples::settled(&reference),
        &placed,
        BawpSyntax {
            enabled: true,
            ..BawpSyntax::default()
        },
        Mv { row: 0, col: 0 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 0, 0)?, 91);
    Ok(())
}

#[test]
fn intrabc_morph_pred_applies_large_luma_block() -> TestResult {
    let mut workspace = workspace(160, 160, 40)?;
    for col in 24..40 {
        workspace.set_reconstructed_sample(PlaneId::Y, col, 23, 20)?;
    }
    for row in 24..40 {
        workspace.set_reconstructed_sample(PlaneId::Y, 23, row, 20)?;
    }

    apply_intrabc_morph_pred(
        &mut workspace,
        PlaneRect::new(16, 16, 128, 128)?,
        (true, true),
        Mv { row: 64, col: 64 },
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 16, 16)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 143, 143)?, 60);
    Ok(())
}

/// § 7.13.3.25 clips `bh` to `MiRows * MI_SIZE - y` only for the template
/// counts: a 16x32 block that the frame bottom cuts to 16x20 still adapts.
#[test]
fn intrabc_morph_pred_applies_block_cut_by_the_frame_edge() -> TestResult {
    let mut workspace = workspace(48, 36, 40)?;
    for i in 16..32 {
        workspace.set_reconstructed_sample(PlaneId::Y, i, 15, 20)?;
        workspace.set_reconstructed_sample(PlaneId::Y, 15, i, 20)?;
    }

    apply_intrabc_morph_pred(
        &mut workspace,
        PlaneRect::new(16, 16, 16, 20)?,
        (true, true),
        Mv { row: -64, col: -64 },
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 16, 16)?, 20);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 31, 35)?, 20);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 32, 16)?, 40);
    Ok(())
}

#[test]
fn inter_bawp_applies_large_luma_block() -> TestResult {
    let mut workspace = workspace(160, 160, 40)?;
    let reference = frame(
        160,
        160,
        vec![20; 160 * 160],
        vec![20; 80 * 80],
        vec![20; 80 * 80],
    )?;

    apply_bawp(
        &mut workspace,
        ReferenceSamples::settled(&reference),
        &placed_luma_block(16, 16, 128, 128),
        BawpSyntax {
            enabled: true,
            avail_up: true,
            avail_left: true,
            ..BawpSyntax::default()
        },
        Mv { row: 0, col: 0 },
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 16, 16)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 143, 143)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::Y, 15, 15)?, 40);
    Ok(())
}

#[test]
fn inter_bawp_uses_full_resolution_444_chroma_geometry() -> TestResult {
    let mut workspace = workspace_with_format(16, 16, 40, PixelFormat::Yuv444)?;
    let reference = frame_with_format(
        16,
        16,
        vec![20; 16 * 16],
        vec![20; 16 * 16],
        vec![20; 16 * 16],
        PixelFormat::Yuv444,
    )?;

    apply_bawp(
        &mut workspace,
        ReferenceSamples::settled(&reference),
        &placed_luma_block(4, 4, 8, 8),
        BawpSyntax {
            enabled: true,
            chroma: true,
            avail_up: true,
            avail_left: true,
            ..BawpSyntax::default()
        },
        Mv::ZERO,
        splot_core::span::ByteOffset::new(0),
    )?;

    assert_eq!(workspace.reconstructed_sample(PlaneId::U, 4, 4)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::V, 4, 4)?, 60);
    assert_eq!(workspace.reconstructed_sample(PlaneId::U, 2, 2)?, 40);
    Ok(())
}

/// A block at luma row 16 reads its reference template over rows 15..=31, so
/// [`bawp_reference_luma_rows`] is exact here: publishing its 32 rows admits the
/// read and one row fewer must fail closed.
#[test]
fn inter_bawp_template_read_fails_closed_one_row_under_its_bound() -> TestResult {
    let placed = placed_luma_block(16, 16, 128, 128);
    let syntax = BawpSyntax {
        enabled: true,
        avail_up: true,
        avail_left: true,
        ..BawpSyntax::default()
    };
    let needed = bawp_reference_luma_rows(16, 128, 0) as usize;
    assert_eq!(needed, 32);

    let size = PlaneSize::new(160, 160)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv444,
        size,
        PlaneRect::new(0, 0, 160, 160)?,
    )?;
    let mut published_references = [None, None];
    for (reference, rows) in published_references.iter_mut().zip([needed, needed - 1]) {
        let (slot, writer) = RefFrameSlot::<u8>::pending(info)?;
        let progress = writer.progress_handle();
        progress.begin(&[(0, rows), (rows, 160)])?;
        let mut lease = progress
            .direct_stripe(0)
            .ok_or_else(|| format!("published {rows}-row stripe lease"))?;
        let mut target = lease.take_target().ok_or("published stripe target")?;
        for plane in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            target
                .take(plane)
                .ok_or("published plane target")?
                .u8_samples_mut()
                .ok_or("published eight-bit plane")?
                .fill(20);
        }
        assert!(lease.submit());
        *reference = Some((slot, writer));
    }
    let [admitted_progress, short_progress] = published_references;

    let (admitted_slot, _writer) = admitted_progress.ok_or("admitted published slot")?;
    let admitted_frame = PublishedFrame::of_slot(&admitted_slot).ok_or("published frame")?;
    let mut admitted = workspace(160, 160, 40)?;
    apply_bawp(
        &mut admitted,
        ReferenceSamples::publishing(&admitted_frame),
        &placed,
        syntax,
        Mv::ZERO,
        splot_core::span::ByteOffset::new(0),
    )?;
    assert_eq!(admitted.reconstructed_sample(PlaneId::Y, 16, 16)?, 60);

    let (short_slot, _short_writer) = short_progress.ok_or("short published slot")?;
    let short_frame = PublishedFrame::of_slot(&short_slot).ok_or("short published frame")?;
    let mut short = workspace(160, 160, 40)?;
    let refused = apply_bawp(
        &mut short,
        ReferenceSamples::publishing(&short_frame),
        &placed,
        syntax,
        Mv::ZERO,
        splot_core::span::ByteOffset::new(0),
    );
    let refused = refused
        .err()
        .map(|error| format!("{error}"))
        .unwrap_or_default();
    assert!(
        refused.contains("published"),
        "one row short of the bound must fail closed, got {refused:?}"
    );
    Ok(())
}

fn workspace(width: usize, height: usize, fill: u8) -> TestResult<CurrentFrameWorkspace<u8>> {
    workspace_with_format(width, height, fill, PixelFormat::Yuv420)
}

fn workspace_with_format(
    width: usize,
    height: usize,
    fill: u8,
    pixel_format: PixelFormat,
) -> TestResult<CurrentFrameWorkspace<u8>> {
    let luma_size = PlaneSize::new(width, height)?;
    let visible = PlaneRect::new(0, 0, width, height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        luma_size,
        visible,
    )?;
    Ok(CurrentFrameWorkspace::new(info, fill)?)
}

fn frame(
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
) -> TestResult<DecodedFrame<u8>> {
    frame_with_format(width, height, y, u, v, PixelFormat::Yuv420)
}

fn frame_with_format(
    width: usize,
    height: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    pixel_format: PixelFormat,
) -> TestResult<DecodedFrame<u8>> {
    let luma_size = PlaneSize::new(width, height)?;
    let luma_rect = PlaneRect::new(0, 0, width, height)?;
    let chroma_width = width.div_ceil(1 << pixel_format.subsampling_x());
    let chroma_height = height.div_ceil(1 << pixel_format.subsampling_y());
    let chroma_size = PlaneSize::new(chroma_width, chroma_height)?;
    let chroma_rect = PlaneRect::new(0, 0, chroma_width, chroma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        pixel_format,
        luma_size,
        luma_rect,
    )?;

    Ok(DecodedFrame::try_new(
        info,
        FramePlanes::new(
            plane(luma_size, width, luma_rect, y)?,
            Some(plane(chroma_size, chroma_width, chroma_rect, u)?),
            Some(plane(chroma_size, chroma_width, chroma_rect, v)?),
        ),
    )?)
}

fn plane(
    size: PlaneSize,
    stride: usize,
    visible: PlaneRect,
    samples: Vec<u8>,
) -> TestResult<Plane<u8>> {
    Ok(Plane::from_vec(size, stride, visible, samples)?)
}

fn placed_luma_block(x: usize, y: usize, width: usize, height: usize) -> PlacedInterBlock {
    PlacedInterBlock {
        luma: [x, y, width, height].map(|v| v as u32),
        chroma_luma: [x, y, width, height].map(|v| v as u32),
        predict_chroma: false,
        sub8x8_chroma: false,
        interintra_chroma: false,
        interintra_edges: [crate::pipeline::reconstruct::IntraEdgeAvailability::new(true, true); 2],
        block: InterBlock {
            ref_frame0: 0,
            ref_frame1: None,
            mv: Mv { row: 0, col: 0 },
            mv1: Mv { row: 0, col: 0 },
            interp: InterpolationFilter::EightTap,
            warp_params: [None, None],
            bawp: BawpSyntax::default(),
            interintra: None,
            compound_blend: CompoundBlend::default(),
            optflow_distances: None,
            residual: None,
        },
    }
}
