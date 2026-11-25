// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Optional multistream presentation according to AV2 Annex D.

mod render;
#[cfg(test)]
mod tests;

use std::{io, sync::Arc};

use splot_core::headers::atlas_segment::{
    AtlasModeInfo, AtlasMultistreamInfo, AtlasMultistreamSegment,
};
use splot_core::headers::content_interpretation::{ColorPrimariesTriple, DerivedColorInfo};
use splot_core::simd::{SimdBackend, SimdScope};
use splot_core::types::GLOBAL_XLAYER_ID;
use splot_recon::{PlaneRect, ReconError};

use super::{ContentInterpretation, Picture, PictureSamples};
use crate::{
    AtlasLayout, BitDepth, DecodeLimitError, DecodeLimitName, DecodeLimitThreshold, DecodeLimits,
    PixelFormat, PlaneId, PlaneRef,
};

/// A presentation error leaves all decoded pictures available to the caller.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AtlasCompositionError {
    /// The atlas uses a mode other than 3 or 4.
    #[error("Select a multistream atlas with mode 3 or 4.")]
    UnsupportedMode,
    /// The segment geometry or alpha pairing is invalid.
    #[error("The atlas has invalid geometry, stream identifiers, or alpha pairing.")]
    InvalidLayout,
    /// The group lacks a required extended layer.
    #[error("Supply a picture for extended layer {0}.")]
    MissingPicture(u8),
    /// The group includes an unused or repeated extended layer.
    #[error("Extended layer {0} is unexpected or occurs twice in this group.")]
    UnexpectedPicture(u8),
    /// The picture belongs to a different layout or segment.
    #[error("Extended layer {0} has a different atlas association.")]
    StaleLayout(u8),
    /// The caller assigned a different presentation timestamp to this picture.
    #[error("Extended layer {0} has a different presentation timestamp.")]
    TimestampMismatch(u8),
    /// The texture does not match the requested rendering format.
    #[error("Use limited-range BT.709 textures with matching chroma format and bit depth.")]
    IncompatibleFormat,
    /// The requested storage exceeds the resource policy.
    #[error(transparent)]
    Limit(#[from] DecodeLimitError),
    /// The allocator could not reserve storage for the canvas or row scratch.
    #[error("Cannot allocate the atlas canvas or row scratch.")]
    Allocation,
    /// The plane or sample storage is invalid.
    #[error(transparent)]
    Plane(#[from] ReconError),
}

type Result<T> = std::result::Result<T, AtlasCompositionError>;

/// A reusable destination for explicit composition. It retains no input pictures.
pub struct AtlasCompositor {
    limits: DecodeLimits,
    canvas: Storage,
    alpha: Storage,
}

#[derive(Default)]
struct Storage {
    eight: Vec<u8>,
    ten: Vec<u16>,
}

impl AtlasCompositor {
    /// Creates an empty compositor. The byte limit covers the canvas and row scratch.
    /// An unlimited decode byte threshold selects a 256 MiB presentation budget.
    #[must_use]
    pub fn new(limits: DecodeLimits) -> Self {
        let limits = match limits.max_decoded_frame_bytes() {
            DecodeLimitThreshold::Unlimited => {
                limits.with_max_decoded_frame_bytes(DecodeLimitThreshold::Max(256 * 1024 * 1024))
            }
            DecodeLimitThreshold::Max(_) => limits,
        };
        Self {
            limits,
            canvas: Storage::default(),
            alpha: Storage::default(),
        }
    }

    /// Composes one group of pictures that share a global layout and a presentation timestamp.
    /// Supply timestamps in a common time base when you combine pictures from separate decoders.
    ///
    /// This operation selects limited-range BT.709 YCbCr. It interprets unspecified color as BT.709.
    /// All textures must use the same bit depth and chroma format. Alpha can use either bit depth.
    /// Nearest-neighbor selects the source coordinate at floor(output coordinate × source extent / segment extent).
    ///
    /// # Errors
    /// Returns immediately if an input is missing, stale, incompatible, or exceeds a limit.
    pub fn compose<'a>(
        &'a mut self,
        layout: &Arc<AtlasLayout>,
        pts: i64,
        pictures: &[&Picture],
    ) -> Result<AtlasComposite<'a>> {
        let (AtlasModeInfo::Multistream(info) | AtlasModeInfo::MultistreamAlpha(info)) =
            &layout.atlas.mode_info
        else {
            return Err(AtlasCompositionError::UnsupportedMode);
        };
        let (streams, has_alpha) = validate_segments(&info.segments, self.limits)?;
        let mut frames = [None; 31];
        for &picture in pictures {
            let xlayer = picture.extended_layer_id().get();
            let slot = frames
                .get_mut(usize::from(xlayer))
                .ok_or(AtlasCompositionError::UnexpectedPicture(xlayer))?;
            if streams & (1 << xlayer) == 0 || slot.replace(picture).is_some() {
                return Err(AtlasCompositionError::UnexpectedPicture(xlayer));
            }
            if picture.props().pts != pts {
                return Err(AtlasCompositionError::TimestampMismatch(xlayer));
            }
        }
        let mut texture: Option<&Picture> = None;
        for (index, segment) in info.segments.iter().enumerate() {
            let id = segment.input_stream_id;
            let frame = frames[usize::from(id)].ok_or(AtlasCompositionError::MissingPicture(id))?;
            if !frame.atlas_associations().iter().any(|association| {
                association.scope == GLOBAL_XLAYER_ID
                    && association
                        .layout
                        .as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, layout))
                    && association.segment_index() == Some(index)
            }) {
                return Err(AtlasCompositionError::StaleLayout(id));
            }
            if !segment.alpha_segment_flag {
                let color = frame.content_interpretation().map_or(
                    DerivedColorInfo::UNSPECIFIED,
                    ContentInterpretation::derived_color,
                );
                if !compatible_color(color)
                    || frame.pixel_format().is_monochrome()
                    || texture.is_some_and(|other| {
                        (other.bit_depth(), other.pixel_format())
                            != (frame.bit_depth(), frame.pixel_format())
                    })
                {
                    return Err(AtlasCompositionError::IncompatibleFormat);
                }
                texture = Some(frame);
            }
        }
        let texture = texture.ok_or(AtlasCompositionError::InvalidLayout)?;
        let geometry = Geometry::new(
            [info.width, info.height],
            texture.pixel_format(),
            texture.bit_depth(),
            self.limits,
            has_alpha,
        )?;
        texture.backend.enter(Render {
            compositor: self,
            frames,
            info,
            geometry,
            pts,
            has_alpha,
        })
    }
}

