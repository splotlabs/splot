// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::film_grain::FilmGrainModel;
use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

use super::{
    ActiveFilmGrain, GrainDestination, GrainGeometry, SCALING_LUT_LEN, Stripe, apply_film_grain,
    fill_stripe, generate_grain, init_scaling_lut, output_ranges, plane_axis_lengths,
    plane_from_visible, round2, scale_lut, scaling_points_for_plane, stripe_count,
    stripe_row_slice,
};
use crate::{
    BitDepth, DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, PixelFormat, Plane,
    PlaneRect, PlaneSize, ReconSample,
};

fn film_grain_model() -> FilmGrainModel {
    let bytes =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf");
    let ParsedBitstream::Ivf(stream) = parse_bitstream_partial(bytes) else {
        panic!("film-grain fixture must be IVF");
    };
    let obu = stream.frames[0]
        .obus
        .iter()
        .find(|obu| obu.header.obu_type == ObuType::FilmGrain)
        .unwrap();
    let PayloadStatus::Parsed(ParsedObu::FilmGrain(grain)) = obu.payload_status().unwrap() else {
        panic!("film-grain fixture must contain a parsed model");
    };
    grain.models[0].model.clone()
}

fn plane<T: ReconSample>(width: usize, height: usize, samples: Vec<T>) -> Plane<T> {
    let size = PlaneSize::new(width, height).unwrap();
    let rect = PlaneRect::new(0, 0, width, height).unwrap();
    Plane::from_vec(size, width, rect, samples).unwrap()
}

fn frame<T: ReconSample>(
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
    samples: &[u8],
) -> DecodedFrame<T> {
    let samples = samples.iter().copied().map(u16::from).collect::<Vec<_>>();
    frame_at_depth(width, height, pixel_format, BitDepth::Eight, &samples)
}

fn frame_at_depth<T: ReconSample>(
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
    bit_depth: BitDepth,
    samples: &[u16],
) -> DecodedFrame<T> {
    let convert = |values: &[u16]| {
        values
            .iter()
            .copied()
            .map(T::try_from_u16)
            .collect::<crate::Result<Vec<_>>>()
            .unwrap()
    };
    let luma_len = width * height;
    let luma = convert(&samples[..luma_len]);
    let luma_size = PlaneSize::new(width, height).unwrap();
    let chroma_size = pixel_format.chroma_size(luma_size).unwrap();
    let mut offset = luma_len;
    let (u, v) = if let Some(size) = chroma_size {
        let len = size.width() * size.height();
        let u = plane(
            size.width(),
            size.height(),
            convert(&samples[offset..offset + len]),
        );
        offset += len;
        let v = plane(
            size.width(),
            size.height(),
            convert(&samples[offset..offset + len]),
        );
        (Some(u), Some(v))
    } else {
        (None, None)
    };
    let rect = PlaneRect::new(0, 0, width, height).unwrap();
    let info = DecodedFrameInfo::new(
        OutputIndex::new(7),
        bit_depth,
        pixel_format,
        luma_size,
        rect,
    )
    .unwrap();
    DecodedFrame::try_new(info, FramePlanes::new(plane(width, height, luma), u, v)).unwrap()
}

fn sample_count(width: usize, height: usize, pixel_format: PixelFormat) -> usize {
    let luma = width * height;
    pixel_format
        .chroma_size(PlaneSize::new(width, height).unwrap())
        .unwrap()
        .map_or(luma, |size| luma + 2 * size.width() * size.height())
}

fn output_samples<T: ReconSample>(frame: &DecodedFrame<T>) -> Vec<u16> {
    [Some(frame.y()), frame.u(), frame.v()]
        .into_iter()
        .flatten()
        .flat_map(Plane::visible_rows)
        .flatten()
        .copied()
        .map(ReconSample::to_u16)
        .collect()
}

fn assert_u8_matches_u16_reference(
    width: usize,
    height: usize,
    pixel_format: PixelFormat,
    samples: &[u8],
    model: &FilmGrainModel,
    seed: u16,
) {
    let direct = apply_film_grain(
        &frame::<u8>(width, height, pixel_format, samples),
        model,
        seed,
    )
    .unwrap();
    let reference = apply_film_grain(
        &frame::<u16>(width, height, pixel_format, samples),
        model,
        seed,
    )
    .unwrap();
    assert_eq!(output_samples(&direct), output_samples(&reference));
}

struct TestRng(u64);

impl TestRng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }
}

