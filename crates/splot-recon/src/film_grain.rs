// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7 display film-grain synthesis.
//!
//! The § 7.21.7.5 noise image is never materialized: each noise stripe is
//! built, blended with its predecessor's overlap rows and added to the output
//! rows it covers before the next stripe replaces the older buffer. Output
//! rows start as copies of the source rows and take their noise in place;
//! chroma reads the untouched source luma, as § 7.21.7.5 adds luma noise last.
//!
//! Every plane resolves `scale_lut` once per frame into a table over all
//! sample values, so a row's scaling is a table read per sample. At 8 bits
//! the scaled noise fits `i16` lanes: a table value is at most 255 and the
//! grain lies in `GrainMin..=GrainMax`, so `|scale * noise| <= 32640`.

use fearless_simd::{Simd, i16x16, i16x32, i32x8, prelude::*, u8x16, u8x32, u16x16};

use splot_core::headers::film_grain::{FilmGrainModel, FilmGrainScalingPoint};

use crate::{
    DecodedFrame, DecodedFrameInfo, FramePlaneSamples, FramePlanes, Plane, PlaneId, PlaneRect,
    PlaneSize, ReconError, ReconSample, Result,
};

mod template;
#[cfg(target_arch = "x86_64")]
mod wide_luma;
use template::{GrainTemplates, generate_grain};
#[cfg(target_arch = "x86_64")]
pub(crate) use wide_luma::add_luma_noise;

const LUMA_GRAIN_WIDTH: usize = 82;
const LUMA_GRAIN_HEIGHT: usize = 73;
const LUMA_GRAIN_BASE: usize = 3;

struct ActiveFilmGrain<'a> {
    model: &'a FilmGrainModel,
    grain_seed: u16,
}

type PackedLumaNoise = fn(&mut [u8], &[u8; 256], &[i16], NoiseParams);

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
    apply_film_grain_into(frame, model, grain_seed, FramePlaneSamples::default())
}

/// [`apply_film_grain`] into the plane buffers of `spare`, so the grained frame
/// reuses a retired frame's storage. A buffer too small for its plane is
/// replaced from the pool `spare` names, or newly allocated without one.
///
/// # Errors
///
/// See [`apply_film_grain`].
pub fn apply_film_grain_into<T: ReconSample>(
    frame: &DecodedFrame<T>,
    model: &FilmGrainModel,
    grain_seed: u16,
    spare: FramePlaneSamples<T>,
) -> Result<DecodedFrame<T>> {
    baseline_simd!(simd => apply_film_grain_simd(
        simd,
        frame,
        model,
        grain_seed,
        spare,
        |row, scale, noise, params| add_noise_row::<_, _, false>(simd, row, scale, noise, params),
        None::<PackedLumaNoise>,
    ))
}

/// [`apply_film_grain_into`] using the caller's startup-selected static
/// SIMD backend `backend`.
///
/// # Errors
///
/// See [`apply_film_grain`].
pub fn apply_film_grain_into_with<T: ReconSample>(
    backend: splot_core::simd::SimdBackend,
    kernels: crate::KernelSelection,
    frame: &DecodedFrame<T>,
    model: &FilmGrainModel,
    grain_seed: u16,
    spare: FramePlaneSamples<T>,
) -> Result<DecodedFrame<T>> {
    #[cfg(target_arch = "x86_64")]
    let wide = kernels.wide().filter(|_| frame.bit_depth().bits() <= 10);
    #[cfg(not(target_arch = "x86_64"))]
    let _ = kernels;
    splot_core::with_simd!(backend, simd => {
        let add_noise = |row: &mut [T], scale: &[i16], noise: &[i16], params| {
            #[cfg(target_arch = "x86_64")]
            if let Some(wide) = wide
                && row.len() >= 32
            {
                if let Some(row) = T::u8_slice_mut(row) {
                    return (wide.grain_u8)(row, scale, noise, params);
                }
                if let Some(row) = T::u16_slice_mut(row) {
                    return (wide.grain_u16)(row, scale, noise, params);
                }
            }
            add_noise_row::<_, _, false>(simd, row, scale, noise, params)
        };
        #[cfg(target_arch = "x86_64")]
        let wide_luma = wide
            .filter(|_| frame.bit_depth().bits() == 8)
            .map(|wide| wide.luma_grain);
        #[cfg(not(target_arch = "x86_64"))]
        let wide_luma = None::<PackedLumaNoise>;
        apply_film_grain_simd(simd, frame, model, grain_seed, spare, add_noise, wide_luma)
    })
}

