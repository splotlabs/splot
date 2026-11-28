// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Nearest-neighbor placement and alpha mapping for Annex D.4 and D.5.

use super::*;
use splot_core::headers::atlas_segment::AtlasMultistreamSegment;
use splot_recon::{AtlasAlphaRow, AtlasSample};

pub(super) trait CanvasSample: AtlasSample {
    fn texture(plane: PictureSamples<'_>) -> Result<PlaneRef<'_, Self>>;
    fn output(plane: PlaneRef<'_, Self>) -> PictureSamples<'_>;
}
impl CanvasSample for u8 {
    fn texture(plane: PictureSamples<'_>) -> Result<PlaneRef<'_, Self>> {
        match plane {
            PictureSamples::Eight(p) => Ok(p),
            PictureSamples::Ten(_) => Err(AtlasCompositionError::IncompatibleFormat),
        }
    }
    fn output(plane: PlaneRef<'_, Self>) -> PictureSamples<'_> {
        PictureSamples::Eight(plane)
    }
}
impl CanvasSample for u16 {
    fn texture(plane: PictureSamples<'_>) -> Result<PlaneRef<'_, Self>> {
        match plane {
            PictureSamples::Ten(p) => Ok(p),
            PictureSamples::Eight(_) => Err(AtlasCompositionError::IncompatibleFormat),
        }
    }
    fn output(plane: PlaneRef<'_, Self>) -> PictureSamples<'_> {
        PictureSamples::Ten(plane)
    }
}

pub(super) fn background(rgb: Option<(u8, u8, u8)>, depth: BitDepth) -> [u16; 3] {
    let (r, g, b) = rgb.unwrap_or((16, 16, 16));
    let (r, g, b) = (i64::from(r), i64::from(g), i64::from(b));
    let y = 2126 * r + 7152 * g + 722 * b;
    let scale = 1 << (depth.bits() - 8);
    let round = |n: i64, d: i64| (n * scale + d / 2).div_euclid(d);
    let cb = 128 * scale + round((10_000 * b - y) * 224, 2 * 9278 * 219);
    let cr = 128 * scale + round((10_000 * r - y) * 224, 2 * 7874 * 219);
    [
        round(y, 10_000).clamp(16 * scale, 235 * scale) as u16,
        cb.clamp(16 * scale, 240 * scale) as u16,
        cr.clamp(16 * scale, 240 * scale) as u16,
    ]
}

pub(super) fn canvas<'a, T: CanvasSample>(
    backend: SimdBackend,
    storage: &'a mut [T],
    alpha_scratch: &mut Storage,
    info: &AtlasMultistreamInfo,
    frames: &[Option<&Picture>; 31],
    geometry: Geometry,
) -> Result<[PictureSamples<'a>; 3]> {
    let (canvas, scratch) = storage.split_at_mut(geometry.samples);
    let y_len = geometry.size[0] as usize * geometry.size[1] as usize;
    let uv_len = geometry.chroma[0] as usize * geometry.chroma[1] as usize;
    let (y, uv) = canvas.split_at_mut(y_len);
    let (u, v) = uv.split_at_mut(uv_len);
    let mut planes = [y, u, v];
    let background = background(info.background, geometry.depth);
    for (data, color) in planes.iter_mut().zip(background) {
        data.fill(T::try_from_u16(color)?);
    }
    let mut alpha = None;
    for segment in &info.segments {
        let picture = frames
            .get(usize::from(segment.input_stream_id))
            .copied()
            .flatten()
            .ok_or(AtlasCompositionError::MissingPicture(
                segment.input_stream_id,
            ))?;
        if segment.alpha_segment_flag {
            alpha = Some((
                segment,
                picture
                    .plane(PlaneId::Y)
                    .ok_or(AtlasCompositionError::IncompatibleFormat)?,
            ));
            continue;
        }
        for (&id, out) in [PlaneId::Y, PlaneId::U, PlaneId::V]
            .iter()
            .zip(planes.iter_mut())
        {
            let source = picture
                .plane(id)
                .ok_or(AtlasCompositionError::IncompatibleFormat)?;
            paint(
                backend,
                out,
                scratch,
                alpha_scratch,
                geometry,
                id,
                segment,
                T::texture(source)?,
                alpha,
            )?;
        }
        alpha = None;
    }
    let [y, u, v] = planes;
    let view = |samples: &'a [T], id| -> Result<PictureSamples<'a>> {
        let [width, height] = geometry.plane_size(id).map(|x| x as usize);
        Ok(T::output(PlaneRef::new(
            samples,
            width,
            PlaneRect::new(0, 0, width, height)?,
        )?))
    };
    Ok([
        view(y, PlaneId::Y)?,
        view(u, PlaneId::U)?,
        view(v, PlaneId::V)?,
    ])
}

fn row<T: splot_recon::ReconSample>(
    plane: PlaneRef<'_, T>,
    out_y: u32,
    target_height: u32,
) -> &[T] {
    let rect = plane.visible_rect();
    let source_y = u64::from(out_y) * rect.height() as u64 / u64::from(target_height);
    let offset = (rect.y() + source_y as usize) * plane.stride_samples() + rect.x();
    &plane.samples()[offset..offset + rect.width()]
}

#[expect(
    clippy::too_many_arguments,
    reason = "the plane, scratch, and two independent segment rectangles define Annex D.5"
)]
pub(super) fn paint<T: AtlasSample>(
    backend: SimdBackend,
    canvas: &mut [T],
    scratch: &mut [T],
    alpha_scratch: &mut Storage,
    geometry: Geometry,
    plane: PlaneId,
    texture: &AtlasMultistreamSegment,
    source: PlaneRef<'_, T>,
    alpha: Option<(&AtlasMultistreamSegment, PictureSamples<'_>)>,
) -> Result<()> {
    let sx = if plane == PlaneId::Y {
        0
    } else {
        geometry.format.subsampling_x()
    };
    let sy = if plane == PlaneId::Y {
        0
    } else {
        geometry.format.subsampling_y()
    };
    let width = texture
        .width
        .min(geometry.size[0].saturating_sub(texture.top_left_pos_x))
        >> sx;
    let height = texture
        .height
        .min(geometry.size[1].saturating_sub(texture.top_left_pos_y))
        >> sy;
    if width == 0 || height == 0 {
        return Ok(());
    }
    let full_width = texture.width >> sx;
    let full_height = texture.height >> sy;
    let x0 = texture.top_left_pos_x >> sx;
    let y0 = texture.top_left_pos_y >> sy;
    let stride = geometry.plane_size(plane)[0] as usize;
    for y in 0..height {
        let offset = (y + y0) as usize * stride + x0 as usize;
        let dest = &mut canvas[offset..offset + width as usize];
        let src = row(source, y, full_height);
        let overlap = alpha.and_then(|(mask, samples)| {
            let global_y = texture.top_left_pos_y + (y << sy);
            let ay = global_y.checked_sub(mask.top_left_pos_y)?;
            if ay >= mask.height {
                return None;
            }
            let begin = mask
                .top_left_pos_x
                .saturating_sub(texture.top_left_pos_x)
                .div_ceil(1 << sx)
                .min(width);
            let end = (u64::from(mask.top_left_pos_x) + u64::from(mask.width))
                .saturating_sub(u64::from(texture.top_left_pos_x))
                .div_ceil(1 << sx)
                .min(u64::from(width)) as u32;
            if begin >= end {
                return None;
            }
            let ax = texture.top_left_pos_x + (begin << sx) - mask.top_left_pos_x;
            Some((mask, samples, begin, end, ax, ay))
        });
        if let Some((mask, samples, begin, end, ax, ay)) = overlap {
            let start = begin as usize;
            let length = (end - begin) as usize;
            let (before, tail) = dest.split_at_mut(start);
            let (inside, after) = tail.split_at_mut(length);
            T::nearest_atlas_row(backend, before, src, full_width, 0, false)?;
            T::nearest_atlas_row(backend, after, src, full_width, end, false)?;
            let texture_row = &mut scratch[..length];
            T::nearest_atlas_row(backend, texture_row, src, full_width, begin, false)?;
            let weights = match samples {
                PictureSamples::Eight(p) => {
                    let buf = alpha_scratch.eight(length)?;
                    u8::nearest_atlas_row(
                        backend,
                        buf,
                        row(p, ay, mask.height),
                        mask.width,
                        ax,
                        sx != 0,
                    )?;
                    AtlasAlphaRow::Eight(buf)
                }
                PictureSamples::Ten(p) => {
                    let buf = alpha_scratch.ten(length)?;
                    u16::nearest_atlas_row(
                        backend,
                        buf,
                        row(p, ay, mask.height),
                        mask.width,
                        ax,
                        sx != 0,
                    )?;
                    AtlasAlphaRow::Ten(buf)
                }
            };
            T::blend_atlas_row(backend, inside, texture_row, weights)?;
        } else {
            T::nearest_atlas_row(backend, dest, src, full_width, 0, false)?;
        }
    }
    Ok(())
}
