// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.30 inter-intra prediction: per-plane intra predictors built
//! from reconstructed neighbour edges, blended over the in-storage inter
//! prediction by the block engine.
//!
//! Feature tracking: `INFRA-DECODE-SERIAL-HOT-PATHS`.

use splot_recon::{
    BitDepth, CurrentFrameIntraEdges, CurrentFrameWorkspace, InterIntraMode,
    IntraCardinalDirection, IntraDirectionalAngleEdges, IntraRectBlockSize, IntraSmoothMode,
    PlaneId as ReconPlaneId, ReconError, ReconSample, apply_intra_ibp_dc_rect,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_value,
};

use super::super::{PlacedInterBlock, mc};
use super::MI_SIZE;
use crate::Result;
use crate::bitstream::tile_payload::{GeneralIntraResidualError, TileBlockDecodedState};
use crate::error::{DecodeError, DecodeHeaderStateError};
use crate::pipeline::reconstruct::{
    SmoothIntraPredictionRequest, predict_intra_smooth_over_available_edges_into,
};

fn interintra_cardinal_edge<'a, T: ReconSample>(
    mode: InterIntraMode,
    edges: &'a CurrentFrameIntraEdges<T>,
    len: usize,
    bit_depth: BitDepth,
    fallback: &'a mut Vec<T>,
) -> splot_recon::Result<&'a [T]> {
    let sample = |above: bool| {
        if above {
            edges
                .left_samples()
                .and_then(|left| left.first().copied())
                .map_or_else(|| no_neighbour_above(bit_depth), Ok)
        } else {
            edges
                .above_samples()
                .and_then(|above_edge| above_edge.first().copied())
                .map_or_else(|| no_neighbour_left(bit_depth), Ok)
        }
    };
    match mode {
        InterIntraMode::Vertical => {
            if let Some(above) = edges.above_samples() {
                Ok(above)
            } else {
                fallback.clear();
                fallback.resize(len, sample(true)?);
                Ok(fallback)
            }
        }
        InterIntraMode::Horizontal => {
            if let Some(left) = edges.left_samples() {
                Ok(left)
            } else {
                fallback.clear();
                fallback.resize(len, sample(false)?);
                Ok(fallback)
            }
        }
        InterIntraMode::Dc | InterIntraMode::Smooth => Ok(&[]),
    }
}

fn no_neighbour_above<T: ReconSample>(bit_depth: BitDepth) -> splot_recon::Result<T> {
    let midpoint = 1u16 << (u32::from(bit_depth.bits()) - 1);
    T::try_from_u16(midpoint - 1)
}

fn no_neighbour_left<T: ReconSample>(bit_depth: BitDepth) -> splot_recon::Result<T> {
    let midpoint = 1u16 << (u32::from(bit_depth.bits()) - 1);
    T::try_from_u16(midpoint + 1)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct InterIntraPlanePrediction {
    pub(super) plane: ReconPlaneId,
    pub(super) sub_x: u32,
    pub(super) sub_y: u32,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) size: IntraRectBlockSize,
    sample_start: usize,
}

#[derive(Debug)]
pub(super) struct InterIntraScratch<T> {
    planes: [Option<InterIntraPlanePrediction>; 3],
    plane_count: usize,
    samples: Vec<T>,
    fallback_edge: Vec<T>,
}

impl<T> Default for InterIntraScratch<T> {
    fn default() -> Self {
        Self {
            planes: [None; 3],
            plane_count: 0,
            samples: Vec::new(),
            fallback_edge: Vec::new(),
        }
    }
}

impl<T> InterIntraScratch<T> {
    pub(super) fn planes(&self) -> impl Iterator<Item = (InterIntraPlanePrediction, &[T])> {
        self.planes[..self.plane_count]
            .iter()
            .flatten()
            .map(|plane| {
                let end = plane.sample_start + plane.size.sample_count();
                (*plane, &self.samples[plane.sample_start..end])
            })
    }

    fn reset(&mut self) {
        self.plane_count = 0;
        self.samples.clear();
        self.fallback_edge.clear();
    }