fn apply_film_grain_simd<S: Simd, T: ReconSample>(
    simd: S,
    frame: &DecodedFrame<T>,
    model: &FilmGrainModel,
    grain_seed: u16,
    mut spare: FramePlaneSamples<T>,
    add_noise: impl Fn(&mut [T], &[i16], &[i16], NoiseParams) -> Result<()>,
    wide_luma: Option<impl Fn(&mut [u8], &[u8; 256], &[i16], NoiseParams)>,
) -> Result<DecodedFrame<T>> {
    let grain = ActiveFilmGrain { model, grain_seed };
    let visible = frame.visible_luma_rect();
    let width = visible.width();
    let height = visible.height();
    let pixel_format = frame.pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let geometry = GrainGeometry {
        widths: plane_axis_lengths(width, sub_x),
        heights: plane_axis_lengths(height, sub_y),
        sub_x,
        sub_y,
        bit_depth: frame.bit_depth().bits(),
    };
    let planes = [Some(frame.y()), frame.u(), frame.v()];
    let sources: Vec<Vec<&[T]>> = planes
        .into_iter()
        .take(pixel_format.num_planes())
        .map_while(|plane| plane)
        .enumerate()
        .map(|(index, plane)| {
            plane
                .visible_rows()
                .map(|row| row.get(..geometry.widths[index]).unwrap_or(&[]))
                .collect()
        })
        .collect();
    for (index, rows) in sources.iter().enumerate() {
        if rows.len() != geometry.heights[index]
            || rows.iter().any(|row| row.len() != geometry.widths[index])
        {
            return Err(ReconError::BufferLengthMismatch {
                expected: geometry.widths[index],
                actual: rows.first().map_or(0, |row| row.len()),
            });
        }
    }
    let mut outputs = synthesize(
        simd, &sources, &geometry, &grain, &mut spare, add_noise, wide_luma,
    )?
    .into_iter();

    let info = DecodedFrameInfo::new(
        frame.output_index(),
        frame.bit_depth(),
        pixel_format,
        PlaneSize::new(width, height)?,
        PlaneRect::new(0, 0, width, height)?,
    )?;
    let y = outputs.next().ok_or(ReconError::BufferLengthMismatch {
        expected: width,
        actual: 0,
    })?;
    let y = plane_from_visible(width, height, y)?;
    let chroma_size = pixel_format.chroma_size(visible.size())?;
    let mut chroma = || -> Result<Option<Plane<T>>> {
        match (chroma_size, outputs.next()) {
            (Some(size), Some(samples)) => Ok(Some(plane_from_visible(
                size.width(),
                size.height(),
                samples,
            )?)),
            _ => Ok(None),
        }
    };
    let u = chroma()?;
    let v = chroma()?;

    DecodedFrame::try_new(info, FramePlanes::new(y, u, v))
}

struct GrainGeometry {
    widths: [usize; 3],
    heights: [usize; 3],
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
}

struct GrainDestination<T: ReconSample> {
    samples: crate::PlaneBuf<T>,
    stride: usize,
}

impl<T: ReconSample> GrainDestination<T> {
    fn with_rows(
        width: usize,
        height: usize,
        mut samples: crate::PlaneBuf<T>,
        pool: Option<&crate::PlanePool>,
    ) -> Self {
        let stride = crate::plane_buf::plane_stride::<T>(width).unwrap_or(width);
        let len = stride * height;
        if let Some(pool) = pool
            && samples.capacity() < len
        {
            pool.recycle(core::mem::take(&mut samples));
            samples = pool.take(len);
        }
        samples.clear();
        samples.reserve_exact(len);
        Self { samples, stride }
    }

