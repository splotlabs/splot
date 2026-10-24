// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! CCSO search (§ 7.19).
//!
//! One statistics pass per candidate classification reads the deblocked luma,
//! the CDEF output and the source, and keeps `source - filtered` and a count
//! per class of every unit; each CCSO unit row is one worker-pool job. Every
//! band and edge variant of a candidate is a sum of these classes, so no
//! candidate runs the filter. A variant takes the offset of each class that minimizes the squared
//! error plus the `ccso_offset_idx` rate, turns each unit on or off with the
//! exact `ccso_blk` rate of the adapting CDF, and refits the offsets on the
//! units that stay on.

use splot_core::headers::frame::{CcsoOffsets, CcsoPlaneParams, ccso_quant_step};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::adapt_cdf_u16;
use splot_decode::encoder_syntax::{
    CcsoBlockMap, CcsoGrid, FrameStart, ccso_offset, ccso_taps, tile_count,
};
use splot_parallel::WorkerPool;
use splot_recon::{DecodedFrame, PlaneId};

use super::coding;
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::intra_frame::map_tiles;
use crate::kernels::ccso::{BANDS, CLASSES, CcsoRow, CcsoStats, EDGE_CLASSES, accumulate_simd};
use crate::rd::{Bits, Lambda, symbol_cost, unary_cost};

/// `(ccso_ext_filter, ccso_scale_idx, ccso_quant_idx)` in the order the
/// search tries them.
const LADDER: [(u8, u8, u8); 8] = [
    (0, 0, 0),
    (1, 0, 0),
    (0, 0, 1),
    (1, 0, 1),
    (2, 0, 0),
    (3, 0, 0),
    (0, 1, 1),
    (1, 1, 1),
];
/// `ccso_offset_idx` values.
const OFFSET_INDICES: u8 = 8;
/// The `ccso_blk` CDF contexts.
const BLK_CONTEXTS: usize = 4;

/// What the search reads.
pub(super) struct CcsoInput<'a> {
    pub(super) seq: &'a SequenceHeader,
    /// A header of the frame whose every plane codes CCSO.
    pub(super) core: &'a splot_core::headers::frame::FrameHeaderCore,
    pub(super) start: &'a FrameStart,
    pub(super) deblocked: &'a DecodedFrame<u16>,
    pub(super) filtered: &'a DecodedFrame<u16>,
    pub(super) source: &'a SourceFrame,
    pub(super) lambda: Lambda,
    pub(super) pool: Option<&'a WorkerPool>,
}

/// The CCSO parameters of each plane and `ccso_blk` per plane and unit.
pub(super) type CcsoChoice = ([CcsoPlaneParams; 3], Vec<Vec<u8>>);

/// The plane parameters of `ccso_planes = 1` that every unit map accepts.
pub(super) fn placeholder_plane() -> CcsoPlaneParams {
    CcsoPlaneParams {
        ccso_planes: true,
        ccso_bo_only: Some(true),
        ccso_scale_idx: Some(0),
        ccso_quant_idx: Some(0),
        ccso_ext_filter: Some(0),
        ccso_edge_clf: Some(false),
        ccso_max_band_log2: Some(0),
        ccso_offset_idx: CcsoOffsets::from_iter_checked([0]).unwrap_or_default(),
        ..CcsoPlaneParams::default()
    }
}

/// One way to merge the statistics classes into coded classes.
#[derive(Clone, Copy)]
struct Variant {
    candidate: usize,
    bo_only: bool,
    edge_clf: bool,
    band_log2: u8,
}

impl Variant {
    const fn edge_interval(self) -> usize {
        match (self.bo_only, self.edge_clf) {
            (true, _) => 1,
            (false, true) => 2,
            (false, false) => EDGE_CLASSES,
        }
    }

    const fn classes(self) -> usize {
        (self.edge_interval() * self.edge_interval()) << self.band_log2
    }

    /// The coded class of statistics class `class`.
    const fn class(self, class: usize) -> usize {
        let band = (class % BANDS) >> (3 - self.band_log2);
        let edges = class / BANDS;
        let (mut cls0, mut cls1) = (edges / EDGE_CLASSES, edges % EDGE_CLASSES);
        if self.bo_only {
            (cls0, cls1) = (0, 0);
        } else if self.edge_clf {
            (cls0, cls1) = (
                if cls0 > 1 { 1 } else { cls0 },
                if cls1 > 1 { 1 } else { cls1 },
            );
        }
        ((cls0 * self.edge_interval() + cls1) << self.band_log2) + band
    }
}

/// A plane decision and its cost relative to CCSO off.
struct PlaneChoice {
    params: CcsoPlaneParams,
    on: Vec<bool>,
    cost: i128,
}