fn validate_segments(
    segments: &[AtlasMultistreamSegment],
    limits: DecodeLimits,
) -> Result<(u32, bool)> {
    let mut streams = 0u32;
    let mut pending_alpha = false;
    let mut has_alpha = false;
    for segment in segments {
        let id = segment.input_stream_id;
        if id >= GLOBAL_XLAYER_ID.get()
            || streams & (1 << id) != 0
            || segment.width == 0
            || segment.height == 0
            || (pending_alpha && segment.alpha_segment_flag)
        {
            return Err(AtlasCompositionError::InvalidLayout);
        }
        streams |= 1 << id;
        limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(segment.width))?;
        limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(segment.height))?;
        pending_alpha = segment.alpha_segment_flag;
        has_alpha |= pending_alpha;
    }
    if pending_alpha || streams == 0 {
        return Err(AtlasCompositionError::InvalidLayout);
    }
    Ok((streams, has_alpha))
}

fn compatible_color(color: DerivedColorInfo) -> bool {
    let bt709 = ColorPrimariesTriple {
        color_primaries: 1,
        transfer_characteristics: 1,
        matrix_coefficients: 1,
    };
    !color.full_range && (color == DerivedColorInfo::UNSPECIFIED || color.code_points == bt709)
}

#[derive(Clone, Copy)]
struct Geometry {
    size: [u32; 2],
    chroma: [u32; 2],
    depth: BitDepth,
    format: PixelFormat,
    samples: usize,
}

impl Geometry {
    fn new(
        size: [u32; 2],
        format: PixelFormat,
        depth: BitDepth,
        limits: DecodeLimits,
        has_alpha: bool,
    ) -> Result<Self> {
        let [width, height] = size;
        let chroma = [
            width >> format.subsampling_x(),
            height >> format.subsampling_y(),
        ];
        if chroma.contains(&0) {
            return Err(AtlasCompositionError::InvalidLayout);
        }
        limits.ensure(DecodeLimitName::MaxFrameWidth, u64::from(width))?;
        limits.ensure(DecodeLimitName::MaxFrameHeight, u64::from(height))?;
        let area = limits
            .ensure_mul(
                DecodeLimitName::MaxLumaSamplesPerFrame,
                u64::from(width),
                u64::from(height),
            )?
            .actual();
        let uv = u64::from(chroma[0]) * u64::from(chroma[1]);
        let samples = uv
            .checked_mul(2)
            .and_then(|x| x.checked_add(area))
            .ok_or(AtlasCompositionError::InvalidLayout)?;
        let bytes = if depth == BitDepth::Eight { 1 } else { 2 };
        let scratch = if has_alpha {
            u64::from(width) * (bytes + 2)
        } else {
            0
        };
        let total = samples
            .checked_mul(bytes)
            .and_then(|x| x.checked_add(scratch))
            .ok_or(AtlasCompositionError::InvalidLayout)?;
        limits.ensure_allocation_len(DecodeLimitName::MaxDecodedFrameBytes, total)?;
        Ok(Self {
            size,
            chroma,
            depth,
            format,
            samples: samples as usize,
        })
    }