    /// Appends a copy of `source` as the next row and returns it for its noise.
    fn push_row(&mut self, source: &[T]) -> &mut [T] {
        let start = self.samples.len();
        self.samples.append_samples(source); // splot-copy-ok: a film-grain output row starts as its source row
        self.samples.resize(start + self.stride, T::default());
        &mut self.samples[start..start + source.len()]
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

/// How one plane takes its noise; planes without one pass through unchanged.
struct PlaneNoise {
    lut: ScaleLut,
    max: i32,
    /// § 7.21.7.5 `merged` weights and offset of a chroma plane that does not
    /// scale from luma.
    mix: Option<ChromaMix>,
}

#[derive(Clone, Copy)]
struct ChromaMix {
    luma_mult: i16,
    mult: i16,
    offset: i32,
}

/// Per-row working storage shared by every plane.
struct RowScratch {
    noise: Vec<i16>,
    index: Vec<i16>,
    scale: Vec<i16>,
}

fn synthesize<S: Simd, T: ReconSample>(
    simd: S,
    sources: &[Vec<&[T]>],
    geometry: &GrainGeometry,
    grain: &ActiveFilmGrain<'_>,
    spare: &mut FramePlaneSamples<T>,
    add_noise: impl Fn(&mut [T], &[i16], &[i16], NoiseParams) -> Result<()>,
    wide_luma: Option<impl Fn(&mut [u8], &[u8; 256], &[i16], NoiseParams)>,
) -> Result<Vec<GrainDestination<T>>> {
    let model = grain.model;
    let bit_depth = geometry.bit_depth;
    let num_planes = sources.len();
    let pool = spare.pool.clone();
    let mut outputs: Vec<GrainDestination<T>> = [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .take(num_planes)
        .map(|plane| {
            let index = plane.index();
            let samples = spare.take(plane);
            GrainDestination::with_rows(
                geometry.widths[index],
                geometry.heights[index],
                samples,
                pool.as_deref(),
            )
        })
        .collect();
    let (min_value, max_luma, max_chroma) = output_ranges(grain, bit_depth);
    let noise: Vec<Option<PlaneNoise>> = (0..num_planes)
        .map(|plane| plane_noise(model, plane, bit_depth, max_luma, max_chroma))
        .collect();
    for (plane, output) in outputs.iter_mut().enumerate() {
        if noise[plane].is_none() {
            for row in &sources[plane] {
                output.push_row(row);
            }
        }
    }
    if noise.iter().all(Option::is_none) {
        return Ok(outputs);
    }

    let grain_range = (-(1_i32 << (bit_depth - 1)), (1_i32 << (bit_depth - 1)) - 1);
    let templates = generate_grain(
        simd,
        grain,
        geometry.sub_x,
        geometry.sub_y,
        bit_depth,
        grain_range,
    )?;
    let luma_size = if model.film_grain_block_size { 32 } else { 16 };
    let width = geometry.widths[0];
    let packed_luma = wide_luma.filter(|_| width >= 64).and_then(|apply| {
        let table = noise.first()?.as_ref()?.lut.exact.as_deref()?;
        Some((apply, core::array::from_fn(|index| table[index] as u8)))
    });
    let widths = &geometry.widths[..num_planes];
    let mut stripes = [
        Stripe::new(widths, luma_size, geometry.sub_x, geometry.sub_y),
        Stripe::new(widths, luma_size, geometry.sub_x, geometry.sub_y),
    ];
    let mut scratch = RowScratch {
        noise: vec![0; width],
        index: vec![0; width],
        scale: vec![0; width],
    };
    let active: [bool; 3] =
        core::array::from_fn(|plane| noise.get(plane).is_some_and(Option::is_some));
    for luma_num in 0..stripe_count(geometry.heights[0], luma_size) {
        let current = luma_num & 1;
        fill_stripe(
            &mut stripes[current],
            luma_num,
            geometry,
            active,
            luma_size,
            grain,
            &templates,
            grain_range,
        );
        for (plane, plane_noise) in noise.iter().enumerate() {
            let Some(plane_noise) = plane_noise else {
                continue;
            };
            let plane_sub_y = if plane > 0 { geometry.sub_y } else { 0 };
            let stripe_shift = 4 + usize::from(model.film_grain_block_size) - plane_sub_y;
            let blend_rows = if plane_sub_y == 0 { 2 } else { 1 };
            let plane_width = geometry.widths[plane];
            let stripe_width = stripes[current].widths[plane];
            let params = NoiseParams {
                shift: model.grain_scaling_minus_8 + 8,
                min: min_value,
                max: plane_noise.max,
                narrow: bit_depth == 8,
            };
            for stripe_row in 0..(1 << stripe_shift) {
                let row = (luma_num << stripe_shift) + stripe_row;
                if row >= geometry.heights[plane] {
                    break;
                }
                let fresh = stripe_row_slice(
                    &stripes[current].planes[plane],
                    stripe_width,
                    stripe_row,
                    plane_width,
                );
                let noise_row: &[i16] =
                    if model.overlap_flag && luma_num > 0 && stripe_row < blend_rows {
                        let old = stripe_row_slice(
                            &stripes[current ^ 1].planes[plane],
                            stripe_width,
                            stripe_row + (luma_size >> plane_sub_y),
                            plane_width,
                        );
                        let weights = match (plane_sub_y, stripe_row) {
                            (0, 0) => [27, 17],
                            (0, _) => [17, 27],
                            _ => [23, 22],
                        };
                        let blended = &mut scratch.noise[..plane_width];
                        blend_noise_row(simd, blended, old, fresh, weights, grain_range);
                        blended
                    } else {
                        fresh
                    };
                let output = outputs[plane].push_row(sources[plane][row]);
                if plane == 0
                    && let Some((apply, table)) = &packed_luma
                    && let Some(output) = T::u8_slice_mut(output)
                {
                    apply(output, table, noise_row, params);
                    continue;
                }
                let luma = sources[0][row << if plane > 0 { geometry.sub_y } else { 0 }];
                let lut = &plane_noise.lut;
                let Some(table) = lut.exact.as_deref() else {
                    let mut merged = |col: usize, sample: T| {
                        if plane == 0 {
                            i32::from(sample.to_u16())
                        } else {
                            merged_index(
                                sample,
                                luma,
                                col,
                                geometry.sub_x,
                                plane_noise.mix,
                                bit_depth,
                            )
                        }
                    };
                    noisy_row_scalar(output, noise_row, &mut merged, lut, params)?;
                    continue;
                };
                let scale = &mut scratch.scale[..plane_width];
                if plane == 0 {
                    luma_scale_row(scale, output, table);
                } else {
                    let index = &mut scratch.index[..plane_width];
                    chroma_index_row(
                        simd,
                        index,
                        output,
                        luma,
                        geometry.sub_x,
                        plane_noise.mix,
                        bit_depth,
                    );
                    for (scale, &index) in scale.iter_mut().zip(index.iter()) {
                        *scale = table[usize::from(index as u16).min(SCALE_TABLE_LEN - 1)];
                    }
                }
                add_noise(output, scale, noise_row, params)?;
            }
        }
    }
    Ok(outputs)
}

fn plane_noise(
    model: &FilmGrainModel,
    plane: usize,
    bit_depth: u8,
    max_luma: i32,
    max_chroma: i32,
) -> Option<PlaneNoise> {
    let active = match plane {
        0 => model.num_y_points > 0,
        1 => model.num_cb_points > 0 || model.chroma_scaling_from_luma,
        _ => model.num_cr_points > 0 || model.chroma_scaling_from_luma,
    };
    if !active {
        return None;
    }
    let mut coarse = [0; SCALING_LUT_LEN];
    init_scaling_lut(scaling_points_for_plane(model, plane), &mut coarse);
    let mix = (plane > 0 && !model.chroma_scaling_from_luma).then(|| {
        let (luma_mult, mult, offset) = if plane == 1 {
            (model.cb_luma_mult, model.cb_mult, model.cb_offset)
        } else {
            (model.cr_luma_mult, model.cr_mult, model.cr_offset)
        };
        ChromaMix {
            luma_mult: i16::from(luma_mult.unwrap_or(128)) - 128,
            mult: i16::from(mult.unwrap_or(128)) - 128,
            offset: (i32::from(offset.unwrap_or(256)) - 256) << (bit_depth - 8),
        }
    });
    Some(PlaneNoise {
        lut: ScaleLut::new(&coarse, bit_depth),
        max: if plane == 0 { max_luma } else { max_chroma },
        mix,
    })
}

/// Table entries of [`ScaleLut`]: every 10-bit sample value.
const SCALE_TABLE_LEN: usize = 1024;

/// A plane's `scale_lut` resolved for every sample value.
struct ScaleLut {
    coarse: [i32; SCALING_LUT_LEN],
    /// `scale_lut(index)` for every index of a bit depth up to 10, or `None`
    /// when a value falls outside `0..=255`, which 8-bit `point_*_scaling`
    /// syntax rules out; such planes take [`noisy_row_scalar`]. From
    /// `255 << (BitDepth - 8)` on, `scale_lut` returns `ScalingLut[255]`, so
    /// the entries past the bit depth hold that and a clamped index is exact.
    exact: Option<Box<[i16; SCALE_TABLE_LEN]>>,
    bit_depth: u8,
}

impl ScaleLut {
    fn new(coarse: &[i32; SCALING_LUT_LEN], bit_depth: u8) -> Self {
        let mut exact = Box::new([0; SCALE_TABLE_LEN]);
        let fits = bit_depth <= 10
            && exact.iter_mut().enumerate().all(|(index, slot)| {
                let index = (index as i32).min((1 << bit_depth) - 1);
                i16::try_from(scale_lut(coarse, index, bit_depth))
                    .ok()
                    .filter(|value| (0..=255).contains(value))
                    .map(|value| *slot = value)
                    .is_some()
            });
        Self {
            coarse: *coarse,
            exact: fits.then_some(exact),
            bit_depth,
        }
    }

    fn scale(&self, index: i32) -> i32 {
        scale_lut(&self.coarse, index, self.bit_depth)
    }
}

fn luma_scale_row<T: ReconSample>(scale: &mut [i16], row: &[T], table: &[i16; SCALE_TABLE_LEN]) {
    if let Some(row) = T::u8_slice(row) {
        for (scale, &sample) in scale.iter_mut().zip(row) {
            *scale = table[usize::from(sample)];
        }
        return;
    }
    for (scale, sample) in scale.iter_mut().zip(row) {
        *scale = table[usize::from(sample.to_u16()).min(SCALE_TABLE_LEN - 1)];
    }
}

/// § 7.21.7.5 noise for a plane whose scaling leaves `0..=255`, one sample at
/// a time; `merged(col, orig)` is the sample's scaling index.
fn noisy_row_scalar<T: ReconSample>(
    row: &mut [T],
    noise: &[i16],
    merged: &mut impl FnMut(usize, T) -> i32,
    lut: &ScaleLut,
    params: NoiseParams,
) -> Result<()> {
    for (col, (sample, &noise)) in row.iter_mut().zip(noise).enumerate() {
        let orig = i32::from(sample.to_u16());
        let scale = lut.scale(merged(col, *sample));
        let delta = round2(scale.wrapping_mul(i32::from(noise)), params.shift);
        *sample = T::try_from_u16((orig + delta).clamp(params.min, params.max) as u16)?;
    }
    Ok(())
}

/// § 7.21.7.5 `merged` of chroma sample `orig` at column `col`.
fn merged_index<T: ReconSample>(
    orig: T,
    luma: &[T],
    col: usize,
    sub_x: usize,
    mix: Option<ChromaMix>,
    bit_depth: u8,
) -> i32 {
    let luma_x = col << sub_x;
    let sample = |x: usize| luma.get(x).map_or(0, |sample| i32::from(sample.to_u16()));
    let average = if sub_x == 1 {
        let next = (luma_x + 1).min(luma.len().saturating_sub(1));
        round2(sample(luma_x) + sample(next), 1)
    } else {
        sample(luma_x)
    };
    mix.map_or(average, |mix| {
        let combined =
            average * i32::from(mix.luma_mult) + i32::from(orig.to_u16()) * i32::from(mix.mult);
        ((combined >> 6) + mix.offset).clamp(0, (1_i32 << bit_depth) - 1)
    })
}

/// § 7.21.7.5 `merged` scaling index of each chroma sample of a row.
fn chroma_index_row<S: Simd, T: ReconSample>(
    simd: S,
    index: &mut [i16],
    chroma: &[T],
    luma: &[T],
    sub_x: usize,
    mix: Option<ChromaMix>,
    bit_depth: u8,
) {
    let max_index = (1_i32 << bit_depth) - 1;
    let done = if let (Some(chroma), Some(luma)) = (T::u8_slice(chroma), T::u8_slice(luma)) {
        chroma_index_lanes(simd, index, chroma, luma, sub_x, mix, max_index)
    } else if let (Some(chroma), Some(luma)) = (T::u16_slice(chroma), T::u16_slice(luma)) {
        chroma_index_lanes(simd, index, chroma, luma, sub_x, mix, max_index)
    } else {
        0
    };
    for (col, (slot, &orig)) in index.iter_mut().zip(chroma).enumerate().skip(done) {
        *slot = merged_index(orig, luma, col, sub_x, mix, bit_depth) as i16;
    }
}

/// Samples a lane kernel loads and stores as `i16` lanes.
trait GrainLanes: Copy {
    fn load16<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S>;
    fn load32<S: Simd>(simd: S, samples: &[Self; 32]) -> (i16x16<S>, i16x16<S>);
    /// Stores lanes already clamped to the sample range.
    fn store32<S: Simd>(low: i16x16<S>, high: i16x16<S>, samples: &mut [Self; 32]);
}

impl GrainLanes for u8 {
    #[inline]
    fn load16<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        let (low, high) = u8x16::load_array_ref(simd, samples).widen();
        simd.combine_u16x8(low, high).bitcast()
    }

    #[inline]
    fn load32<S: Simd>(simd: S, samples: &[Self; 32]) -> (i16x16<S>, i16x16<S>) {
        let (low, high) = u8x32::load_array_ref(simd, samples).widen();
        (low.bitcast(), high.bitcast())
    }

    #[inline]
    fn store32<S: Simd>(low: i16x16<S>, high: i16x16<S>, samples: &mut [Self; 32]) {
        low.bitcast::<u16x16<S>>()
            .narrow(high.bitcast::<u16x16<S>>())
            .store_array(samples);
    }
}

impl GrainLanes for u16 {
    #[inline]
    fn load16<S: Simd>(simd: S, samples: &[Self; 16]) -> i16x16<S> {
        u16x16::load_array_ref(simd, samples).bitcast()
    }

    #[inline]
    fn load32<S: Simd>(simd: S, samples: &[Self; 32]) -> (i16x16<S>, i16x16<S>) {
        let (halves, _) = samples.as_chunks::<16>();
        (
            Self::load16(simd, &halves[0]),
            Self::load16(simd, &halves[1]),
        )
    }

    #[inline]
    fn store32<S: Simd>(low: i16x16<S>, high: i16x16<S>, samples: &mut [Self; 32]) {
        let (halves, _) = samples.as_chunks_mut::<16>();
        low.bitcast::<u16x16<S>>().store_array(&mut halves[0]);
        high.bitcast::<u16x16<S>>().store_array(&mut halves[1]);
    }
}

/// Lane-group [`chroma_index_row`]; returns the columns done. Indices are
/// at most `max_index`, below `2^15`.
#[inline]
fn chroma_index_lanes<S: Simd, L: GrainLanes>(
    simd: S,
    index: &mut [i16],
    chroma: &[L],
    luma: &[L],
    sub_x: usize,
    mix: Option<ChromaMix>,
    max_index: i32,
) -> usize {
    let weights = mix.map(|mix| {
        let lanes = i16x16::from_fn(simd, |lane| {
            if lane % 2 == 0 {
                mix.luma_mult
            } else {
                mix.mult
            }
        });
        (lanes, mix.offset)
    });
    let (zero, max) = (i32x8::splat(simd, 0), i32x8::splat(simd, max_index));
    let mut col = 0;
    while let (Some(slot), Some(orig)) = (
        index
            .get_mut(col..)
            .and_then(<[i16]>::first_chunk_mut::<16>),
        chroma.get(col..).and_then(<[L]>::first_chunk::<16>),
    ) {
        let average = if sub_x == 1 {
            let Some(pairs) = luma.get(2 * col..).and_then(<[L]>::first_chunk::<32>) else {
                break;
            };
            let (low, high) = L::load32(simd, pairs);
            let (even, odd) = low.deinterleave(high);
            (even + odd + 1) >> 1
        } else {
            let Some(luma) = luma.get(col..).and_then(<[L]>::first_chunk::<16>) else {
                break;
            };
            L::load16(simd, luma)
        };
        let merged = weights.map_or(average, |(weights, offset)| {
            let (low, high) = average.interleave(L::load16(simd, orig));
            let merge = |pairs: i16x16<S>| {
                ((pairs.pairwise_dot_product(weights) >> 6) + offset).clamp(zero, max)
            };
            merge(low).narrow(merge(high))
        });
        merged.store_array(slot);
        col += 16;
    }
    col
}

#[derive(Clone, Copy)]
pub(crate) struct NoiseParams {
    shift: u8,
    min: i32,
    max: i32,
    /// The bit depth is 8, so scaled noise is exact in `i16` lanes.
    narrow: bool,
}

/// Adds the scaled noise to one output row in place.
#[expect(
    clippy::inline_always,
    reason = "keep the row in the selected SIMD scope"
)]
#[inline(always)]
pub(crate) fn add_noise_row<S: Simd, T: ReconSample, const WIDE: bool>(
    simd: S,
    row: &mut [T],
    scale: &[i16],
    noise: &[i16],
    params: NoiseParams,
) -> Result<()> {
    let done = if i32::from(T::MAX_VALUE) < params.max {
        0
    } else if let Some(row) = T::u8_slice_mut(row) {
        add_noise_lanes::<_, _, WIDE>(simd, row, scale, noise, params)
    } else if let Some(row) = T::u16_slice_mut(row) {
        add_noise_lanes::<_, _, WIDE>(simd, row, scale, noise, params)
    } else {
        0
    };
    for ((sample, &scale), &noise) in row.iter_mut().zip(scale).zip(noise).skip(done) {
        let orig = i32::from(sample.to_u16());
        let delta = round2(i32::from(scale) * i32::from(noise), params.shift);
        *sample = T::try_from_u16((orig + delta).clamp(params.min, params.max) as u16)?;
    }
    Ok(())
}