#[test]
fn fixed_seed_output_is_deterministic() {
    let model = film_grain_model();
    let samples = (0..sample_count(17, 19, PixelFormat::Yuv420))
        .map(|index| (index * 37) as u8)
        .collect::<Vec<_>>();
    let first = apply_film_grain(
        &frame::<u8>(17, 19, PixelFormat::Yuv420, &samples),
        &model,
        0x1234,
    )
    .unwrap();
    let second = apply_film_grain(
        &frame::<u8>(17, 19, PixelFormat::Yuv420, &samples),
        &model,
        0x1234,
    )
    .unwrap();
    let different_seed = apply_film_grain(
        &frame::<u8>(17, 19, PixelFormat::Yuv420, &samples),
        &model,
        0x4321,
    )
    .unwrap();

    assert_eq!(first, second);
    assert_ne!(output_samples(&first), output_samples(&different_seed));
}

#[test]
fn direct_u8_matches_u16_reference_across_randomized_small_planes() {
    let mut rng = TestRng(0x2f6e_2b1d_91c4_7a35);
    for case in 0..128 {
        let width = (rng.next() as usize % 17) + 1;
        let height = (rng.next() as usize % 17) + 1;
        let pixel_format = match case % 4 {
            0 => PixelFormat::Monochrome,
            1 => PixelFormat::Yuv420,
            2 => PixelFormat::Yuv422,
            _ => PixelFormat::Yuv444,
        };
        let mut model = film_grain_model();
        model.overlap_flag = case & 1 != 0;
        model.film_grain_block_size = case & 2 != 0;
        model.clip_to_restricted_range = case & 4 != 0;
        model.mc_identity = model.clip_to_restricted_range && case & 8 != 0;
        model.chroma_scaling_from_luma = case % 3 == 0;
        if model.chroma_scaling_from_luma {
            model.num_cb_points = 0;
            model.num_cr_points = 0;
        }
        let samples = (0..sample_count(width, height, pixel_format))
            .map(|_| rng.next() as u8)
            .collect::<Vec<_>>();
        assert_u8_matches_u16_reference(
            width,
            height,
            pixel_format,
            &samples,
            &model,
            rng.next() as u16,
        );
    }
}

#[test]
fn edge_dimensions_overlap_and_extreme_grain_values_match_reference() {
    let mut model = film_grain_model();
    model.overlap_flag = true;
    model.film_grain_block_size = true;
    for (index, coefficient) in model.ar_coeffs_y.iter_mut().enumerate() {
        *coefficient = if index & 1 == 0 { -128 } else { 127 };
    }
    for (index, coefficient) in model.ar_coeffs_cb.iter_mut().enumerate() {
        *coefficient = if index & 1 == 0 { 127 } else { -128 };
    }
    for (index, coefficient) in model.ar_coeffs_cr.iter_mut().enumerate() {
        *coefficient = if index & 1 == 0 { -128 } else { 127 };
    }
    for point in model
        .point_y
        .iter_mut()
        .chain(model.point_cb.iter_mut())
        .chain(model.point_cr.iter_mut())
    {
        point.scaling = 255;
    }

    for &(width, height) in &[(1, 1), (1, 33), (33, 1), (63, 65), (65, 63)] {
        let samples = (0..sample_count(width, height, PixelFormat::Yuv420))
            .map(|index| if index & 1 == 0 { 0 } else { 255 })
            .collect::<Vec<_>>();
        assert_u8_matches_u16_reference(
            width,
            height,
            PixelFormat::Yuv420,
            &samples,
            &model,
            0xffff,
        );
    }
}

fn destination_allocation_is_retained<T: ReconSample + std::fmt::Debug + PartialEq>(
    samples: &[T],
) -> Vec<T> {
    let destination = GrainDestination {
        samples: crate::plane_buf_from(samples),
        stride: 2,
    };
    let pointer = destination.samples.as_ptr();
    let capacity = destination.samples.capacity();
    let plane = plane_from_visible(2, 2, destination).unwrap();
    let samples = plane.into_samples();

    assert_eq!(samples.as_ptr(), pointer);
    assert_eq!(samples.capacity(), capacity);
    samples.to_vec() // splot-copy-ok: the test compares against plain Vec storage
}

#[test]
fn u16_plane_retains_the_synthesis_allocation() {
    let mut samples = Vec::with_capacity(8);
    samples.extend([1_u16, 2, 3, 4]);
    assert_eq!(destination_allocation_is_retained(&samples), [1, 2, 3, 4]);
}

#[test]
fn u8_plane_uses_its_single_final_sample_allocation() {
    let mut samples = Vec::with_capacity(8);
    samples.extend([1_u8, 2, 3, 4]);
    assert_eq!(destination_allocation_is_retained(&samples), [1, 2, 3, 4]);
}