    #[cfg(test)]
    pub(super) fn storage_identity(&self) -> [(usize, usize); 2] {
        [
            (self.samples.as_ptr() as usize, self.samples.capacity()),
            (
                self.fallback_edge.as_ptr() as usize,
                self.fallback_edge.capacity(),
            ),
        ]
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn predict_interintra_planes<T: ReconSample>(
    scratch: &mut InterIntraScratch<T>,
    workspace: &CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    block_decoded: &TileBlockDecodedState,
    mode: InterIntraMode,
    enable_ibp: bool,
    bit_depth: BitDepth,
) -> Result<()> {
    scratch.reset();
    for (plane, sub_x, sub_y) in mc::mc_planes(workspace.info().pixel_format()) {
        if plane != ReconPlaneId::Y && !placed.interintra_chroma {
            continue;
        }
        let (luma_x, luma_y, luma_w, luma_h) = if plane == ReconPlaneId::Y {
            (placed.luma_x, placed.luma_y, placed.luma_w, placed.luma_h)
        } else {
            (
                placed.chroma_luma_x,
                placed.chroma_luma_y,
                placed.chroma_luma_w,
                placed.chroma_luma_h,
            )
        };
        let x = luma_x >> sub_x;
        let y = luma_y >> sub_y;
        let w = luma_w >> sub_x;
        let h = luma_h >> sub_y;
        if !w.is_power_of_two() || !h.is_power_of_two() {
            return Err(DecodeHeaderStateError::InvalidBlockGeometry.into());
        }
        let log2_w = u8::try_from(w.trailing_zeros())
            .map_err(|_| DecodeHeaderStateError::InvalidBlockGeometry)?;
        let log2_h = u8::try_from(h.trailing_zeros())
            .map_err(|_| DecodeHeaderStateError::InvalidBlockGeometry)?;
        let size = IntraRectBlockSize::new(log2_w, log2_h)
            .map_err(|_| DecodeHeaderStateError::InvalidBlockGeometry)?;
        let edges = workspace
            .intra_dc_edges_for_rect(plane, x, y, size)
            .map_err(|error| match error {
                ReconError::WorkspaceRectOutOfBounds { .. } => {
                    DecodeError::from(DecodeHeaderStateError::InvalidBlockGeometry)
                }
                _ => DecodeError::from(error),
            })?;
        let sample_start = scratch.samples.len();
        let sample_end = sample_start
            .checked_add(size.sample_count())
            .ok_or(DecodeHeaderStateError::InvalidBlockGeometry)?;
        scratch.samples.resize(sample_end, T::default());
        let samples = &mut scratch.samples[sample_start..];
        match mode {
            InterIntraMode::Dc => {
                let dc = predict_intra_dc_rect_value(bit_depth, size, edges.as_dc_edges())?;
                samples.fill(dc);
                if enable_ibp && !(w == 4 && h == 4) {
                    apply_intra_ibp_dc_rect(bit_depth, size, edges.as_dc_edges(), samples, w)?;
                }
            }
            InterIntraMode::Vertical | InterIntraMode::Horizontal => {
                let (direction, edge) = if mode == InterIntraMode::Vertical {
                    (
                        IntraCardinalDirection::Vertical,
                        interintra_cardinal_edge(
                            mode,
                            &edges,
                            w,
                            bit_depth,
                            &mut scratch.fallback_edge,
                        )?,
                    )
                } else {
                    (
                        IntraCardinalDirection::Horizontal,
                        interintra_cardinal_edge(
                            mode,
                            &edges,
                            h,
                            bit_depth,
                            &mut scratch.fallback_edge,
                        )?,
                    )
                };
                let prepared = if mode == InterIntraMode::Vertical {
                    IntraDirectionalAngleEdges::above(edge)
                } else {
                    IntraDirectionalAngleEdges::left(edge)
                };
                predict_intra_cardinal_directional_rect_into(
                    bit_depth, size, direction, prepared, samples, w,
                )?;
            }
            InterIntraMode::Smooth => {
                let sb_mask = block_decoded.sb_size4().saturating_sub(1);
                let x4 = ((luma_x / MI_SIZE) & sb_mask) >> sub_x;
                let y4 = ((luma_y / MI_SIZE) & sb_mask) >> sub_y;
                let w4 = (w / MI_SIZE).max(1);
                let h4 = (h / MI_SIZE).max(1);
                predict_intra_smooth_over_available_edges_into(
                    workspace,
                    SmoothIntraPredictionRequest {
                        plane_id: plane,
                        x,
                        y,
                        block_size: size,
                        mode: IntraSmoothMode::Smooth,
                        available_left_samples: None,
                        available_above_samples: None,
                        num4_above_right: block_decoded.count_top_right_avail(
                            plane.index(),
                            x4,
                            y4,
                            w4,
                        ),
                        num4_below_left: block_decoded.count_bottom_left_avail(
                            plane.index(),
                            x4,
                            y4,
                            h4,
                        ),
                        bit_depth,
                    },
                    samples,
                )
                .map_err(|error| match error {
                    GeneralIntraResidualError::Reconstruct { source } => DecodeError::from(source),
                    _ => DecodeHeaderStateError::InvalidBlockGeometry.into(),
                })?;
            }
        }
        let prediction = InterIntraPlanePrediction {
            plane,
            sub_x,
            sub_y,
            x,
            y,
            size,
            sample_start,
        };
        scratch.planes[scratch.plane_count] = Some(prediction);
        scratch.plane_count += 1;
    }
    Ok(())
}