/// Adds grain using the same rounding and saturation for a half or full group.
/// The 8-bit path shifts the product before rounding so the sum stays in i16.
#[expect(clippy::inline_always, reason = "shared vector noise arithmetic")]
#[inline(always)]
fn add_noise_vector<S, V>(simd: S, orig: V, scale: V, noise: V, params: NoiseParams) -> V
where
    S: Simd,
    V: SimdIntWiden<S, Element = i16, Widened: SimdInt<S, Element = i32>>,
{
    let shift = u32::from(params.shift);
    if params.narrow {
        let delta = (((scale * noise) >> (shift - 1)) + V::splat(simd, 1)) >> 1;
        return (orig + delta).clamp(
            V::splat(simd, params.min as i16),
            V::splat(simd, params.max as i16),
        );
    }
    let (low, high) = scale.widening_mul(noise);
    let (orig_low, orig_high) = orig.widen();
    let half = V::Widened::splat(simd, (1 << shift) >> 1);
    let (min, max) = (
        V::Widened::splat(simd, params.min),
        V::Widened::splat(simd, params.max),
    );
    let add = |product: V::Widened, orig: V::Widened| {
        (orig + ((product + half) >> shift)).clamp(min, max)
    };
    add(low, orig_low).narrow(add(high, orig_high))
}