/// `delta` squared error plus `bits` at `lambda`, scaled as [`crate::rd::RdCost`].
pub(super) fn rd(delta: i64, bits: Bits, lambda: Lambda) -> i128 {
    (i128::from(delta) << (16 + Bits::FRAC_BITS))
        + i128::from(lambda.sse_per_bit_q16()) * i128::from(bits.0)
}

/// The squared-error change of adding `offset` to `count` samples whose
/// errors sum to `sum`.
const fn offset_delta(sum: i64, count: u32, offset: i32) -> i64 {
    let offset = offset as i64;
    count as i64 * offset * offset - 2 * offset * sum
}

/// The CCSO choice of each plane, `candidates` ladder entries deep.
pub(super) fn search(input: &CcsoInput<'_>, candidates: usize) -> Result<CcsoChoice> {
    let grid = CcsoGrid::new(input.seq, input.core).map_err(coding("ccso grid"))?;
    let ladder = &LADDER[..candidates.min(LADDER.len())];
    let planes = if input.filtered.plane(PlaneId::U).is_some() {
        3
    } else {
        1
    };
    let rows = map_tiles(input.pool, (0..grid.rows).collect(), |row| {
        unit_row_stats(input, grid, row, planes, ladder)
    })?;
    let mut params: [CcsoPlaneParams; 3] = Default::default();
    let mut blocks = Vec::new();
    for plane in 0..planes {
        let stats: Vec<Vec<CcsoStats>> = (0..ladder.len())
            .map(|candidate| {
                rows.iter()
                    .flat_map(|row| {
                        let start = (plane * ladder.len() + candidate) * grid.cols;
                        row[start..start + grid.cols].iter().cloned()
                    })
                    .collect()
            })
            .collect();
        let cdfs: Vec<Vec<u16>> = (0..BLK_CONTEXTS)
            .map(|ctx| input.start.ccso_blk_cdf(plane, ctx))
            .collect::<core::result::Result<_, _>>()
            .map_err(coding("ccso_blk CDF"))?;
        let mut best: Option<PlaneChoice> = None;
        for variant in variants(ladder) {
            let choice = evaluate(
                input,
                (plane, &cdfs, grid.cols),
                &stats[variant.candidate],
                ladder,
                variant,
            )?;
            if best.as_ref().is_none_or(|best| choice.cost < best.cost) {
                best = Some(choice);
            }
        }
        if let Some(choice) = best.filter(|choice| choice.cost < 0) {
            params[plane] = choice.params;
            blocks.resize(plane + 1, Vec::new());
            blocks[plane] = choice.on.iter().map(|&on| u8::from(on)).collect();
        }
    }
    Ok((params, blocks))
}

/// Every variant of the ladder: band-offset-only classes of the first
/// candidate, and the band and edge classes of each candidate.
fn variants(ladder: &[(u8, u8, u8)]) -> impl Iterator<Item = Variant> + '_ {
    ladder
        .iter()
        .enumerate()
        .flat_map(|(candidate, &(_, scale, quant))| {
            let edge = ccso_quant_step(scale, quant) != 0;
            (0..4u8).flat_map(move |band_log2| {
                let variant = |bo_only, edge_clf| Variant {
                    candidate,
                    bo_only,
                    edge_clf,
                    band_log2,
                };
                [
                    (candidate == 0).then(|| variant(true, false)),
                    Some(variant(false, false)),
                    edge.then(|| variant(false, true)),
                ]
                .into_iter()
                .flatten()
            })
        })
}

