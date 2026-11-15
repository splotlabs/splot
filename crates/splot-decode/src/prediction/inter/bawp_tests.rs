// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, FramePlanes,
    InterpolationFilter, OutputIndex, PixelFormat, Plane, PlaneId, PlaneRect, PlaneSize,
};

use super::{
    ReferenceSamples, TemplatePairs, TemplateStats, apply_bawp, apply_intrabc_morph_pred,
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

/// The § 7.13.3.25 sums of the oracle: every `(reference, current)` pair of
/// both arms added one at a time.
fn oracle_stats(
    arms: [(usize, usize); 2],
    pair: impl Fn(usize, usize) -> (u16, u16),
) -> TemplateStats {
    let mut stats = TemplateStats::default();
    for (arm, (len, num)) in arms.into_iter().enumerate() {
        if num == 0 {
            continue;
        }
        let step = len / num;
        let mut i = step >> 1;
        while i < len {
            let (x, y) = pair(arm, i);
            let (x, y) = (i32::from(x), i32::from(y));
            stats.sum_x += x;
            stats.sum_y += y;
            stats.sum_xy += x * y;
            stats.sum_xx += x * x;
            i += step;
        }
        stats.count += num as i32;
    }
    stats
}

/// Every template shape [`bawp_template_counts`] yields, at both bit
/// depths, with extreme and asymmetric samples: the lane sums equal the
/// per-pair oracle.
#[test]
fn template_lane_sums_match_the_pair_oracle() {
    let sides = [4, 8, 12, 16, 32, 64];
    for (bw, bh, luma, up, left) in sides.into_iter().flat_map(|bw| {
        sides.into_iter().flat_map(move |bh| {
            [(true, true, true), (false, true, true), (true, true, false)]
                .into_iter()
                .chain([(true, false, true), (true, false, false)])
                .map(move |(luma, up, left)| (bw, bh, luma, up, left))
        })
    }) {
        let (width, height, num_up, num_left) = bawp_template_counts(bw, bh, luma, up, left);
        for (max, seed) in [(255u16, 3usize), (1023, 7), (1023, 0)] {
            let pair = |arm: usize, i: usize| {
                let k = (arm * 16 + i) * 2_654_435_761 + seed;
                let value = |k: usize| match k % 5 {
                    0 => max,
                    1 => 0,
                    _ => (k >> 9) as u16 % (max + 1),
                };
                (value(k), value(k / 3 + 1))
            };
            let want = oracle_stats([(width, num_up), (height, num_left)], pair);
            for row_len in [width, 16, 24] {
                let rows: [Vec<u16>; 2] = core::array::from_fn(|side| {
                    let sample = |i: usize| [pair(0, i).0, pair(0, i).1][side];
                    (0..row_len)
                        .map(|i| if i < width { sample(i) } else { max - 1 })
                        .collect()
                });
                let bytes: [Vec<u8>; 2] = rows
                    .clone()
                    .map(|row| row.into_iter().map(|v| v as u8).collect());
                let mut pairs = TemplatePairs::default();
                let mut byte_pairs = TemplatePairs::default();
                if num_up > 0 {
                    let up = (width, num_up);
                    assert!(pairs.add_above(up, [&rows[0][..], &rows[1]]).is_some());
                    assert!(
                        byte_pairs
                            .add_above(up, [&bytes[0][..], &bytes[1]])
                            .is_some()
                    );
                }
                if num_left > 0 {
                    let left = (height, num_left);
                    assert!(pairs.add_left(left, |i| Some(pair(1, i))).is_some());
                    assert!(byte_pairs.add_left(left, |i| Some(pair(1, i))).is_some());
                }
                assert_eq!(
                    pairs.stats(),
                    want,
                    "{bw}x{bh} {luma} {up} {left} {max} {row_len}"
                );
                if max == 255 {
                    assert_eq!(
                        byte_pairs.stats(),
                        want,
                        "u8 {bw}x{bh} {up} {left} {row_len}"
                    );
                }
            }
        }
    }
    let mut pairs = TemplatePairs::default();
    let full_rows = [&[1023u16; 16][..], &[1023; 16]];
    assert!(pairs.add_above((16, 16), full_rows).is_some());
    assert!(pairs.add_left((16, 16), |_| Some((1023, 1023))).is_some());
    let full = oracle_stats([(16, 16); 2], |_, _| (1023, 1023));
    assert_eq!((pairs.stats(), full.sum_xx), (full, 32 * 1023 * 1023));
    assert!(
        TemplatePairs::default()
            .add_above((8, 8), [&[0u16; 7][..], &[0; 8]])
            .is_none()
    );
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