/// Lane-group [`add_noise_row`]; returns the columns done. The wide backend
/// processes all 32 samples in the group together.
#[expect(
    clippy::inline_always,
    reason = "keep the row in the selected SIMD scope"
)]
#[inline(always)]
fn add_noise_lanes<S: Simd, L: GrainLanes, const WIDE: bool>(
    simd: S,
    row: &mut [L],
    scale: &[i16],
    noise: &[i16],
    params: NoiseParams,
) -> usize {
    let mut col = 0;
    while let (Some(samples), Some(scale), Some(noise)) = (
        row.get_mut(col..).and_then(<[L]>::first_chunk_mut::<32>),
        scale.get(col..).and_then(<[i16]>::first_chunk::<32>),
        noise.get(col..).and_then(<[i16]>::first_chunk::<32>),
    ) {
        let (orig_low, orig_high) = L::load32(simd, samples);
        let (low, high) = if WIDE {
            let orig = simd.combine_i16x16(orig_low, orig_high);
            let scale = i16x32::load_array_ref(simd, scale);
            let noise = i16x32::load_array_ref(simd, noise);
            simd.split_i16x32(add_noise_vector(simd, orig, scale, noise, params))
        } else {
            let split =
                |values: &[i16; 32]| simd.split_i16x32(i16x32::load_array_ref(simd, values));
            let (scale_low, scale_high) = split(scale);
            let (noise_low, noise_high) = split(noise);
            (
                add_noise_vector(simd, orig_low, scale_low, noise_low, params),
                add_noise_vector(simd, orig_high, scale_high, noise_high, params),
            )
        };
        L::store32(low, high, samples);
        col += 32;
    }
    col
}

