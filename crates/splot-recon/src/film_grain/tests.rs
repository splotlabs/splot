// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::film_grain::FilmGrainModel;
use splot_core::obu::{ParsedObu, PayloadStatus};
use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
use splot_core::types::ObuType;

use super::{GrainDestination, apply_film_grain, plane_from_visible};
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
    let luma_len = width * height;
    let luma = samples[..luma_len]
        .iter()
        .copied()
        .map(u16::from)
        .map(T::try_from_u16)
        .collect::<crate::Result<Vec<_>>>()
        .unwrap();
    let luma_size = PlaneSize::new(width, height).unwrap();
    let chroma_size = pixel_format.chroma_size(luma_size).unwrap();
    let mut offset = luma_len;
    let (u, v) = if let Some(size) = chroma_size {
        let len = size.width() * size.height();
        let convert = |values: &[u8]| {
            values
                .iter()
                .copied()
                .map(u16::from)
                .map(T::try_from_u16)
                .collect::<crate::Result<Vec<_>>>()
                .unwrap()
        };
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
        BitDepth::Eight,
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
    samples: Vec<T>,
) -> Vec<T> {
    let destination = GrainDestination { samples };
    let pointer = destination.samples.as_ptr();
    let capacity = destination.samples.capacity();
    let plane = plane_from_visible(2, 2, destination).unwrap();
    let samples = plane.into_samples();

    assert_eq!(samples.as_ptr(), pointer);
    assert_eq!(samples.capacity(), capacity);
    samples
}

#[test]
fn u16_plane_retains_the_synthesis_allocation() {
    let mut samples = Vec::with_capacity(8);
    samples.extend([1_u16, 2, 3, 4]);
    assert_eq!(destination_allocation_is_retained(samples), [1, 2, 3, 4]);
}

#[test]
fn u8_plane_uses_its_single_final_sample_allocation() {
    let mut samples = Vec::with_capacity(8);
    samples.extend([1_u8, 2, 3, 4]);
    assert_eq!(destination_allocation_is_retained(samples), [1, 2, 3, 4]);
}

#[test]
fn visible_source_samples_are_copied_directly_in_destination_type() {
    let storage = PlaneSize::new(4, 3).unwrap();
    let visible = PlaneRect::new(1, 1, 2, 2).unwrap();
    let source = Plane::from_vec(
        storage,
        4,
        visible,
        vec![0_u8, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23],
    )
    .unwrap();
    let destination = GrainDestination::from_visible(&source);

    assert_eq!(destination.samples, [11, 12, 21, 22]);
}
