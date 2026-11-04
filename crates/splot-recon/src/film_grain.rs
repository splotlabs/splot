// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain synthesis.

use std::simd::{Mask, Simd, cmp::SimdOrd, num::SimdInt, num::SimdUint, simd_swizzle};

use splot_core::headers::film_grain::{FilmGrainModel, FilmGrainScalingPoint};
use splot_core::tables::conversion::GAUSSIAN_SEQUENCE;

use crate::wienerns_filter::{
    LumaSimdOutput, LumaSimdSource, interleave_pairs, multiply_add_pairs,
};
use crate::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, Plane, PlaneRect, PlaneSize,
    ReconSample, Result,
};

const LUMA_GRAIN_WIDTH: usize = 82;
const LUMA_GRAIN_HEIGHT: usize = 73;
const LUMA_GRAIN_BASE: usize = 3;
const MAX_AR_COEFFICIENTS: usize = 24;

struct ActiveFilmGrain<'a> {
    model: &'a FilmGrainModel,
    grain_seed: u16,
}

/// Applies AV2 § 7.21.7 display film grain to a decoded frame.
///
/// # Errors
///
/// Returns [`ReconError`](crate::ReconError) if the synthesized frame geometry
/// or sample representation cannot be constructed.
pub fn apply_film_grain<T: ReconSample>(
    frame: &DecodedFrame<T>,
    model: &FilmGrainModel,
    grain_seed: u16,
) -> Result<DecodedFrame<T>> {
    let grain = ActiveFilmGrain { model, grain_seed };
    apply_grain(frame, &grain)
}

fn apply_grain<T: ReconSample>(
    frame: &DecodedFrame<T>,
    grain: &ActiveFilmGrain<'_>,
) -> Result<DecodedFrame<T>> {
    let visible = frame.visible_luma_rect();
    let width = visible.width();
    let height = visible.height();
    let pixel_format = frame.pixel_format();
    let bit_depth = frame.bit_depth().bits();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let num_planes = pixel_format.num_planes();

    let mut y = GrainDestination::from_visible(frame.y());
    let mut u = frame.u().map(GrainDestination::from_visible);
    let mut v = frame.v().map(GrainDestination::from_visible);

    synthesize_into(
        &mut y,
        u.as_mut(),
        v.as_mut(),
        width,
        height,
        sub_x,
        sub_y,
        bit_depth,
        num_planes,
        grain,
    )?;

    let info = DecodedFrameInfo::new(
        OutputIndex::new(frame.output_index().get()),
        frame.bit_depth(),
        pixel_format,
        PlaneSize::new(width, height)?,
        PlaneRect::new(0, 0, width, height)?,
    )?;
    let y = plane_from_visible(width, height, y)?;
    let chroma_size = pixel_format.chroma_size(visible.size())?;
    let u = match (chroma_size, u) {
        (Some(size), Some(samples)) => {
            Some(plane_from_visible(size.width(), size.height(), samples)?)
        }
        _ => None,
    };
    let v = match (chroma_size, v) {
        (Some(size), Some(samples)) => {
            Some(plane_from_visible(size.width(), size.height(), samples)?)
        }
        _ => None,
    };

    DecodedFrame::try_new(info, FramePlanes::new(y, u, v))
}

struct GrainDestination<T: ReconSample> {
    samples: crate::PlaneBuf<T>,
    stride: usize,
}

impl<T: ReconSample> GrainDestination<T> {
    fn from_visible(plane: &Plane<T>) -> Self {
        let size = plane.visible_size();
        let stride = crate::plane_stride::<T>(size.width()).unwrap_or(size.width());
        let mut samples = crate::plane_buf();
        samples.reserve_exact(stride * size.height());
        for row in plane.visible_rows() {
            samples.extend_from_slice(row); // splot-copy-ok: final film-grain output materialization
            let padded = samples.len() + stride - row.len();
            samples.resize(padded, T::default());
        }
        Self { samples, stride }
    }
}

fn plane_from_visible<T: ReconSample>(
    width: usize,
    height: usize,
    destination: GrainDestination<T>,
) -> Result<Plane<T>> {
    let size = PlaneSize::new(width, height)?;
    let rect = PlaneRect::new(0, 0, width, height)?;
    Plane::from_buf(size, destination.stride, rect, destination.samples)
}

#[allow(clippy::too_many_arguments)]
fn synthesize_into<T: ReconSample>(
    y: &mut GrainDestination<T>,
    u: Option<&mut GrainDestination<T>>,
    v: Option<&mut GrainDestination<T>>,
    width: usize,
    height: usize,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    num_planes: usize,
    grain: &ActiveFilmGrain<'_>,
) -> Result<()> {
    let grain_min = -(1_i32 << (bit_depth - 1));
    let grain_max = (1_i32 << (bit_depth - 1)) - 1;
    let templates = generate_grain(grain, sub_x, sub_y, bit_depth, grain_min, grain_max);
    let scaling = scaling_luts(grain, num_planes);
    let noise = build_noise_image(
        width, height, sub_x, sub_y, num_planes, grain, &templates, grain_min, grain_max,
    );
    add_noise_to_samples(
        y, u, v, width, height, sub_x, sub_y, bit_depth, num_planes, grain, &scaling, &noise,
    )
}