/// The § 7.21.7.5 vertical overlap of two noise rows, exact in `i16` lanes
/// since `|grain| <= 512` and the weights sum to at most 45.
fn blend_noise_row<S: Simd>(
    simd: S,
    output: &mut [i16],
    old: &[i16],
    new: &[i16],
    weights: [i16; 2],
    (grain_min, grain_max): (i32, i32),
) {
    let range = [grain_min as i16, grain_max as i16];
    let done = blend_noise_lanes(simd, output, old, new, weights, range);
    for ((slot, &old), &new) in output.iter_mut().zip(old).zip(new).skip(done) {
        let sample = round2(
            i32::from(old) * i32::from(weights[0]) + i32::from(new) * i32::from(weights[1]),
            5,
        );
        *slot = sample.clamp(grain_min, grain_max) as i16;
    }
}

#[inline]
fn blend_noise_lanes<S: Simd>(
    simd: S,
    output: &mut [i16],
    old: &[i16],
    new: &[i16],
    [old_weight, new_weight]: [i16; 2],
    [grain_min, grain_max]: [i16; 2],
) -> usize {
    let (low, high) = (
        i16x16::splat(simd, grain_min),
        i16x16::splat(simd, grain_max),
    );
    let mut col = 0;
    while let (Some(slot), Some(old), Some(new)) = (
        output
            .get_mut(col..)
            .and_then(<[i16]>::first_chunk_mut::<16>),
        old.get(col..).and_then(<[i16]>::first_chunk::<16>),
        new.get(col..).and_then(<[i16]>::first_chunk::<16>),
    ) {
        let sum = i16x16::load_array_ref(simd, old) * old_weight
            + i16x16::load_array_ref(simd, new) * new_weight;
        ((sum + 16) >> 5).clamp(low, high).store_array(slot);
        col += 16;
    }
    col
}