/// The cost, parameters and unit flags of `variant` on `plane`.
fn evaluate(
    input: &CcsoInput<'_>,
    (plane, cdfs, grid_cols): (usize, &[Vec<u16>], usize),
    stats: &[CcsoStats],
    ladder: &[(u8, u8, u8)],
    variant: Variant,
) -> Result<PlaneChoice> {
    let (ext_filter, scale, quant) = ladder[variant.candidate];
    let classes = variant.classes();
    let merged: Vec<Vec<(i64, u32)>> = stats
        .iter()
        .map(|unit| {
            let mut merged = vec![(0, 0); classes];
            for class in 0..CLASSES {
                let target = &mut merged[variant.class(class)];
                target.0 += unit.sum[class];
                target.1 += unit.count[class];
            }
            merged
        })
        .collect();
    let mut on = vec![true; merged.len()];
    let mut result = None;
    for _ in 0..2 {
        let mut totals = vec![(0i64, 0u32); classes];
        for unit in merged
            .iter()
            .zip(&on)
            .filter_map(|(unit, &on)| on.then_some(unit))
        {
            for (total, &(sum, count)) in totals.iter_mut().zip(unit) {
                total.0 += sum;
                total.1 += count;
            }
        }
        let offsets: Vec<(u8, i32)> = totals
            .iter()
            .map(|&(sum, count)| {
                (0..OFFSET_INDICES)
                    .filter_map(|index| Some((index, ccso_offset(index, scale)?)))
                    .min_by_key(|&(index, offset)| {
                        rd(
                            offset_delta(sum, count, offset),
                            unary_cost(u32::from(index), 7),
                            input.lambda,
                        )
                    })
                    .unwrap_or((0, 0))
            })
            .collect();
        let deltas: Vec<i64> = merged
            .iter()
            .map(|unit| {
                unit.iter()
                    .zip(&offsets)
                    .map(|(&(sum, count), &(_, offset))| offset_delta(sum, count, offset))
                    .sum()
            })
            .collect();
        let (flags, flag_bits) = decide(input, (plane, cdfs, grid_cols), &deltas)?;
        let header = header_bits(input, variant, ladder, &offsets);
        let distortion = deltas
            .iter()
            .zip(&flags)
            .filter_map(|(&delta, &on)| on.then_some(delta))
            .sum();
        let cost = rd(distortion, header + flag_bits, input.lambda);
        on = flags;
        result = Some((cost, offsets));
    }
    let (cost, offsets) = result.unwrap_or((i128::MAX, Vec::new()));
    let ccso_offset_idx = CcsoOffsets::from_iter_checked(offsets.iter().map(|&(index, _)| index))
        .ok_or(Error::Unsupported {
        what: "more CCSO offsets than a plane codes",
    })?;
    let coded = |value: u8| Some(if variant.bo_only { 0 } else { value });
    Ok(PlaneChoice {
        params: CcsoPlaneParams {
            ccso_planes: true,
            ccso_bo_only: Some(variant.bo_only),
            ccso_scale_idx: Some(scale),
            ccso_quant_idx: coded(quant),
            ccso_ext_filter: coded(ext_filter),
            ccso_edge_clf: Some(variant.edge_clf),
            ccso_max_band_log2: Some(variant.band_log2),
            ccso_offset_idx,
            ..CcsoPlaneParams::default()
        },
        on,
        cost,
    })
}

/// The § 5.18.7.12 bits of a plane coded with `variant` and `offsets`.
fn header_bits(
    input: &CcsoInput<'_>,
    variant: Variant,
    ladder: &[(u8, u8, u8)],
    offsets: &[(u8, i32)],
) -> Bits {
    let (_, scale, quant) = ladder[variant.candidate];
    let reuse = if input.core.frame_is_intra == Some(true) {
        0
    } else {
        2
    };
    let fields = if variant.bo_only {
        1 + 2 + 3
    } else {
        1 + 2 + 2 + 3 + u32::from(ccso_quant_step(scale, quant) != 0) + 2
    };
    Bits::whole(1 + reuse + fields)
        + offsets
            .iter()
            .map(|&(index, _)| unary_cost(u32::from(index), 7))
            .sum()
}

/// Turns each unit of `plane` on when its squared-error change `deltas`
/// pays for the `ccso_blk` rate of the CDF as it adapts in coding order.
/// Returns the flags in raster order and their bits.
fn decide(
    input: &CcsoInput<'_>,
    (plane, cdfs, grid_cols): (usize, &[Vec<u16>], usize),
    deltas: &[i64],
) -> Result<(Vec<bool>, Bits)> {
    let mut on = vec![false; deltas.len()];
    let mut bits = Bits::default();
    for tile in 0..tile_count(input.core) {
        let mut map = CcsoBlockMap::new(input.seq, input.core, tile).map_err(coding("ccso map"))?;
        let mut rows = cdfs.to_vec();
        for unit in map.units() {
            let (Some(ctx), Some(&delta)) = (
                map.ctx(plane, unit),
                deltas.get(unit.0 * grid_cols + unit.1),
            ) else {
                continue;
            };
            let row = rows.get_mut(ctx).ok_or(Error::Unsupported {
                what: "a ccso_blk context without a CDF",
            })?;
            let (off, on_bits) = (symbol_cost(row, 0), symbol_cost(row, 1));
            let take = rd(delta, on_bits, input.lambda) < rd(0, off, input.lambda);
            adapt_cdf_u16(row, Symbol::new(u8::from(take))).map_err(coding("ccso_blk CDF"))?;
            map.set(plane, unit, take).map_err(coding("ccso map"))?;
            on[unit.0 * grid_cols + unit.1] = take;
            bits += if take { on_bits } else { off };
        }
    }
    Ok((on, bits))
}