struct GrainTemplates {
    y: Vec<i16>,
    cb: Vec<i16>,
    cr: Vec<i16>,
    chroma_width: usize,
}

fn generate_grain(
    grain: &ActiveFilmGrain<'_>,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    grain_min: i32,
    grain_max: i32,
) -> GrainTemplates {
    let model = grain.model;
    let mut rng = GrainRng::new(grain.grain_seed);
    let shift = 12 - bit_depth + model.grain_scale_shift;
    let mut luma = vec![0; LUMA_GRAIN_WIDTH * LUMA_GRAIN_HEIGHT];
    for row in 0..LUMA_GRAIN_HEIGHT {
        for col in 0..LUMA_GRAIN_WIDTH {
            let g = if model.num_y_points > 0 {
                GAUSSIAN_SEQUENCE[rng.get(11)]
            } else {
                0
            };
            luma[row * LUMA_GRAIN_WIDTH + col] = round2(g, shift);
        }
    }
    apply_luma_ar(model, &mut luma, grain_min, grain_max);

    let chroma_width = if sub_x == 1 { 44 } else { 82 };
    let chroma_height = if sub_y == 1 { 38 } else { 73 };
    let mut cb = vec![0; chroma_width * chroma_height];
    let mut cr = vec![0; chroma_width * chroma_height];
    fill_chroma_white_noise(
        &mut cb,
        chroma_width,
        chroma_height,
        grain.grain_seed ^ 0xb524,
        model.num_cb_points > 0 || model.chroma_scaling_from_luma,
        shift,
    );
    fill_chroma_white_noise(
        &mut cr,
        chroma_width,
        chroma_height,
        grain.grain_seed ^ 0x49d8,
        model.num_cr_points > 0 || model.chroma_scaling_from_luma,
        shift,
    );
    apply_chroma_ar(
        model,
        &luma,
        &mut cb,
        &mut cr,
        chroma_width,
        chroma_height,
        sub_x,
        sub_y,
        grain_min,
        grain_max,
    );

    GrainTemplates {
        y: narrow_grain(luma),
        cb: narrow_grain(cb),
        cr: narrow_grain(cr),
        chroma_width,
    }
}

/// Packs filtered grain into `i16` lanes; filtered samples are clamped to the
/// grain range and white-noise samples stay within the Gaussian magnitude.
fn narrow_grain(planes: Vec<i32>) -> Vec<i16> {
    planes.into_iter().map(|sample| sample as i16).collect()
}

fn apply_luma_ar(model: &FilmGrainModel, luma: &mut [i32], grain_min: i32, grain_max: i32) {
    let (offsets, offset_count) = ar_neighbor_offsets(model.ar_coeff_lag, LUMA_GRAIN_WIDTH);
    let shift = model.ar_coeff_shift_minus_6 + 6;
    for row in LUMA_GRAIN_BASE..LUMA_GRAIN_HEIGHT {
        for col in LUMA_GRAIN_BASE..(LUMA_GRAIN_WIDTH - LUMA_GRAIN_BASE) {
            let index = row * LUMA_GRAIN_WIDTH + col;
            let mut sum = 0;
            for (&offset, &coeff) in offsets[..offset_count].iter().zip(&model.ar_coeffs_y) {
                let source = (index as isize + offset) as usize;
                sum += luma[source] * coeff;
            }
            let sample = luma[index] + round2(sum, shift);
            luma[index] = sample.clamp(grain_min, grain_max);
        }
    }
}