#[test]
fn scale_lut_maps_out_of_range_samples_to_the_last_entry() {
    let mut lut: [i32; 257] = core::array::from_fn(|index| index as i32 * 3);
    lut[256] = lut[255];
    assert_eq!(scale_lut(&lut, 1023, 10), lut[255]);
    assert_eq!(scale_lut(&lut, 4095, 10), lut[255]);
    assert_eq!(scale_lut(&lut, u16::MAX.into(), 10), lut[255]);
    assert_eq!(scale_lut(&lut, 6, 10), lut[1] + 2);
}

/// The § 7.21.7.5 process as written: the whole noise image first, then the
/// noise of every chroma sample from the source luma, then luma.
fn reference_output<T: ReconSample>(
    frame: &DecodedFrame<T>,
    model: &FilmGrainModel,
    seed: u16,
) -> Vec<u16> {
    let grain = ActiveFilmGrain {
        model,
        grain_seed: seed,
    };
    let bit_depth = frame.bit_depth().bits();
    let format = frame.pixel_format();
    let (width, height) = (
        frame.y().visible_size().width(),
        frame.y().visible_size().height(),
    );
    let (sub_x, sub_y) = (
        usize::from(format.subsampling_x()),
        usize::from(format.subsampling_y()),
    );
    let geometry = GrainGeometry {
        widths: plane_axis_lengths(width, sub_x),
        heights: plane_axis_lengths(height, sub_y),
        sub_x,
        sub_y,
        bit_depth,
    };
    let num_planes = format.num_planes();
    let range = (-(1 << (bit_depth - 1)), (1 << (bit_depth - 1)) - 1);
    let templates = generate_grain(&grain, sub_x, sub_y, bit_depth, range);
    let luma_size = if model.film_grain_block_size { 32 } else { 16 };
    let mut stripes =
        [0, 1].map(|_| Stripe::new(&geometry.widths[..num_planes], luma_size, sub_x, sub_y));
    let mut noise: Vec<Vec<Vec<i32>>> = (0..num_planes).map(|_| Vec::new()).collect();
    for luma_num in 0..stripe_count(height, luma_size) {
        let current = luma_num & 1;
        fill_stripe(
            &mut stripes[current],
            luma_num,
            &geometry,
            [true; 3],
            luma_size,
            &grain,
            &templates,
            range,
        );
        for (plane, noise) in noise.iter_mut().enumerate() {
            let plane_sub_y = if plane > 0 { sub_y } else { 0 };
            let shift = 4 + usize::from(model.film_grain_block_size) - plane_sub_y;
            let stripe_width = stripes[current].widths[plane];
            for i in 0..1 << shift {
                if (luma_num << shift) + i >= geometry.heights[plane] {
                    break;
                }
                let row = stripe_row_slice(
                    &stripes[current].planes[plane],
                    stripe_width,
                    i,
                    geometry.widths[plane],
                );
                let old_row = i + (luma_size >> plane_sub_y);
                let blend =
                    model.overlap_flag && luma_num > 0 && i < if plane_sub_y == 0 { 2 } else { 1 };
                noise.push(
                    row.iter()
                        .enumerate()
                        .map(|(x, &g)| {
                            let g = i32::from(g);
                            if !blend {
                                return g;
                            }
                            let old = i32::from(
                                stripes[current ^ 1].planes[plane][old_row * stripe_width + x],
                            );
                            let (wo, wn) = match (plane_sub_y, i) {
                                (0, 0) => (27, 17),
                                (0, _) => (17, 27),
                                _ => (23, 22),
                            };
                            round2(old * wo + g * wn, 5).clamp(range.0, range.1)
                        })
                        .collect(),
                );
            }
        }
    }
    let (min_value, max_luma, max_chroma) = output_ranges(&grain, bit_depth);
    let scaling_shift = model.grain_scaling_minus_8 + 8;
    let luts: Vec<[i32; SCALING_LUT_LEN]> = (0..num_planes)
        .map(|plane| {
            let mut lut = [0; SCALING_LUT_LEN];
            init_scaling_lut(scaling_points_for_plane(model, plane), &mut lut);
            lut
        })
        .collect();
    let rows = |plane: &Plane<T>| -> Vec<Vec<i32>> {
        plane
            .visible_rows()
            .map(|row| row.iter().map(|s| i32::from(s.to_u16())).collect())
            .collect()
    };
    let luma = rows(frame.y());
    let mut planes = vec![luma.clone()];
    let mults = [
        (
            model.cb_luma_mult,
            model.cb_mult,
            model.cb_offset,
            model.num_cb_points,
        ),
        (
            model.cr_luma_mult,
            model.cr_mult,
            model.cr_offset,
            model.num_cr_points,
        ),
    ];
    for (index, chroma) in [frame.u(), frame.v()]
        .into_iter()
        .flatten()
        .enumerate()
        .take(num_planes - 1)
    {
        let plane = index + 1;
        let (luma_mult, mult, offset, points) = mults[index];
        let mut out = rows(chroma);
        if points > 0 || model.chroma_scaling_from_luma {
            for (y, row) in out.iter_mut().enumerate() {
                for (x, sample) in row.iter_mut().enumerate() {
                    let lx = x << sub_x;
                    let ly = y << sub_y;
                    let next = (lx + 1).min(width - 1);
                    let average = if sub_x == 1 {
                        round2(luma[ly][lx] + luma[ly][next], 1)
                    } else {
                        luma[ly][lx]
                    };
                    let orig = *sample;
                    let merged = if model.chroma_scaling_from_luma {
                        average
                    } else {
                        let combined = average * (i32::from(luma_mult.unwrap_or(128)) - 128)
                            + orig * (i32::from(mult.unwrap_or(128)) - 128);
                        ((combined >> 6)
                            + ((i32::from(offset.unwrap_or(256)) - 256) << (bit_depth - 8)))
                            .clamp(0, (1 << bit_depth) - 1)
                    };
                    let delta = round2(
                        scale_lut(&luts[plane], merged, bit_depth) * noise[plane][y][x],
                        scaling_shift,
                    );
                    *sample = (orig + delta).clamp(min_value, max_chroma);
                }
            }
        }
        planes.push(out);
    }
    if model.num_y_points > 0 {
        for (y, row) in planes[0].iter_mut().enumerate() {
            for (x, sample) in row.iter_mut().enumerate() {
                let delta = round2(
                    scale_lut(&luts[0], *sample, bit_depth) * noise[0][y][x],
                    scaling_shift,
                );
                *sample = (*sample + delta).clamp(min_value, max_luma);
            }
        }
    }
    planes
        .into_iter()
        .flatten()
        .flatten()
        .map(|sample| sample as u16)
        .collect()
}

