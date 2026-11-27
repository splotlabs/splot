// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder-facing GDF facade: the `use_gdf` state of § 5.20.10.3 and the
//! frame's GDF unit grid.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;

use super::super::cdf::TileCdfSelector;
use super::super::tile_frontier::minimal_partition_frame_facts;
use super::{
    EncoderReferences, EncoderSyntaxError, FrameStart, IntraTileSyntax, LeafBlock, Result, rule,
    tile_ranges,
};
use splot_recon::{BitDepth, DecodedFrame};

use crate::filters::gdf::{GDF_ERROR_BASE, GdfBlockGrid, GdfReferenceContext, gdf_stripe_row};
use crate::filters::source::{FramePlane, StripePlane};
use crate::filters::wienerns_lr::tx_records::gdf::GdfState;

impl IntraTileSyntax {
    /// The `use_gdf` CDF row (§ 5.20.10.3).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] when the row is missing.
    pub fn use_gdf_cdf(&mut self) -> Result<&mut [u16]> {
        self.tile_row(TileCdfSelector::UseGdf)
    }
}

impl FrameStart {
    /// A copy of the `use_gdf` CDF row every tile of the frame starts from.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] when the row is missing.
    pub fn use_gdf_cdf(&self) -> Result<Vec<u16>> {
        let mut cdfs = self.cdfs.tile_copy();
        let row = cdfs
            .row_mut(TileCdfSelector::UseGdf)
            .map_err(rule("CDF row"))?;
        Ok(row.to_vec())
    }
}

/// The `use_gdf` units of a frame that codes `gdf_per_block = 1`
/// (§ 5.20.10.3), in raster order over `GdfBlkSize` luma blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GdfGrid {
    /// `GdfBlkSize` in luma samples.
    pub block_size: usize,
    /// Unit rows.
    pub rows: usize,
    /// Unit columns.
    pub cols: usize,
}

impl GdfGrid {
    /// The grid of the frame `core` coded with `seq`, or `None` when the
    /// frame codes no `use_gdf`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for missing frame facts.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Option<Self>> {
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let state = GdfState::new(frame.mi_rows(), frame.mi_cols(), seq, core)
            .map_err(rule("GDF state"))?;
        let block_size = state.block_size();
        Ok((block_size > 0).then(|| Self {
            block_size,
            rows: (frame.mi_rows() * 4).div_ceil(block_size),
            cols: (frame.mi_cols() * 4).div_ceil(block_size),
        }))
    }

    /// The unit row whose `use_gdf` governs luma row `y` of a frame
    /// `luma_height` rows high: GDF filters 64-row stripes offset up by 8
    /// rows (§ 7.20.1), and each stripe follows the unit of its stripe row.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Rule`] for a row outside the frame.
    pub fn unit_row(&self, core: &FrameHeaderCore, y: usize, luma_height: usize) -> Result<usize> {
        let row = gdf_stripe_row(core, y, luma_height).map_err(rule("GDF stripe"))?;
        Ok((row * 4 / self.block_size).min(self.rows.saturating_sub(1)))
    }
}

/// The `use_gdf` state of one tile of a frame, filled in coding order.
pub struct GdfBlockMap {
    state: GdfState,
}

impl GdfBlockMap {
    /// An empty map for tile `tile` of the frame `core` coded with `seq`.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for missing frame facts or a tile the
    /// frame does not have.
    pub fn new(seq: &SequenceHeader, core: &FrameHeaderCore, tile: usize) -> Result<Self> {
        let frame = minimal_partition_frame_facts(seq, core).map_err(rule("frame facts"))?;
        let (rows, cols) = tile_ranges(core, tile)?;
        let state = GdfState::new(frame.mi_rows(), frame.mi_cols(), seq, core)
            .and_then(|state| state.for_tile(rows, cols, None))
            .map_err(rule("GDF state"))?;
        Ok(Self { state })
    }

    /// The `(row, col)` unit whose `use_gdf` `leaf` codes, if any.
    #[must_use]
    pub fn unit(&self, leaf: &LeafBlock) -> Option<(usize, usize)> {
        self.state.unit_for(&leaf.frontier)
    }

    /// The units of the tile in coding order.
    #[must_use]
    pub fn units(&self) -> Vec<(usize, usize)> {
        self.state.units().collect()
    }
}

/// The `use_gdf` flags of a frame in [`GdfGrid`] raster order and the state
/// its GDF reads, as [`super::apply_loop_filters`] takes them.
#[derive(Clone, Copy)]
pub struct GdfFlags<'a> {
    /// `use_gdf` per unit; empty when the frame codes none.
    pub blocks: &'a [u8],
    /// The references of the frame, whose order hints pick the GDF tables of
    /// an inter frame (§ 7.20.5 `refDstIdx`).
    pub refs: &'a EncoderReferences,
}

impl GdfFlags<'_> {
    /// The decoder's GDF block grid and reference context of the frame `core`;
    /// no `gdf`, or no `blocks`, codes every `use_gdf` of the grid as `0`.
    pub(super) fn decoder_inputs(
        gdf: Option<Self>,
        seq: &SequenceHeader,
        core: &FrameHeaderCore,
    ) -> Result<(Option<GdfBlockGrid>, Option<GdfReferenceContext>)> {
        let blocks = gdf.map_or(&[][..], |gdf| gdf.blocks);
        let grid = match GdfGrid::new(seq, core)? {
            Some(grid) => {
                let values = match blocks {
                    [] => vec![0; grid.rows * grid.cols],
                    blocks => blocks.to_vec(),
                };
                Some(
                    GdfBlockGrid::new(grid.block_size, grid.rows, grid.cols, values).ok_or(
                        EncoderSyntaxError::Unsupported("use_gdf grid of another size"),
                    )?,
                )
            }
            None => None,
        };
        let reference = gdf.zip(core.inter.as_ref()).map(|(gdf, inter)| {
            let view = gdf.refs.header_view();
            GdfReferenceContext::from_reference_list(
                core.display_order_hint().unwrap_or(0),
                inter.ref_frame_idx.as_slice(),
                view.ref_order_hint.unwrap_or(&[]),
            )
        });
        Ok((grid, reference))
    }
}