fn fill_chroma_white_noise(
    out: &mut [i32],
    width: usize,
    height: usize,
    seed: u16,
    active: bool,
    shift: u8,
) {
    let mut rng = GrainRng::new(seed);
    for row in 0..height {
        for col in 0..width {
            let g = if active {
                GAUSSIAN_SEQUENCE[rng.get(11)]
            } else {
                0
            };
            out[row * width + col] = round2(g, shift);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_chroma_ar(
    model: &FilmGrainModel,
    luma: &[i32],
    cb: &mut [i32],
    cr: &mut [i32],
    width: usize,
    height: usize,
    sub_x: usize,
    sub_y: usize,
    grain_min: i32,
    grain_max: i32,
) {
    let (offsets, offset_count) = ar_neighbor_offsets(model.ar_coeff_lag, width);
    let luma_coeff_cb = model.ar_coeffs_cb.get(offset_count).copied().unwrap_or(0);
    let luma_coeff_cr = model.ar_coeffs_cr.get(offset_count).copied().unwrap_or(0);
    let shift = model.ar_coeff_shift_minus_6 + 6;
    for row in LUMA_GRAIN_BASE..height {
        for col in LUMA_GRAIN_BASE..(width - LUMA_GRAIN_BASE) {
            let index = row * width + col;
            let mut sum_cb = 0;
            let mut sum_cr = 0;
            for (pos, &offset) in offsets[..offset_count].iter().enumerate() {
                let source = (index as isize + offset) as usize;
                sum_cb += cb[source] * model.ar_coeffs_cb.get(pos).copied().unwrap_or(0);
                sum_cr += cr[source] * model.ar_coeffs_cr.get(pos).copied().unwrap_or(0);
            }
            if model.num_y_points > 0 {
                let luma_x = ((col - LUMA_GRAIN_BASE) << sub_x) + LUMA_GRAIN_BASE;
                let luma_y = ((row - LUMA_GRAIN_BASE) << sub_y) + LUMA_GRAIN_BASE;
                let mut average = 0;
                for y_offset in 0..=sub_y {
                    for x_offset in 0..=sub_x {
                        average += luma[(luma_y + y_offset) * LUMA_GRAIN_WIDTH + luma_x + x_offset];
                    }
                }
                let average = round2(average, (sub_x + sub_y) as u8);
                sum_cb += average * luma_coeff_cb;
                sum_cr += average * luma_coeff_cr;
            }
            cb[index] = (cb[index] + round2(sum_cb, shift)).clamp(grain_min, grain_max);
            cr[index] = (cr[index] + round2(sum_cr, shift)).clamp(grain_min, grain_max);
        }
    }
}

fn ar_neighbor_offsets(lag: u8, stride: usize) -> ([isize; MAX_AR_COEFFICIENTS], usize) {
    let lag = i32::from(lag);
    let mut offsets = [0; MAX_AR_COEFFICIENTS];
    let mut count = 0;
    'outer: for delta_row in -lag..=0 {
        for delta_col in -lag..=lag {
            if delta_row == 0 && delta_col == 0 {
                break 'outer;
            }
            offsets[count] = delta_row as isize * stride as isize + delta_col as isize;
            count += 1;
        }
    }
    (offsets, count)
}

const SCALING_LUT_LEN: usize = 257;

fn scaling_luts(grain: &ActiveFilmGrain<'_>, num_planes: usize) -> Vec<[i32; SCALING_LUT_LEN]> {
    let mut luts = vec![[0; SCALING_LUT_LEN]; num_planes];
    for (plane, lut) in luts.iter_mut().enumerate() {
        let points = scaling_points_for_plane(grain.model, plane);
        init_scaling_lut(points, lut);
    }
    luts
}

fn scaling_points_for_plane(model: &FilmGrainModel, plane: usize) -> &[FilmGrainScalingPoint] {
    if plane == 0 || model.chroma_scaling_from_luma {
        &model.point_y
    } else if plane == 1 {
        &model.point_cb
    } else {
        &model.point_cr
    }
}

fn init_scaling_lut(points: &[FilmGrainScalingPoint], lut: &mut [i32; SCALING_LUT_LEN]) {
    if points.is_empty() {
        return;
    }
    let first_x = points[0].value.min(255) as usize;
    let first_y = points[0].scaling as i32;
    for value in lut[..256].iter_mut().take(first_x) {
        *value = first_y;
    }
    for pair in points.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let start_x = start.value.min(255) as usize;
        let end_x = end.value.min(255) as usize;
        if end_x <= start_x {
            continue;
        }
        let delta_y = end.scaling as i32 - start.scaling as i32;
        let delta_x = (end_x - start_x) as i32;
        let delta = delta_y * ((65536 + (delta_x >> 1)) / delta_x);
        for x in 0..(end_x - start_x) {
            lut[start_x + x] = start.scaling as i32 + (((x as i32) * delta + 32768) >> 16);
        }
    }
    let last = points[points.len() - 1];
    for value in lut[..256].iter_mut().skip(last.value.min(255) as usize) {
        *value = last.scaling as i32;
    }
    lut[256] = lut[255];
}

struct NoiseImage {
    planes: Vec<Vec<i16>>,
    widths: [usize; 3],
}

#[allow(clippy::too_many_arguments)]
fn build_noise_image(
    width: usize,
    height: usize,
    sub_x: usize,
    sub_y: usize,
    num_planes: usize,
    grain: &ActiveFilmGrain<'_>,
    templates: &GrainTemplates,
    grain_min: i32,
    grain_max: i32,
) -> NoiseImage {
    let luma_size = if grain.model.film_grain_block_size {
        32
    } else {
        16
    };
    let stripe_count = stripe_count(height, luma_size);
    let plane_widths = plane_axis_lengths(width, sub_x);
    let plane_heights = plane_axis_lengths(height, sub_y);
    let mut planes: Vec<Vec<i16>> = (0..num_planes)
        .map(|plane| Vec::with_capacity(plane_widths[plane] * plane_heights[plane]))
        .collect();
    let mut stripes = [
        Stripe::new(&plane_widths, luma_size, sub_x, sub_y),
        Stripe::new(&plane_widths, luma_size, sub_x, sub_y),
    ];

    for luma_num in 0..stripe_count {
        let current = luma_num & 1;
        fill_stripe(
            &mut stripes[current],
            luma_num,
            width,
            sub_x,
            sub_y,
            num_planes,
            luma_size,
            grain,
            templates,
            grain_min,
            grain_max,
        );
        for plane in 0..num_planes {
            let plane_sub_y = if plane > 0 { sub_y } else { 0 };
            let stripe_shift = 4 + usize::from(grain.model.film_grain_block_size) - plane_sub_y;
            let blend_rows = if plane_sub_y == 0 { 2 } else { 1 };
            let stripe_width = stripes[current].widths[plane];
            let buffer = &stripes[current].planes[plane];
            let previous = &stripes[current ^ 1].planes[plane];
            let destination = &mut planes[plane];
            for stripe_row in 0..(1 << stripe_shift) {
                let row = (luma_num << stripe_shift) + stripe_row;
                if row >= plane_heights[plane] {
                    break;
                }
                let source =
                    stripe_row_slice(buffer, stripe_width, stripe_row, plane_widths[plane]);
                let blended = grain.model.overlap_flag && luma_num > 0 && stripe_row < blend_rows;
                if !blended {
                    destination.extend_from_slice(source);
                    continue;
                }
                let old_row = stripe_row
                    + if plane_sub_y == 0 {
                        luma_size
                    } else {
                        luma_size >> 1
                    };
                let old = stripe_row_slice(previous, stripe_width, old_row, plane_widths[plane]);
                let old_weight = if plane_sub_y == 0 && stripe_row == 0 {
                    27
                } else if plane_sub_y == 0 {
                    17
                } else {
                    23
                };
                let new_weight = if plane_sub_y == 0 && stripe_row == 0 {
                    17
                } else if plane_sub_y == 0 {
                    27
                } else {
                    22
                };
                for (&old_sample, &new_sample) in old.iter().zip(source) {
                    let sample = round2(
                        i32::from(old_sample) * old_weight + i32::from(new_sample) * new_weight,
                        5,
                    );
                    destination.push(sample.clamp(grain_min, grain_max) as i16);
                }
            }
        }
    }

    NoiseImage {
        planes,
        widths: plane_widths,
    }
}

fn stripe_row_slice(stripe: &[i16], stripe_width: usize, row: usize, width: usize) -> &[i16] {
    &stripe[row * stripe_width..row * stripe_width + width]
}

struct Stripe {
    planes: Vec<Vec<i16>>,
    widths: Vec<usize>,
}

impl Stripe {
    fn new(plane_widths: &[usize], luma_size: usize, sub_x: usize, sub_y: usize) -> Self {
        let mut planes = Vec::with_capacity(plane_widths.len());
        let mut widths = Vec::with_capacity(plane_widths.len());
        for (plane, &width) in plane_widths.iter().enumerate() {
            let plane_sub_x = if plane > 0 { sub_x } else { 0 };
            let plane_sub_y = if plane > 0 { sub_y } else { 0 };
            let stripe_width = width + ((luma_size + 2) >> plane_sub_x) + 2;
            let stripe_height = (luma_size + 2) >> plane_sub_y;
            widths.push(stripe_width);
            planes.push(vec![0; stripe_width * stripe_height]);
        }
        Self { planes, widths }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_stripe(
    stripe: &mut Stripe,
    luma_num: usize,
    width: usize,
    sub_x: usize,
    sub_y: usize,
    num_planes: usize,
    luma_size: usize,
    grain: &ActiveFilmGrain<'_>,
    templates: &GrainTemplates,
    grain_min: i32,
    grain_max: i32,
) {
    let mut rng = GrainRng::new(grain.grain_seed);
    let luma_rand = (luma_num * (luma_size >> 1)) >> 3;
    rng.register ^= (((luma_rand * 37 + 178) & 255) as u16) << 8;
    rng.register ^= ((luma_rand * 173 + 105) & 255) as u16;
    let step = luma_size >> 1;
    let half_width = width.div_ceil(2);
    let mut block_x = 0usize;
    while block_x < half_width {
        let offset_y = (rng.get(9) * (3 - usize::from(grain.model.film_grain_block_size))) >> 6;
        let _ = rng.get(1);
        let _ = rng.get(1);
        let _ = rng.get(1);
        let offset_x = (rng.get(9) * (3 - usize::from(grain.model.film_grain_block_size))) >> 6;
        let _ = rng.get(1);
        let _ = rng.get(1);
        let _ = rng.get(1);
        for plane in 0..num_planes {
            fill_stripe_plane(
                stripe, plane, block_x, offset_x, offset_y, sub_x, sub_y, luma_size, grain,
                templates, grain_min, grain_max,
            );
        }
        block_x += step;
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_stripe_plane(
    stripe: &mut Stripe,
    plane: usize,
    block_x: usize,
    offset_x: usize,
    offset_y: usize,
    sub_x: usize,
    sub_y: usize,
    luma_size: usize,
    grain: &ActiveFilmGrain<'_>,
    templates: &GrainTemplates,
    grain_min: i32,
    grain_max: i32,
) {
    let plane_sub_x = if plane > 0 { sub_x } else { 0 };
    let plane_sub_y = if plane > 0 { sub_y } else { 0 };
    let plane_offset_x = if plane_sub_x == 1 {
        6 + offset_x
    } else {
        9 + offset_x * 2
    };
    let plane_offset_y = if plane_sub_y == 1 {
        6 + offset_y
    } else {
        9 + offset_y * 2
    };
    let block_height = (luma_size + 2) >> plane_sub_y;
    let block_width = (luma_size + 2) >> plane_sub_x;
    let (template, template_width) = template_plane(templates, plane);
    let dst_col = if plane_sub_x == 0 {
        block_x * 2
    } else {
        block_x
    };
    let blended_cols = if !grain.model.overlap_flag || block_x == 0 {
        0
    } else if plane_sub_x == 0 {
        2
    } else {
        1
    };
    for row in 0..block_height {
        let source =
            &template[(plane_offset_y + row) * template_width + plane_offset_x..][..block_width];
        let destination = row * stripe.widths[plane] + dst_col;
        let target = &mut stripe.planes[plane][destination..destination + block_width];
        if blended_cols == 0 {
            target.copy_from_slice(source);
            continue;
        }
        let old = [i32::from(target[0]), i32::from(target[1])];
        target.copy_from_slice(source);
        for col in 0..blended_cols {
            let sample = if plane_sub_x == 0 {
                if col == 0 {
                    round2(old[0] * 27 + i32::from(source[col]) * 17, 5)
                } else {
                    round2(old[1] * 17 + i32::from(source[col]) * 27, 5)
                }
            } else {
                round2(old[0] * 23 + i32::from(source[col]) * 22, 5)
            };
            target[col] = sample.clamp(grain_min, grain_max) as i16;
        }
    }
}

fn template_plane(templates: &GrainTemplates, plane: usize) -> (&[i16], usize) {
    match plane {
        0 => (&templates.y, LUMA_GRAIN_WIDTH),
        1 => (&templates.cb, templates.chroma_width),
        _ => (&templates.cr, templates.chroma_width),
    }
}

#[allow(clippy::too_many_arguments)]
fn add_noise_to_samples<T: ReconSample>(
    y: &mut GrainDestination<T>,
    u: Option<&mut GrainDestination<T>>,
    v: Option<&mut GrainDestination<T>>,
    width: usize,
    height: usize,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    num_planes: usize,
    grain: &ActiveFilmGrain<'_>,
    scaling: &[[i32; SCALING_LUT_LEN]],
    noise: &NoiseImage,
) -> Result<()> {
    let (min_value, max_luma, max_chroma) = output_ranges(grain, bit_depth);
    let scaling_shift = grain.model.grain_scaling_minus_8 + 8;
    if num_planes > 1 {
        let chroma_width = (width + sub_x) >> sub_x;
        let chroma_height = (height + sub_y) >> sub_y;
        if let (Some(u), Some(v)) = (u, v) {
            add_chroma_noise(
                y,
                u,
                v,
                width,
                chroma_width,
                chroma_height,
                sub_x,
                sub_y,
                bit_depth,
                max_chroma,
                min_value,
                scaling_shift,
                grain,
                scaling,
                noise,
            )?;
        }
    }
    if grain.model.num_y_points > 0 {
        for row in 0..height {
            add_luma_noise_row(
                &mut y.samples[row * y.stride..][..width],
                &noise.planes[0][row * noise.widths[0]..][..width],
                &scaling[0],
                bit_depth,
                scaling_shift,
                min_value,
                max_luma,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_luma_noise_row<T: ReconSample>(
    row: &mut [T],
    noise_row: &[i16],
    lut: &[i32; SCALING_LUT_LEN],
    bit_depth: u8,
    scaling_shift: u8,
    min_value: i32,
    max_luma: i32,
) -> Result<()> {
    if i32::from(T::MAX_VALUE) >= max_luma {
        if let Some(row) = T::u8_slice_mut(row) {
            add_luma_noise_row_lanes::<u8>(
                row,
                noise_row,
                lut,
                bit_depth,
                scaling_shift,
                min_value,
                max_luma,
            );
            return Ok(());
        }
        if let Some(row) = T::u16_slice_mut(row) {
            add_luma_noise_row_lanes::<u16>(
                row,
                noise_row,
                lut,
                bit_depth,
                scaling_shift,
                min_value,
                max_luma,
            );
            return Ok(());
        }
    }
    for (col, &noise_sample) in noise_row.iter().enumerate() {
        let orig = i32::from(row[col].to_u16());
        let scaled = scale_lut(lut, orig, bit_depth);
        let delta = round2(scaled * i32::from(noise_sample), scaling_shift);
        row[col] = T::try_from_u16((orig + delta).clamp(min_value, max_luma) as u16)?;
    }
    Ok(())
}

fn add_luma_noise_row_lanes<S: LumaSimdSource + LumaSimdOutput>(
    row: &mut [S],
    noise_row: &[i16],
    lut: &[i32; SCALING_LUT_LEN],
    bit_depth: u8,
    scaling_shift: u8,
    min_value: i32,
    max_luma: i32,
) {
    let shift = bit_depth - 8;
    let mut x = 0;
    while x + 16 <= row.len() {
        let orig = S::load::<16>(row, x);
        let scaled = scaling_lut_lanes(lut, orig.cast::<i32>(), shift);
        let lanes = Simd::<i16, 16>::from_slice(&noise_row[x..]);
        add_scaled_noise_lanes::<S>(
            row,
            x,
            orig,
            scaled,
            lanes,
            scaling_shift,
            min_value,
            max_luma,
        );
        x += 16;
    }
    for (col, &noise_sample) in noise_row.iter().enumerate().skip(x) {
        let orig = i32::from(S::scalar(row, col));
        let scaled = scale_lut(lut, orig, bit_depth);
        let delta = round2(scaled * i32::from(noise_sample), scaling_shift);
        row[col] = S::from_u16((orig + delta).clamp(min_value, max_luma) as u16);
    }
}

#[allow(clippy::too_many_arguments)]
fn add_chroma_noise<T: ReconSample>(
    y: &GrainDestination<T>,
    u: &mut GrainDestination<T>,
    v: &mut GrainDestination<T>,
    luma_width: usize,
    chroma_width: usize,
    chroma_height: usize,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    max_chroma: i32,
    min_value: i32,
    scaling_shift: u8,
    grain: &ActiveFilmGrain<'_>,
    scaling: &[[i32; SCALING_LUT_LEN]],
    noise: &NoiseImage,
) -> Result<()> {
    let model = grain.model;
    for row in 0..chroma_height {
        let luma_row = &y.samples[(row << sub_y) * y.stride..];
        if model.num_cb_points > 0 || model.chroma_scaling_from_luma {
            add_chroma_noise_row(
                &mut u.samples[row * u.stride..][..chroma_width],
                luma_row,
                &noise.planes[1][row * noise.widths[1]..][..chroma_width],
                &scaling[1],
                luma_width,
                sub_x,
                bit_depth,
                scaling_shift,
                min_value,
                max_chroma,
                model.cb_luma_mult,
                model.cb_mult,
                model.cb_offset,
                model.chroma_scaling_from_luma,
            )?;
        }
        if model.num_cr_points > 0 || model.chroma_scaling_from_luma {
            add_chroma_noise_row(
                &mut v.samples[row * v.stride..][..chroma_width],
                luma_row,
                &noise.planes[2][row * noise.widths[2]..][..chroma_width],
                &scaling[2],
                luma_width,
                sub_x,
                bit_depth,
                scaling_shift,
                min_value,
                max_chroma,
                model.cr_luma_mult,
                model.cr_mult,
                model.cr_offset,
                model.chroma_scaling_from_luma,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_chroma_noise_row<T: ReconSample>(
    chroma_row: &mut [T],
    luma_row: &[T],
    noise_row: &[i16],
    lut: &[i32; SCALING_LUT_LEN],
    luma_width: usize,
    sub_x: usize,
    bit_depth: u8,
    scaling_shift: u8,
    min_value: i32,
    max_chroma: i32,
    luma_mult: Option<u8>,
    mult: Option<u8>,
    offset: Option<u16>,
    from_luma: bool,
) -> Result<()> {
    if i32::from(T::MAX_VALUE) >= max_chroma {
        if let (Some(chroma), Some(luma)) =
            (T::u8_slice_mut(&mut *chroma_row), T::u8_slice(luma_row))
        {
            add_chroma_noise_row_lanes::<u8>(
                chroma,
                luma,
                noise_row,
                lut,
                luma_width,
                sub_x,
                bit_depth,
                scaling_shift,
                min_value,
                max_chroma,
                luma_mult,
                mult,
                offset,
                from_luma,
            );
            return Ok(());
        }
        if let (Some(chroma), Some(luma)) =
            (T::u16_slice_mut(&mut *chroma_row), T::u16_slice(luma_row))
        {
            add_chroma_noise_row_lanes::<u16>(
                chroma,
                luma,
                noise_row,
                lut,
                luma_width,
                sub_x,
                bit_depth,
                scaling_shift,
                min_value,
                max_chroma,
                luma_mult,
                mult,
                offset,
                from_luma,
            );
            return Ok(());
        }
    }
    let sample_max = (1_i32 << bit_depth) - 1;
    for col in 0..chroma_row.len() {
        let luma_x = col << sub_x;
        let luma_next_x = (luma_x + 1).min(luma_width - 1);
        let average_luma = if sub_x == 1 {
            round2(
                i32::from(luma_row[luma_x].to_u16()) + i32::from(luma_row[luma_next_x].to_u16()),
                1,
            )
        } else {
            i32::from(luma_row[luma_x].to_u16())
        };
        let orig = i32::from(chroma_row[col].to_u16());
        let merged = chroma_merged_sample(
            average_luma,
            orig,
            luma_mult,
            mult,
            offset,
            from_luma,
            bit_depth,
            sample_max,
        );
        let scaled = scale_lut(lut, merged, bit_depth);
        let delta = round2(scaled * i32::from(noise_row[col]), scaling_shift);
        chroma_row[col] = T::try_from_u16((orig + delta).clamp(min_value, max_chroma) as u16)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add_chroma_noise_row_lanes<S: LumaSimdSource + LumaSimdOutput>(
    chroma_row: &mut [S],
    luma_row: &[S],
    noise_row: &[i16],
    lut: &[i32; SCALING_LUT_LEN],
    luma_width: usize,
    sub_x: usize,
    bit_depth: u8,
    scaling_shift: u8,
    min_value: i32,
    max_chroma: i32,
    luma_mult: Option<u8>,
    mult: Option<u8>,
    offset: Option<u16>,
    from_luma: bool,
) {
    let shift = bit_depth - 8;
    let weights: Simd<i16, 16> = Simd::from_array(core::array::from_fn(|lane| {
        if lane % 2 == 0 {
            i16::from(luma_mult.unwrap_or(128)) - 128
        } else {
            i16::from(mult.unwrap_or(128)) - 128
        }
    }));
    let mut x = 0;
    while x + 16 <= chroma_row.len() && (sub_x == 0 || x * 2 + 32 <= luma_width) {
        let orig = S::load::<16>(chroma_row, x);
        let average_luma = if sub_x == 1 {
            let pairs = S::load::<32>(luma_row, x * 2);
            let even: Simd<u16, 16> = simd_swizzle!(
                pairs,
                [0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30]
            );
            let odd: Simd<u16, 16> = simd_swizzle!(
                pairs,
                [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31]
            );
            (even.cast::<i32>() + odd.cast::<i32>() + Simd::splat(1)) >> Simd::splat(1)
        } else {
            S::load::<16>(luma_row, x).cast::<i32>()
        };
        let merged = if from_luma {
            average_luma
        } else {
            let combined =
                merged_combined_lanes(average_luma.cast::<i16>(), orig.cast::<i16>(), weights);
            ((combined >> Simd::splat(6)) + Simd::splat(chroma_offset_term(offset, bit_depth)))
                .simd_max(Simd::splat(0))
                .simd_min(Simd::splat((1_i32 << bit_depth) - 1))
        };
        let scaled = scaling_lut_lanes(lut, merged, shift);
        let lanes = Simd::<i16, 16>::from_slice(&noise_row[x..]);
        add_scaled_noise_lanes::<S>(
            chroma_row,
            x,
            orig,
            scaled,
            lanes,
            scaling_shift,
            min_value,
            max_chroma,
        );
        x += 16;
    }
    let sample_max = (1_i32 << bit_depth) - 1;
    for col in x..chroma_row.len() {
        let luma_x = col << sub_x;
        let luma_next_x = (luma_x + 1).min(luma_width - 1);
        let average_luma = if sub_x == 1 {
            round2(
                i32::from(S::scalar(luma_row, luma_x))
                    + i32::from(S::scalar(luma_row, luma_next_x)),
                1,
            )
        } else {
            i32::from(S::scalar(luma_row, luma_x))
        };
        let orig = i32::from(S::scalar(chroma_row, col));
        let merged = chroma_merged_sample(
            average_luma,
            orig,
            luma_mult,
            mult,
            offset,
            from_luma,
            bit_depth,
            sample_max,
        );
        let scaled = scale_lut(lut, merged, bit_depth);
        let delta = round2(scaled * i32::from(noise_row[col]), scaling_shift);
        chroma_row[col] = S::from_u16((orig + delta).clamp(min_value, max_chroma) as u16);
    }
}

#[inline]
fn merged_combined_lanes(
    average: Simd<i16, 16>,
    orig: Simd<i16, 16>,
    weights: Simd<i16, 16>,
) -> Simd<i32, 16> {
    let [low, high] = interleave_pairs(average, orig);
    let low = multiply_add_pairs(low, weights);
    let high = multiply_add_pairs(high, weights);
    simd_swizzle!(
        low,
        high,
        [0, 1, 2, 3, 8, 9, 10, 11, 4, 5, 6, 7, 12, 13, 14, 15]
    )
}

fn chroma_offset_term(offset: Option<u16>, bit_depth: u8) -> i32 {
    (i32::from(offset.unwrap_or(256)) - 256) << (bit_depth - 8)
}

#[inline]
fn scaling_lut_lanes(
    lut: &[i32; SCALING_LUT_LEN],
    index: Simd<i32, 16>,
    shift: u8,
) -> Simd<i32, 16> {
    let shift_lanes = Simd::splat(i32::from(shift));
    let x = index >> shift_lanes;
    let x = x.simd_max(Simd::splat(0)).simd_min(Simd::splat(255));
    let rem = index - (x << shift_lanes);
    let empty = Simd::<i32, 16>::splat(0);
    let select = Mask::<isize, 16>::splat(true);
    let start = Simd::<i32, 16>::gather_select(lut, select, x.cast::<usize>(), empty);
    let end = Simd::<i32, 16>::gather_select(&lut[1..], select, x.cast::<usize>(), empty);
    let half = Simd::splat((1_i32 << shift) >> 1);
    start + (((end - start) * rem + half) >> shift_lanes)
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn add_scaled_noise_lanes<T: LumaSimdOutput>(
    row: &mut [T],
    x: usize,
    orig: Simd<u16, 16>,
    scaled: Simd<i32, 16>,
    noise: Simd<i16, 16>,
    scaling_shift: u8,
    min_value: i32,
    max_value: i32,
) {
    let delta = (scaled * noise.cast::<i32>() + Simd::splat((1_i32 << scaling_shift) >> 1))
        >> Simd::splat(i32::from(scaling_shift));
    let out = orig.cast::<i32>() + delta;
    let out = out
        .simd_max(Simd::splat(min_value))
        .simd_min(Simd::splat(max_value));
    T::write(&mut row[x..], out.cast::<u16>());
}

#[allow(clippy::too_many_arguments)]
fn chroma_merged_sample(
    average_luma: i32,
    orig: i32,
    luma_mult: Option<u8>,
    mult: Option<u8>,
    offset: Option<u16>,
    from_luma: bool,
    bit_depth: u8,
    sample_max: i32,
) -> i32 {
    if from_luma {
        return average_luma;
    }
    let combined = average_luma * (i32::from(luma_mult.unwrap_or(128)) - 128)
        + orig * (i32::from(mult.unwrap_or(128)) - 128);
    ((combined >> 6) + ((i32::from(offset.unwrap_or(256)) - 256) << (bit_depth - 8)))
        .clamp(0, sample_max)
}

fn output_ranges(grain: &ActiveFilmGrain<'_>, bit_depth: u8) -> (i32, i32, i32) {
    if grain.model.clip_to_restricted_range {
        let min_value = 16 << (bit_depth - 8);
        let max_luma = 235 << (bit_depth - 8);
        let max_chroma = if grain.model.mc_identity {
            max_luma
        } else {
            240 << (bit_depth - 8)
        };
        (min_value, max_luma, max_chroma)
    } else {
        let max = (256 << (bit_depth - 8)) - 1;
        (0, max, max)
    }
}

fn stripe_count(height: usize, luma_size: usize) -> usize {
    let step = luma_size >> 1;
    let half_height = height.div_ceil(2);
    half_height.div_ceil(step)
}

fn plane_axis_lengths(length: usize, subsampling: usize) -> [usize; 3] {
    let chroma = (length + subsampling) >> subsampling;
    [length, chroma, chroma]
}

fn scale_lut(lut: &[i32; SCALING_LUT_LEN], index: i32, bit_depth: u8) -> i32 {
    let shift = bit_depth - 8;
    let x = (index >> shift).clamp(0, 255) as usize;
    let rem = index - ((x as i32) << shift);
    let start = lut[x];
    start + round2((lut[x + 1] - start) * rem, shift)
}

fn round2(value: i32, shift: u8) -> i32 {
    if shift == 0 {
        value
    } else {
        (value + (1_i32 << (shift - 1))) >> shift
    }
}

struct GrainRng {
    register: u16,
}

impl GrainRng {
    const fn new(seed: u16) -> Self {
        Self { register: seed }
    }

    fn get(&mut self, bits: usize) -> usize {
        let bit =
            ((self.register) ^ (self.register >> 1) ^ (self.register >> 3) ^ (self.register >> 12))
                & 1;
        self.register = (self.register >> 1) | (bit << 15);
        usize::from((self.register >> (16 - bits)) & ((1_u16 << bits) - 1))
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used)]
mod tests;