#[test]
fn streamed_synthesis_matches_the_whole_noise_image_reference() {
    let mut rng = TestRng(0x51a7_e0c3_9b2d_4f61);
    for case in 0..96 {
        let width = (rng.next() as usize % 90) + 1;
        let height = (rng.next() as usize % 80) + 1;
        let pixel_format = match case % 4 {
            0 => PixelFormat::Yuv420,
            1 => PixelFormat::Yuv422,
            2 => PixelFormat::Yuv444,
            _ => PixelFormat::Monochrome,
        };
        let mut model = film_grain_model();
        model.overlap_flag = rng.next() & 1 != 0;
        model.film_grain_block_size = rng.next() & 1 != 0;
        model.clip_to_restricted_range = rng.next() & 1 != 0;
        model.mc_identity = model.clip_to_restricted_range && rng.next() & 1 != 0;
        model.chroma_scaling_from_luma = rng.next().is_multiple_of(3);
        model.grain_scaling_minus_8 = (rng.next() % 4) as u8;
        model.cb_mult = Some(rng.next() as u8);
        model.cb_luma_mult = Some(rng.next() as u8);
        model.cb_offset = Some((rng.next() % 512) as u16);
        model.cr_mult = Some(rng.next() as u8);
        model.cr_luma_mult = Some(rng.next() as u8);
        model.cr_offset = Some((rng.next() % 512) as u16);
        for point in model
            .point_y
            .iter_mut()
            .chain(&mut model.point_cb)
            .chain(&mut model.point_cr)
        {
            point.scaling = rng.next() % 256;
        }
        let seed = rng.next() as u16;
        let count = sample_count(width, height, pixel_format);
        let eight = (0..count)
            .map(|_| (rng.next() % 256) as u16)
            .collect::<Vec<_>>();
        let ten = (0..count)
            .map(|_| (rng.next() % 1024) as u16)
            .collect::<Vec<_>>();
        for (bit_depth, samples) in [(BitDepth::Eight, &eight), (BitDepth::Ten, &ten)] {
            let wide = frame_at_depth::<u16>(width, height, pixel_format, bit_depth, samples);
            let expected = reference_output(&wide, &model, seed);
            assert_eq!(
                output_samples(&apply_film_grain(&wide, &model, seed).unwrap()),
                expected,
                "case {case} {bit_depth:?}"
            );
            if bit_depth == BitDepth::Eight {
                let narrow = frame_at_depth::<u8>(width, height, pixel_format, bit_depth, samples);
                assert_eq!(
                    output_samples(&apply_film_grain(&narrow, &model, seed).unwrap()),
                    expected,
                    "case {case} u8"
                );
            }
        }
    }
}