/// The § 7.20.5 `err` of every luma sample of the frame `core` that a unit
/// of `gdf` enables (`0` elsewhere), row by row over the luma storage plane.
/// GDF of `GdfPixScale` `s` gives the sample `Clip1(lr + Round2Signed(err * s,
/// 12 - BitDepth))`, where `lr` is the sample after loop restoration.
///
/// `deblocked` and `cdef` are the chain's frames of the same decisions after
/// deblocking and after CDEF and CCSO: each 64-row stripe reads the CDEF
/// output inside the stripe and the deblocked frame outside it. Returns the
/// row width and the errors.
///
/// # Errors
/// Returns [`EncoderSyntaxError`] when the frames differ in size, the frame
/// has a lossless segment (whose samples GDF leaves alone), or a GDF stage
/// rejects its inputs.
pub fn gdf_errors(
    seq: &SequenceHeader,
    core: &FrameHeaderCore,
    (deblocked, cdef): (&DecodedFrame<u16>, &DecodedFrame<u16>),
    gdf: GdfFlags<'_>,
) -> Result<(usize, Vec<i16>)> {
    if core
        .lossless_info
        .is_some_and(|info| info.has_lossless_segment)
    {
        return Err(EncoderSyntaxError::Unsupported(
            "GDF errors of a frame with a lossless segment",
        ));
    }
    let (grid, reference) = GdfFlags::decoder_inputs(Some(gdf), seq, core)?;
    let size = cdef.y().storage_size();
    let (width, height) = (size.width(), size.height());
    let packed = |frame: &DecodedFrame<u16>| -> Result<Vec<u16>> {
        let y = frame.y();
        if y.storage_size() != size {
            return Err(EncoderSyntaxError::Unsupported(
                "GDF frames of another size",
            ));
        }
        let stride = y.stride_samples();
        Ok((0..height)
            .flat_map(|row| &y.samples()[row * stride..row * stride + width])
            .copied()
            .collect())
    };
    let shape = || EncoderSyntaxError::Unsupported("GDF stripe shape");
    let (deblocked_y, cdef_y) = (packed(deblocked)?, packed(cdef)?);
    let deblocked_plane =
        FramePlane::window(&deblocked_y, width, height, 0, height).ok_or_else(shape)?;
    let stripe = |samples: Vec<u16>, start: usize| {
        StripePlane::from_samples(width, height, start, samples).ok_or_else(shape)
    };
    let cdef_frame = [stripe(cdef_y.clone(), 0)?];
    let disable_across_tiles = seq
        .filter
        .is_some_and(|filter| filter.disable_loopfilters_across_tiles);
    let mut errors = vec![0i16; width * height];
    {
        struct StripeScope<'a> {
            core: &'a FrameHeaderCore,
            width: usize,
            height: usize,
            deblocked_plane: FramePlane<'a, u16>,
            cdef_y: &'a [u16],
            cdef_frame: &'a [StripePlane<u16>],
            grid: Option<&'a GdfBlockGrid>,
            bit_depth: BitDepth,
            disable_across_tiles: bool,
            reference: Option<GdfReferenceContext>,
            errors: &'a mut [i16],
        }
        impl splot_core::simd::SimdScope<Result<()>> for StripeScope<'_> {
            fn run<B: splot_core::simd::SimdBackend>(self, backend: B) -> Result<()> {
                let narrow = backend.narrow();
                let stripe = |samples: Vec<u16>, start: usize| {
                    StripePlane::from_samples(self.width, self.height, start, samples)
                        .ok_or(EncoderSyntaxError::Unsupported("GDF stripe shape"))
                };
                for range in crate::filters::gdf::stripe_ranges(self.core, self.height) {
                    let (start, end) = range.map_err(rule("GDF stripe"))?;
                    let rows = start * self.width..end * self.width;
                    let separate = stripe(self.cdef_y[rows.clone()].to_vec(), start)?;
                    let base = vec![GDF_ERROR_BASE; rows.len()];
                    let mut output = stripe(base, start)?;
                    crate::filters::gdf::error_stripe(
                        narrow,
                        self.core,
                        self.deblocked_plane,
                        Some(&separate),
                        self.cdef_frame,
                        &mut output,
                        self.grid,
                        self.bit_depth,
                        self.disable_across_tiles,
                        self.reference,
                    )
                    .map_err(rule("GDF"))?;
                    for (error, &sample) in self.errors[rows].iter_mut().zip(output.samples()) {
                        *error = sample as i16 - GDF_ERROR_BASE as i16;
                    }
                }
                Ok(())
            }
        }
        splot_core::simd::resolve_backend(splot_core::simd::SimdPolicy::Auto)
            .map_err(|_| EncoderSyntaxError::Unsupported("SIMD backend"))?
            .enter(StripeScope {
                core,
                width,
                height,
                deblocked_plane,
                cdef_y: &cdef_y,
                cdef_frame: &cdef_frame,
                grid: grid.as_ref(),
                bit_depth: cdef.bit_depth(),
                disable_across_tiles,
                reference,
                errors: &mut errors,
            })?;
    }
    Ok((width, errors))
}
