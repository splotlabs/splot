// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain synthesis.

use splot_core::headers::film_grain::{FilmGrainModel, FilmGrainScalingPoint};
use splot_core::tables::conversion::GAUSSIAN_SEQUENCE;

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
    samples: Vec<T>,
}

impl<T: ReconSample> GrainDestination<T> {
    fn from_visible(plane: &Plane<T>) -> Self {
        let mut samples =
            Vec::with_capacity(plane.visible_size().width() * plane.visible_size().height());
        for row in plane.visible_rows() {
            samples.extend_from_slice(row); // splot-copy-ok: final film-grain output materialization
        }
        Self { samples }
    }

    fn get(&self, index: usize) -> i32 {
        i32::from(self.samples[index].to_u16())
    }

    fn set(&mut self, index: usize, value: u16) -> Result<()> {
        self.samples[index] = T::try_from_u16(value)?;
        Ok(())
    }
}

fn plane_from_visible<T: ReconSample>(
    width: usize,
    height: usize,
    destination: GrainDestination<T>,
) -> Result<Plane<T>> {
    let size = PlaneSize::new(width, height)?;
    let rect = PlaneRect::new(0, 0, width, height)?;
    Plane::from_vec(size, width, rect, destination.samples)
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
    y: Vec<i32>,
    cb: Vec<i32>,
    cr: Vec<i32>,
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
        y: luma,
        cb,
        cr,
        chroma_width,
    }
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