/// The statistics of unit row `row`, indexed
/// `(plane * ladder.len() + candidate) * grid.cols + unit column`.
fn unit_row_stats(
    input: &CcsoInput<'_>,
    grid: CcsoGrid,
    row: usize,
    planes: usize,
    ladder: &[(u8, u8, u8)],
) -> Result<Vec<CcsoStats>> {
    let missing = || Error::Unsupported {
        what: "a CCSO plane the frame does not have",
    };
    let luma = input.deblocked.plane(PlaneId::Y).ok_or_else(missing)?;
    let (luma_width, luma_height) = (luma.storage_size().width(), luma.storage_size().height());
    let luma_stride = luma.stride_samples();
    let format = input.filtered.info().pixel_format();
    let band_shift = u32::from(input.filtered.bit_depth().bits()) - 3;
    let taps: Vec<[(isize, isize); 2]> = ladder
        .iter()
        .map(|&(ext_filter, ..)| ccso_taps(ext_filter))
        .collect::<core::result::Result<_, _>>()
        .map_err(coding("ccso taps"))?;
    let mut stats = vec![CcsoStats::default(); planes * ladder.len() * grid.cols];
    let mut gathered = [Vec::new(), Vec::new(), Vec::new()];
    for (plane, id) in [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .enumerate()
        .take(planes)
    {
        let (sub_x, sub_y) = if plane == 0 {
            (0, 0)
        } else {
            (
                usize::from(format.subsampling_x()),
                usize::from(format.subsampling_y()),
            )
        };
        let size = input.source.plane_sizes()[plane];
        let source = &input.source.planes()[plane];
        let filtered = input.filtered.plane(id).ok_or_else(missing)?;
        let unit_x = |col: usize| ((col << grid.luma_size_log2) >> sub_x).min(size.width());
        let top = (row << grid.luma_size_log2) >> sub_y;
        let bottom = (((row + 1) << grid.luma_size_log2) >> sub_y).min(size.height());
        for y in top..bottom {
            let y_luma = (y << sub_y) as isize;
            let luma_at = |dx: isize, dy: isize, x: usize| {
                let ly = (y_luma + dy).clamp(0, luma_height as isize - 1) as usize;
                let lx = (((x << sub_x) as isize) + dx).clamp(0, luma_width as isize - 1) as usize;
                luma.samples()
                    .get(ly * luma_stride + lx)
                    .copied()
                    .unwrap_or(0)
            };
            gathered[0].clear();
            gathered[0].extend((0..size.width()).map(|x| luma_at(0, 0, x)));
            let stride = filtered.stride_samples();
            let (Some(source_row), Some(filtered_row)) = (
                source.get(y * size.width()..(y + 1) * size.width()),
                filtered
                    .samples()
                    .get(y * stride..y * stride + size.width()),
            ) else {
                return Err(missing());
            };
            for (candidate, (&(_, scale, quant), taps)) in ladder.iter().zip(&taps).enumerate() {
                for (tap, &(dx, dy)) in taps.iter().enumerate() {
                    gathered[tap + 1].clear();
                    gathered[tap + 1].extend((0..size.width()).map(|x| luma_at(dx, dy, x)));
                }
                let quant_step = ccso_quant_step(scale, quant);
                let base = (plane * ladder.len() + candidate) * grid.cols;
                for col in 0..grid.cols {
                    let span = unit_x(col)..unit_x(col + 1);
                    accumulate_simd(
                        CcsoRow {
                            center: &gathered[0][span.clone()],
                            tap0: &gathered[1][span.clone()],
                            tap1: &gathered[2][span.clone()],
                            source: &source_row[span.clone()],
                            filtered: &filtered_row[span],
                        },
                        quant_step,
                        band_shift,
                        &mut stats[base + col],
                    );
                }
            }
        }
    }
    Ok(stats)
}

/// The frame sums and counts of the coded classes of `plane` under `params`.
#[cfg(test)]
pub(super) fn class_sums(
    input: &CcsoInput<'_>,
    plane: usize,
    params: &CcsoPlaneParams,
) -> Result<Vec<(i64, u32)>> {
    let grid = CcsoGrid::new(input.seq, input.core).map_err(coding("ccso grid"))?;
    let entry = (
        params.ccso_ext_filter.unwrap_or(0),
        params.ccso_scale_idx.unwrap_or(0),
        params.ccso_quant_idx.unwrap_or(0),
    );
    let variant = Variant {
        candidate: 0,
        bo_only: params.ccso_bo_only == Some(true),
        edge_clf: params.ccso_edge_clf == Some(true),
        band_log2: params.ccso_max_band_log2.unwrap_or(0),
    };
    let mut sums = vec![(0, 0); variant.classes()];
    for row in 0..grid.rows {
        let stats = unit_row_stats(input, grid, row, plane + 1, &[entry])?;
        for unit in &stats[plane * grid.cols..] {
            for class in 0..CLASSES {
                let target = &mut sums[variant.class(class)];
                target.0 += unit.sum[class];
                target.1 += unit.count[class];
            }
        }
    }
    Ok(sums)
}