    fn plane_size(self, plane: PlaneId) -> [u32; 2] {
        if plane == PlaneId::Y {
            self.size
        } else {
            self.chroma
        }
    }
}

/// A borrowed composite in planar Y, U, V order. The next call can reuse its storage.
pub struct AtlasComposite<'a> {
    geometry: Geometry,
    pts: i64,
    planes: [PictureSamples<'a>; 3],
}

impl<'a> AtlasComposite<'a> {
    /// Width in luma samples.
    pub const fn width(&self) -> usize {
        self.geometry.size[0] as usize
    }
    /// Height in luma samples.
    pub const fn height(&self) -> usize {
        self.geometry.size[1] as usize
    }
    /// Native texture bit depth.
    pub const fn bit_depth(&self) -> BitDepth {
        self.geometry.depth
    }
    /// Chroma sampling of the output.
    pub const fn pixel_format(&self) -> PixelFormat {
        self.geometry.format
    }
    /// Presentation timestamp in the caller's time base.
    pub const fn pts(&self) -> i64 {
        self.pts
    }
    /// Borrows one canvas plane.
    pub fn plane(&self, plane: PlaneId) -> Option<PictureSamples<'a>> {
        self.planes.get(plane.index()).copied()
    }
    /// Writes the Y, U, and V samples. Ten-bit samples use little-endian words.
    ///
    /// # Errors
    /// Returns the error from the writer.
    pub fn write_raw(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        for plane in self.planes {
            match plane {
                PictureSamples::Eight(view) => {
                    splot_recon::write_visible_plane(BitDepth::Eight, view, writer)?;
                }
                PictureSamples::Ten(view) => {
                    splot_recon::write_visible_plane(BitDepth::Ten, view, writer)?;
                }
            }
        }
        Ok(())
    }
}

struct Render<'a, 'b> {
    compositor: &'a mut AtlasCompositor,
    frames: [Option<&'b Picture>; 31],
    info: &'b AtlasMultistreamInfo,
    geometry: Geometry,
    pts: i64,
    has_alpha: bool,
}

impl<'a> SimdScope<Result<AtlasComposite<'a>>> for Render<'a, '_> {
    fn run<B: SimdBackend>(self, backend: B) -> Result<AtlasComposite<'a>> {
        let width = self.geometry.size[0] as usize;
        let needed = self.geometry.samples + if self.has_alpha { width } else { 0 };
        let mask_bytes = if self.has_alpha { 2 * width } else { 0 };
        if self.compositor.alpha.capacity_bytes() > mask_bytes {
            self.compositor.alpha = Storage::default();
        }
        let storage = &mut self.compositor.canvas;
        if !self
            .compositor
            .limits
            .max_decoded_frame_bytes()
            .allows((storage.capacity_bytes() as u64).saturating_add(mask_bytes as u64))
        {
            *storage = Storage::default();
        }
        let planes = match self.geometry.depth {
            BitDepth::Eight => render::canvas(
                backend,
                storage.eight(needed)?,
                &mut self.compositor.alpha,
                self.info,
                &self.frames,
                self.geometry,
            )?,
            BitDepth::Ten => render::canvas(
                backend,
                storage.ten(needed)?,
                &mut self.compositor.alpha,
                self.info,
                &self.frames,
                self.geometry,
            )?,
        };
        Ok(AtlasComposite {
            geometry: self.geometry,
            pts: self.pts,
            planes,
        })
    }
}

impl Storage {
    fn capacity_bytes(&self) -> usize {
        self.eight.capacity() + self.ten.capacity() * 2
    }

    fn eight(&mut self, len: usize) -> Result<&mut [u8]> {
        self.ten = Vec::new();
        overwrite_buffer(&mut self.eight, len)
    }
    fn ten(&mut self, len: usize) -> Result<&mut [u16]> {
        self.eight = Vec::new();
        overwrite_buffer(&mut self.ten, len)
    }
}

fn overwrite_buffer<T: Default + Copy>(storage: &mut Vec<T>, len: usize) -> Result<&mut [T]> {
    if len > storage.capacity() {
        *storage = Vec::new();
        storage
            .try_reserve_exact(len)
            .map_err(|_| AtlasCompositionError::Allocation)?;
    }
    if len > storage.len() {
        storage.resize(len, T::default());
    }
    Ok(&mut storage[..len])
}