fn scaling_luts(grain: &ActiveFilmGrain<'_>, num_planes: usize) -> Vec<[i32; 256]> {
    let mut luts = vec![[0; 256]; num_planes];
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

fn init_scaling_lut(points: &[FilmGrainScalingPoint], lut: &mut [i32; 256]) {
    if points.is_empty() {
        return;
    }
    let first_x = points[0].value.min(255) as usize;
    let first_y = points[0].scaling as i32;
    for value in lut.iter_mut().take(first_x) {
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
    for value in lut.iter_mut().skip(last.value.min(255) as usize) {
        *value = last.scaling as i32;
    }
}

struct NoiseImage {
    planes: Vec<Vec<i32>>,
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
    let mut stripes = (0..stripe_count)
        .map(|_| Stripe::new(&plane_widths, luma_size, sub_x, sub_y))
        .collect::<Vec<_>>();

    for (luma_num, stripe) in stripes.iter_mut().enumerate() {
        fill_stripe(
            stripe, luma_num, width, sub_x, sub_y, num_planes, luma_size, grain, templates,
            grain_min, grain_max,
        );
    }

    let mut planes = Vec::with_capacity(num_planes);
    for plane in 0..num_planes {
        planes.push(vec![0; plane_widths[plane] * plane_heights[plane]]);
    }
    for plane in 0..num_planes {
        let plane_sub_y = if plane > 0 { sub_y } else { 0 };
        let stripe_shift = 4 + usize::from(grain.model.film_grain_block_size) - plane_sub_y;
        if !grain.model.overlap_flag {
            for row in 0..plane_heights[plane] {
                let luma_num = row >> stripe_shift;
                let stripe_row = row - (luma_num << stripe_shift);
                let stripe = &stripes[luma_num];
                let source = stripe_row * stripe.widths[plane];
                let destination = row * plane_widths[plane];
                planes[plane][destination..destination + plane_widths[plane]]
                    .copy_from_slice(&stripe.planes[plane][source..source + plane_widths[plane]]); // splot-copy-ok: assembled film-grain noise image
            }
            continue;
        }
        for row in 0..plane_heights[plane] {
            let luma_num = row >> stripe_shift;
            let stripe_row = row - (luma_num << stripe_shift);
            for col in 0..plane_widths[plane] {
                let mut sample = stripes[luma_num].get(plane, stripe_row, col);
                if luma_num > 0 {
                    if plane_sub_y == 0 && stripe_row < 2 {
                        let old = stripes[luma_num - 1].get(plane, stripe_row + luma_size, col);
                        sample = if stripe_row == 0 {
                            round2(old * 27 + sample * 17, 5)
                        } else {
                            round2(old * 17 + sample * 27, 5)
                        };
                        sample = sample.clamp(grain_min, grain_max);
                    } else if plane_sub_y == 1 && stripe_row < 1 {
                        let old =
                            stripes[luma_num - 1].get(plane, stripe_row + (luma_size >> 1), col);
                        sample = round2(old * 23 + sample * 22, 5).clamp(grain_min, grain_max);
                    }
                }
                planes[plane][row * plane_widths[plane] + col] = sample;
            }
        }
    }

    NoiseImage {
        planes,
        widths: plane_widths,
    }
}

struct Stripe {
    planes: Vec<Vec<i32>>,
    widths: Vec<usize>,
    heights: Vec<usize>,
}

impl Stripe {
    fn new(plane_widths: &[usize], luma_size: usize, sub_x: usize, sub_y: usize) -> Self {
        let mut planes = Vec::with_capacity(plane_widths.len());
        let mut widths = Vec::with_capacity(plane_widths.len());
        let mut heights = Vec::with_capacity(plane_widths.len());
        for (plane, &width) in plane_widths.iter().enumerate() {
            let plane_sub_x = if plane > 0 { sub_x } else { 0 };
            let plane_sub_y = if plane > 0 { sub_y } else { 0 };
            let stripe_width = width + ((luma_size + 2) >> plane_sub_x) + 2;
            let stripe_height = (luma_size + 2) >> plane_sub_y;
            widths.push(stripe_width);
            heights.push(stripe_height);
            planes.push(vec![0; stripe_width * stripe_height]);
        }
        Self {
            planes,
            widths,
            heights,
        }
    }

    fn get(&self, plane: usize, row: usize, col: usize) -> i32 {
        if row >= self.heights[plane] || col >= self.widths[plane] {
            0
        } else {
            self.planes[plane][row * self.widths[plane] + col]
        }
    }

    fn set(&mut self, plane: usize, row: usize, col: usize, value: i32) {
        if row < self.heights[plane] && col < self.widths[plane] {
            self.planes[plane][row * self.widths[plane] + col] = value;
        }
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
    if !grain.model.overlap_flag {
        let (template, template_width) = match plane {
            0 => (&templates.y, LUMA_GRAIN_WIDTH),
            1 => (&templates.cb, templates.chroma_width),
            _ => (&templates.cr, templates.chroma_width),
        };
        let dst_col = if plane_sub_x == 0 {
            block_x * 2
        } else {
            block_x
        };
        for row in 0..block_height {
            let source = (plane_offset_y + row) * template_width + plane_offset_x;
            let destination = row * stripe.widths[plane] + dst_col;
            stripe.planes[plane][destination..destination + block_width]
                .copy_from_slice(&template[source..source + block_width]); // splot-copy-ok: no-overlap grain block publication
        }
        return;
    }
    for row in 0..block_height {
        for col in 0..block_width {
            let mut sample =
                template_sample(templates, plane, plane_offset_y + row, plane_offset_x + col);
            let dst_col = if plane_sub_x == 0 {
                block_x * 2 + col
            } else {
                block_x + col
            };
            if block_x > 0 {
                if plane_sub_x == 0 && col < 2 {
                    let old = stripe.get(plane, row, dst_col);
                    sample = if col == 0 {
                        round2(old * 27 + sample * 17, 5)
                    } else {
                        round2(old * 17 + sample * 27, 5)
                    };
                    sample = sample.clamp(grain_min, grain_max);
                } else if plane_sub_x == 1 && col == 0 {
                    let old = stripe.get(plane, row, dst_col);
                    sample = round2(old * 23 + sample * 22, 5).clamp(grain_min, grain_max);
                }
            }
            stripe.set(plane, row, dst_col, sample);
        }
    }
}

fn template_sample(templates: &GrainTemplates, plane: usize, row: usize, col: usize) -> i32 {
    match plane {
        0 => templates.y[row * LUMA_GRAIN_WIDTH + col],
        1 => templates.cb[row * templates.chroma_width + col],
        _ => templates.cr[row * templates.chroma_width + col],
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
    scaling: &[[i32; 256]],
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
            for col in 0..width {
                let index = row * width + col;
                let orig = y.get(index);
                let scaled = scale_lut(&scaling[0], orig, bit_depth);
                let noise_sample = noise.planes[0][row * noise.widths[0] + col];
                let delta = round2(scaled * noise_sample, scaling_shift);
                y.set(index, (orig + delta).clamp(min_value, max_luma) as u16)?;
            }
        }
    }
    Ok(())
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
    scaling: &[[i32; 256]],
    noise: &NoiseImage,
) -> Result<()> {
    let model = grain.model;
    let sample_max = (1_i32 << bit_depth) - 1;
    for row in 0..chroma_height {
        for col in 0..chroma_width {
            let luma_x = col << sub_x;
            let luma_y = row << sub_y;
            let luma_next_x = (luma_x + 1).min(luma_width - 1);
            let average_luma = if sub_x == 1 {
                round2(
                    y.get(luma_y * luma_width + luma_x) + y.get(luma_y * luma_width + luma_next_x),
                    1,
                )
            } else {
                y.get(luma_y * luma_width + luma_x)
            };
            let index = row * chroma_width + col;
            if model.num_cb_points > 0 || model.chroma_scaling_from_luma {
                let orig = u.get(index);
                let merged = chroma_merged_sample(
                    average_luma,
                    orig,
                    model.cb_luma_mult,
                    model.cb_mult,
                    model.cb_offset,
                    model.chroma_scaling_from_luma,
                    bit_depth,
                    sample_max,
                );
                let delta = round2(
                    scale_lut(&scaling[1], merged, bit_depth) * noise.planes[1][index],
                    scaling_shift,
                );
                u.set(index, (orig + delta).clamp(min_value, max_chroma) as u16)?;
            }
            if model.num_cr_points > 0 || model.chroma_scaling_from_luma {
                let orig = v.get(index);
                let merged = chroma_merged_sample(
                    average_luma,
                    orig,
                    model.cr_luma_mult,
                    model.cr_mult,
                    model.cr_offset,
                    model.chroma_scaling_from_luma,
                    bit_depth,
                    sample_max,
                );
                let delta = round2(
                    scale_lut(&scaling[2], merged, bit_depth) * noise.planes[2][index],
                    scaling_shift,
                );
                v.set(index, (orig + delta).clamp(min_value, max_chroma) as u16)?;
            }
        }
    }
    Ok(())
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

fn scale_lut(lut: &[i32; 256], index: i32, bit_depth: u8) -> i32 {
    let shift = bit_depth - 8;
    let x = (index >> shift) as usize;
    if shift == 0 || x == 255 {
        return lut[x];
    }
    let rem = index - ((x as i32) << shift);
    lut[x] + round2((lut[x + 1] - lut[x]) * rem, shift)
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