const SCALING_LUT_LEN: usize = 257;

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

/// One § 7.21.7.5 `noiseStripe`: per plane `(lumaSize + 2) >> subY` rows of
/// `widths[plane]` samples.
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

fn stripe_row_slice(stripe: &[i16], stripe_width: usize, row: usize, width: usize) -> &[i16] {
    &stripe[row * stripe_width..row * stripe_width + width]
}

#[expect(clippy::too_many_arguments)]
fn fill_stripe(
    stripe: &mut Stripe,
    luma_num: usize,
    geometry: &GrainGeometry,
    active: [bool; 3],
    luma_size: usize,
    grain: &ActiveFilmGrain<'_>,
    templates: &GrainTemplates,
    grain_range: (i32, i32),
) {
    let mut rng = GrainRng::new(grain.grain_seed);
    let luma_rand = (luma_num * (luma_size >> 1)) >> 3;
    rng.register ^= (((luma_rand * 37 + 178) & 255) as u16) << 8;
    rng.register ^= ((luma_rand * 173 + 105) & 255) as u16;
    let step = luma_size >> 1;
    let half_width = geometry.widths[0].div_ceil(2);
    let block_size = usize::from(grain.model.film_grain_block_size);
    let mut block_x = 0usize;
    while block_x < half_width {
        let offset_y = (rng.get(9) * (3 - block_size)) >> 6;
        let _ = rng.get(1);
        let _ = rng.get(1);
        let _ = rng.get(1);
        let offset_x = (rng.get(9) * (3 - block_size)) >> 6;
        let _ = rng.get(1);
        let _ = rng.get(1);
        let _ = rng.get(1);
        for plane in (0..stripe.planes.len()).filter(|&plane| active[plane]) {
            fill_stripe_plane(
                stripe,
                plane,
                block_x,
                (offset_x, offset_y),
                geometry,
                luma_size,
                grain,
                templates,
                grain_range,
            );
        }
        block_x += step;
    }
}

