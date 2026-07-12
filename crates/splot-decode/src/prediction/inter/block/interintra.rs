// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.3.30 inter-intra prediction: per-plane intra predictors built
//! from reconstructed neighbour edges, blended over the in-storage inter
//! prediction by the block engine.

use std::borrow::Cow;

use splot_core::span::ByteOffset;
use splot_recon::{
    BitDepth, CurrentFrameIntraEdges, CurrentFrameWorkspace, InterIntraMode,
    IntraCardinalDirection, IntraDirectionalAngleEdges, IntraRectBlockSize, IntraSmoothMode,
    PlaneId as ReconPlaneId, ReconSample, apply_intra_ibp_dc_rect,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_value,
};

use super::super::{PlacedInterBlock, mc, unsupported_at};
use super::MI_SIZE;
use crate::Result;
use crate::bitstream::tile_payload::TileBlockDecodedState;
use crate::pipeline::reconstruct::{
    SmoothIntraPredictionRequest, predict_intra_smooth_over_available_edges,
};

macro_rules! inter_diag {
    ($reason:literal, $offset:expr, $message:literal, $spec_section:expr $(,)?) => {
        unsupported_at($reason, $offset, $message, $spec_section)
    };
}

fn interintra_cardinal_edge<T: ReconSample>(
    mode: InterIntraMode,
    edges: &CurrentFrameIntraEdges<T>,
    len: usize,
    bit_depth: BitDepth,
) -> splot_recon::Result<Cow<'_, [T]>> {
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
        InterIntraMode::Vertical => edges.above_samples().map_or_else(
            || sample(true).map(|value| Cow::Owned(vec![value; len])),
            |above| Ok(Cow::Borrowed(above)),
        ),
        InterIntraMode::Horizontal => edges.left_samples().map_or_else(
            || sample(false).map(|value| Cow::Owned(vec![value; len])),
            |left| Ok(Cow::Borrowed(left)),
        ),
        InterIntraMode::Dc | InterIntraMode::Smooth => Ok(Cow::Borrowed(&[])),
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

pub(super) struct InterIntraPlanePrediction<T> {
    pub(super) plane: ReconPlaneId,
    pub(super) sub_x: u32,
    pub(super) sub_y: u32,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) size: IntraRectBlockSize,
    pub(super) samples: Vec<T>,
}

pub(super) fn predict_interintra_planes<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    placed: &PlacedInterBlock,
    block_decoded: &TileBlockDecodedState,
    mode: InterIntraMode,
    enable_ibp: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<Vec<InterIntraPlanePrediction<T>>> {
    let geometry_error = || {
        inter_diag!(
            "inter_interintra_geometry",
            tile_offset,
            "invalid interintra plane geometry",
            "5.20.7.22"
        )
    };
    let mut planes = Vec::with_capacity(mc::YUV420_MC_PLANES.len());
    for (plane, sub_x, sub_y) in mc::YUV420_MC_PLANES {
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
            return Err(geometry_error());
        }
        let log2_w = u8::try_from(w.trailing_zeros()).map_err(|_| geometry_error())?;
        let log2_h = u8::try_from(h.trailing_zeros()).map_err(|_| geometry_error())?;
        let size = IntraRectBlockSize::new(log2_w, log2_h).map_err(|_| geometry_error())?;
        let edges = workspace
            .intra_dc_edges_for_rect(plane, x, y, size)
            .map_err(|_| geometry_error())?;
        let mut samples = vec![T::default(); w * h];
        match mode {
            InterIntraMode::Dc => {
                let dc = predict_intra_dc_rect_value(bit_depth, size, edges.as_dc_edges())
                    .map_err(|_| geometry_error())?;
                samples.fill(dc);
                if enable_ibp && !(w == 4 && h == 4) {
                    apply_intra_ibp_dc_rect(bit_depth, size, edges.as_dc_edges(), &mut samples, w)
                        .map_err(|_| geometry_error())?;
                }
            }
            InterIntraMode::Vertical | InterIntraMode::Horizontal => {
                let (direction, edge) = if mode == InterIntraMode::Vertical {
                    (
                        IntraCardinalDirection::Vertical,
                        interintra_cardinal_edge(mode, &edges, w, bit_depth)
                            .map_err(|_| geometry_error())?,
                    )
                } else {
                    (
                        IntraCardinalDirection::Horizontal,
                        interintra_cardinal_edge(mode, &edges, h, bit_depth)
                            .map_err(|_| geometry_error())?,
                    )
                };
                let prepared = if mode == InterIntraMode::Vertical {
                    IntraDirectionalAngleEdges::above(edge.as_ref())
                } else {
                    IntraDirectionalAngleEdges::left(edge.as_ref())
                };
                predict_intra_cardinal_directional_rect_into(
                    bit_depth,
                    size,
                    direction,
                    prepared,
                    &mut samples,
                    w,
                )
                .map_err(|_| geometry_error())?;
            }
            InterIntraMode::Smooth => {
                let x4 = x / MI_SIZE;
                let y4 = y / MI_SIZE;
                let w4 = (w / MI_SIZE).max(1);
                let h4 = (h / MI_SIZE).max(1);
                samples = predict_intra_smooth_over_available_edges(
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
                )
                .map_err(|_| geometry_error())?;
            }
        }
        planes.push(InterIntraPlanePrediction {
            plane,
            sub_x,
            sub_y,
            x,
            y,
            size,
            samples,
        });
    }
    Ok(planes)
}
