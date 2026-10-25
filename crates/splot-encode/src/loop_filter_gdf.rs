// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The GDF choice of a frame: `gdf_pic_qc_idx`, `gdf_pic_scale_idx`
//! (§ 5.18.7.9) and the `use_gdf` flags (§ 5.20.10.3).
//!
//! GDF runs last in the filter chain and changes luma only, from inputs the
//! flags do not change (§ 7.20.5). So one run with every unit on gives the
//! exact squared-error change of each unit, and the flags follow by RD.

use splot_core::headers::frame::{FrameFilterParams, FrameGdf, FrameHeaderCore};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::adapt_cdf_u16;
use splot_decode::encoder_syntax::{
    GdfBlockMap, GdfFlags, GdfGrid, LrUnit, LrUnitType, apply_loop_filters, tile_count,
};
use splot_recon::DecodedFrame;

use super::ccso::rd;
use super::{FilterSyntax, Search, coding, plane_sse};
use crate::error::{Error, Result};
use crate::frame::SourceFrame;
use crate::intra_frame::map_tiles;
use crate::rd::{Bits, symbol_cost};

/// The 8-bit-equivalent `base_q_idx` from which frames try
/// `gdf_pic_scale_idx` 2 and 3 first; below the first, 1.
const SCALE_QINDEX: [u32; 2] = [125, 155];

/// Every `(gdf_pic_qc_idx, gdf_pic_scale_idx)` in the order a frame of
/// `qindex` at `bit_depth` tries them: the largest `gdf_pic_qc_idx` with the
/// scale index of [`SCALE_QINDEX`] first, then by distance from it.
pub(super) fn candidates(qindex: u32, bit_depth: u32) -> [(u8, u8); 16] {
    let q8 = qindex.saturating_sub(24 * bit_depth.saturating_sub(8));
    let scale = 1 + SCALE_QINDEX.iter().filter(|&&q| q8 >= q).count() as u8;
    let mut pairs: [(u8, u8); 16] = core::array::from_fn(|i| (i as u8 / 4, i as u8 % 4));
    pairs.sort_by_key(|&(qc, s)| (3 - qc + s.abs_diff(scale), 3 - qc));
    pairs
}

/// `gdf_pic_qc_idx` and `gdf_pic_scale_idx` bits; `gdf_per_block` adds one
/// where a grid codes it.
const HEADER_BITS: u32 = 4;

impl Search<'_> {
    /// The GDF choice on top of `params` and `syntax`, from the first
    /// `candidates` entries of [`candidates`]; `None` keeps GDF off.
    pub(super) fn gdf(
        &self,
        params: &FrameFilterParams,
        syntax: &FilterSyntax,
        count: usize,
    ) -> Result<Option<(FrameGdf, Vec<u8>)>> {
        let lr_on: Vec<(LrUnit, LrUnitType)> = syntax.lr_on.iter().map(|(&u, &t)| (u, t)).collect();
        let run = |core: &FrameHeaderCore, blocks: &[u8]| {
            let gdf = GdfFlags {
                blocks,
                refs: self.refs,
            };
            apply_loop_filters(
                self.seq,
                core,
                self.recorder,
                self.recon,
                (&syntax.cdef_indices, &syntax.ccso_blocks, Some(gdf)),
                &lr_on,
            )
            .map_err(coding("loop filters"))
        };
        let off = run(&self.header(params)?, &[])?;
        let order = candidates(self.qindex, u32::from(self.recon.bit_depth().bits()));
        let headers = order[..count.min(order.len())]
            .iter()
            .map(|&(qc_idx, scale_idx)| {
                let gdf = FrameGdf { qc_idx, scale_idx };
                Ok((
                    gdf,
                    self.header(&FrameFilterParams {
                        gdf: Some(gdf),
                        ..*params
                    })?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let Some((_, first)) = headers.first() else {
            return Ok(None);
        };
        let grid = GdfGrid::new(self.seq, first).map_err(coding("gdf grid"))?;
        let all_on = grid.map_or_else(Vec::new, |grid| vec![1u8; grid.rows * grid.cols]);
        let (source, height) = (self.source, self.recon.coded_luma_size().height());
        let tried = map_tiles(self.pool, headers, |(gdf, core)| {
            let on = run(&core, &all_on)?;
            let deltas = unit_deltas(source, (&core, grid, height), (&on, &off))?;
            Ok((gdf, core, deltas))
        })?;
        let mut best: Option<(i128, FrameGdf, Vec<u8>)> = None;
        for (gdf, core, deltas) in tried {
            let (flags, cost) = match grid {
                Some(grid) => self.gdf_flags(&core, grid, &deltas)?,
                None => (
                    Vec::new(),
                    rd(deltas.iter().sum(), Bits::default(), self.lambda),
                ),
            };
            let header = HEADER_BITS + u32::from(grid.is_some());
            let cost = cost + rd(0, Bits::whole(header), self.lambda);
            if cost < best.as_ref().map_or(0, |best| best.0) {
                best = Some((cost, gdf, flags));
            }
        }
        Ok(best.map(|(_, gdf, flags)| (gdf, flags)))
    }

    /// Turns each unit on when its squared-error change `deltas` pays for
    /// the `use_gdf` rate of the CDF as it adapts in coding order. Returns
    /// the flags in raster order and their RD cost.
    fn gdf_flags(
        &self,
        core: &FrameHeaderCore,
        grid: GdfGrid,
        deltas: &[i64],
    ) -> Result<(Vec<u8>, i128)> {
        let start = self.start.use_gdf_cdf().map_err(coding("use_gdf CDF"))?;
        let mut flags = vec![0u8; deltas.len()];
        let mut cost = 0;
        for tile in 0..tile_count(core) {
            let map = GdfBlockMap::new(self.seq, core, tile).map_err(coding("gdf map"))?;
            let mut row = start.clone();
            for (unit_row, unit_col) in map.units() {
                let index = unit_row * grid.cols + unit_col;
                let delta = *deltas.get(index).ok_or(Error::Unsupported {
                    what: "a GDF unit outside the grid",
                })?;
                let (off, on) = (
                    rd(0, symbol_cost(&row, 0), self.lambda),
                    rd(delta, symbol_cost(&row, 1), self.lambda),
                );
                let take = on < off;
                adapt_cdf_u16(&mut row, Symbol::new(u8::from(take)))
                    .map_err(coding("use_gdf CDF"))?;
                flags[index] = u8::from(take);
                cost += on.min(off);
            }
        }
        Ok((flags, cost))
    }
}

/// The luma squared-error change of GDF per unit of `grid` in raster order
/// (one unit without a grid), `on` against `off`; luma rows follow the unit
/// that governs their stripe in a frame `height` coded rows high.
fn unit_deltas(
    source: &SourceFrame,
    (core, grid, height): (&FrameHeaderCore, Option<GdfGrid>, usize),
    (on, off): (&DecodedFrame<u16>, &DecodedFrame<u16>),
) -> Result<Vec<i64>> {
    let delta =
        |rect| plane_sse(source, on, 0, rect) as i64 - plane_sse(source, off, 0, rect) as i64;
    let Some(grid) = grid else {
        return Ok(vec![delta((0, 0, usize::MAX, usize::MAX))]);
    };
    let mut deltas = vec![0i64; grid.rows * grid.cols];
    for y in 0..source.plane_sizes()[0].height() {
        let row = grid.unit_row(core, y, height).map_err(coding("gdf grid"))?;
        for col in 0..grid.cols {
            deltas[row * grid.cols + col] += delta((col * grid.block_size, y, grid.block_size, 1));
        }
    }
    Ok(deltas)
}