#[expect(clippy::too_many_arguments)]
fn fill_stripe_plane(
    stripe: &mut Stripe,
    plane: usize,
    block_x: usize,
    (offset_x, offset_y): (usize, usize),
    geometry: &GrainGeometry,
    luma_size: usize,
    grain: &ActiveFilmGrain<'_>,
    templates: &GrainTemplates,
    (grain_min, grain_max): (i32, i32),
) {
    let plane_sub_x = if plane > 0 { geometry.sub_x } else { 0 };
    let plane_sub_y = if plane > 0 { geometry.sub_y } else { 0 };
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
    let block = GrainBlock {
        template,
        template_width,
        source: plane_offset_y * template_width + plane_offset_x,
        stripe_width: stripe.widths[plane],
        target: dst_col,
        rows: block_height,
        blend: match blended_cols {
            0 => &[],
            _ if plane_sub_x == 0 => &[[27, 17], [17, 27]],
            _ => &[[23, 22]],
        },
        range: (grain_min, grain_max),
    };
    let target = &mut stripe.planes[plane];
    match block_width {
        34 => block.copy::<34>(target),
        18 => block.copy::<18>(target),
        17 => block.copy::<17>(target),
        _ => block.copy::<9>(target),
    }
}

/// One grain block of a noise stripe: `rows` template rows from `source` to
/// stripe column `target`, the leading columns blended with what the
/// previous block left there.
struct GrainBlock<'a> {
    template: &'a [i16],
    template_width: usize,
    source: usize,
    stripe_width: usize,
    target: usize,
    rows: usize,
    /// `[old, new]` weights of the leading overlap columns.
    blend: &'static [[i32; 2]],
    range: (i32, i32),
}

impl GrainBlock<'_> {
    /// Copies the block with its § 7.21.7.5 width as a constant, so each row
    /// copy inlines.
    #[inline]
    fn copy<const WIDTH: usize>(&self, stripe: &mut [i16]) {
        let (Some(template), Some(stripe)) = (
            self.template.get(self.source..),
            stripe.get_mut(self.target..),
        ) else {
            return;
        };
        let rows = template
            .chunks(self.template_width)
            .zip(stripe.chunks_mut(self.stripe_width))
            .take(self.rows);
        for (source, target) in rows {
            let (Some(source), Some(target)) = (
                source.first_chunk::<WIDTH>(),
                target.first_chunk_mut::<WIDTH>(),
            ) else {
                return;
            };
            let old = [target[0], target[1]];
            *target = *source;
            for (col, [old_weight, new_weight]) in self.blend.iter().enumerate() {
                let sample = round2(
                    i32::from(old[col]) * old_weight + i32::from(source[col]) * new_weight,
                    5,
                );
                target[col] = sample.clamp(self.range.0, self.range.1) as i16;
            }
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
mod tests;
