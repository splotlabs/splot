// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma loop restoration by rate and distortion: the frame Wiener-NS filter
//! (§ 5.20.10.6, § 7.20.3) and the type of each unit (§ 5.20.10.5).
//!
//! One pass over the CDEF and CCSO output keeps the normal equations of each
//! unit (one worker-pool job per unit). The filter is their least-squares
//! solution, rounded to the coded precision and clipped to the coded range,
//! for the coefficient subset that minimizes the predicted squared error plus
//! the bank bits. A later pass refits on the units that take the filter. The
//! decoder filters the frame with each candidate; each unit then takes `none`,
//! PC-Wiener or Wiener-NS with the exact rate of the adapting CDFs, and the
//! frame takes the plane type and bank of the lowest total cost. An inter
//! frame also tries the bank of its primary reference (`temporal_pred_flag`).

use std::collections::BTreeMap;

use splot_core::headers::frame::{
    CoreSeqRestorationView, CoreSeqView, FrameFilterParams, FrameHeaderCore, FrameRestorationType,
    FrameWienerNs, WienerNsCoeffs, WienerNsFrameFilterBank, WienerNsFrameFilterClass,
    wiener_ns_coeff_range, wiener_ns_tap_present, write_frame_wiener_ns_filter,
};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::adapt_cdf_u16;
use splot_core::write::BitWriter;
use splot_decode::encoder_syntax::{LrLayout, LrSymbol, LrUnit, LrUnitType, apply_loop_filters};
use splot_recon::{DecodedFrame, PlaneId};

use super::ccso::rd;
use super::{Search, coding};
use crate::error::{Error, Result};
use crate::intra_frame::map_tiles;
use crate::kernels::wiener::{COEFFS, RADIUS, WienerRow, WienerStats, accumulate_simd};
use crate::preset::PresetParams;
use crate::rd::{Bits, symbol_cost};
use crate::tile_writer::filters::{FilterSyntax, lr_unit_symbols};

/// `WIENER_NS_PREC_BITS`.
const PREC: f64 = 128.0;
/// The luma `wiener_ns_length` subsets.
const SUBSETS: usize = 4;

/// One frame-level choice: its cost relative to restoration off, its filter
/// parameters and the type of each restoring unit.
struct Choice {
    cost: i128,
    params: FrameFilterParams,
    lr_on: BTreeMap<LrUnit, LrUnitType>,
}

/// The luma units of a frame in coding order, with the layout of their tile.
struct Units<'a> {
    layouts: &'a [LrLayout],
    units: Vec<(usize, LrUnit)>,
}

impl Search<'_> {
    /// The loop restoration of `pc` (luma PC-Wiener at its unit size) or of a
    /// Wiener-NS plane, whichever costs least, with the CDEF and CCSO choices
    /// of `syntax`; restoration off returns no units.
    pub(super) fn restoration_rd(
        &self,
        pc: &FrameFilterParams,
        syntax: &FilterSyntax,
        preset: &PresetParams,
    ) -> Result<(FrameFilterParams, BTreeMap<LrUnit, LrUnitType>)> {
        let core = self.header(pc)?;
        let layouts = LrLayout::tiles(self.seq, &core).map_err(coding("restoration layout"))?;
        let mut units = Units {
            layouts: &layouts,
            units: Vec::new(),
        };
        for (index, layout) in layouts.iter().enumerate() {
            for unit in layout.all_units().map_err(coding("restoration layout"))? {
                if unit.plane == 0 {
                    units.units.push((index, unit));
                }
            }
        }
        let run = |core: &FrameHeaderCore, kind: Option<LrUnitType>| {
            let lr_on: Vec<_> = kind
                .map(|kind| units.units.iter().map(|&(_, unit)| (unit, kind)).collect())
                .unwrap_or_default();
            apply_loop_filters(
                self.seq,
                core,
                self.recorder,
                self.recon,
                (&syntax.cdef_indices, &syntax.ccso_blocks),
                &lr_on,
            )
            .map_err(coding("loop filters"))
        };
        let off = run(&core, None)?;
        let off_sse = self.unit_sse(&off, &units)?;
        let pc_sse = self.unit_sse(&run(&core, Some(LrUnitType::PcWiener))?, &units)?;
        let off_bits = self
            .header(&FrameFilterParams {
                lr_luma_unit_size: None,
                ..*pc
            })?
            .consumed_bits;
        let sse = [Some(&pc_sse), None];
        let mut best = Choice {
            cost: 0,
            params: *pc,
            lr_on: BTreeMap::new(),
        };
        let mut consider = |choice: Choice| {
            if choice.cost < best.cost {
                best = choice;
            }
        };
        consider(self.decide(pc, &core, (&units, &off_sse, sse), off_bits)?);
        let wiener = |rst_ref_pic_idx, bank, switchable| FrameFilterParams {
            lr_luma_wiener_ns: Some(FrameWienerNs {
                switchable,
                rst_ref_pic_idx,
                bank,
            }),
            ..*pc
        };
        let mut try_bank = |rst_ref_pic_idx, bank: WienerNsFrameFilterBank| -> Result<Vec<bool>> {
            let params = wiener(rst_ref_pic_idx, bank, true);
            let core = self.header(&params)?;
            let wns_sse = self.unit_sse(&run(&core, Some(LrUnitType::WienerNs))?, &units)?;
            let switchable = self.decide(
                &params,
                &core,
                (&units, &off_sse, [Some(&pc_sse), Some(&wns_sse)]),
                off_bits,
            )?;
            let taken = units
                .units
                .iter()
                .map(|(_, unit)| switchable.lr_on.get(unit) == Some(&LrUnitType::WienerNs))
                .collect();
            consider(switchable);
            let params = wiener(rst_ref_pic_idx, bank, false);
            let core = self.header(&params)?;
            consider(self.decide(
                &params,
                &core,
                (&units, &off_sse, [None, Some(&wns_sse)]),
                off_bits,
            )?);
            Ok(taken)
        };
        if let Some(index) = self.temporal_bank(&core) {
            try_bank(
                Some(index),
                WienerNsFrameFilterBank {
                    classes: splot_core::tile::InlineVec::default(),
                },
            )?;
        }
        let stats = self.wiener_stats(&off, &units)?;
        let mut taken = vec![true; stats.len()];
        for _ in 0..preset.wiener_ns {
            let mut total = WienerStats::default();
            for (unit, _) in stats.iter().zip(&taken).filter(|&(_, &on)| on) {
                total.add(unit);
            }
            let Some(bank) = self.fit(&total)? else {
                break;
            };
            taken = try_bank(None, bank)?;
            if !taken.contains(&true) {
                break;
            }
        }
        Ok((best.params, best.lr_on))
    }

    /// `rst_ref_pic_idx` of the primary reference when it holds luma frame filters.
    fn temporal_bank(&self, core: &FrameHeaderCore) -> Option<u8> {
        let inter = core.inter.as_ref()?;
        let position = inter.primary_ref_frame?;
        if u32::from(position) >= inter.num_total_refs? {
            return None;
        }
        let slot = *inter.ref_frame_idx.get(usize::from(position))? as usize;
        let counts = self.refs.header_view().lr_frame_filter_class_counts?;
        (counts.get(slot)?[0] > 0).then_some(position)
    }

    /// The units' squared error in `frame`, in [`Units`] order.
    fn unit_sse(&self, frame: &DecodedFrame<u16>, units: &Units<'_>) -> Result<Vec<u64>> {
        units
            .units
            .iter()
            .map(|&(layout, unit)| {
                let rects = units.layouts[layout]
                    .unit_rects(unit)
                    .map_err(coding("restoration unit"))?;
                Ok(rects
                    .iter()
                    .map(|r| self.sse(frame, 0, (r.x(), r.y(), r.width(), r.height())))
                    .sum())
            })
            .collect()
    }

    /// The normal equations of each unit on the visible luma of `off`.
    fn wiener_stats(&self, off: &DecodedFrame<u16>, units: &Units<'_>) -> Result<Vec<WienerStats>> {
        let missing = || Error::Unsupported {
            what: "a frame without luma",
        };
        let luma = off.plane(PlaneId::Y).ok_or_else(missing)?;
        let coded = off.coded_luma_size();
        let (width, height) = (coded.width(), coded.height());
        let stride = luma.stride_samples();
        let padded_width = width + 2 * RADIUS;
        let mut padded = Vec::with_capacity(padded_width * (height + 2 * RADIUS));
        for y in 0..height + 2 * RADIUS {
            let row = y.saturating_sub(RADIUS).min(height - 1) * stride;
            let samples = luma.samples().get(row..row + width).ok_or_else(missing)?;
            padded.extend(core::iter::repeat_n(samples[0], RADIUS));
            padded.extend_from_slice(samples);
            padded.extend(core::iter::repeat_n(samples[width - 1], RADIUS));
        }
        let size = self.source.plane_sizes()[0];
        let source = &self.source.planes()[0];
        map_tiles(self.pool, units.units.clone(), |(layout, unit)| {
            let mut stats = WienerStats::default();
            for rect in units.layouts[layout]
                .unit_rects(unit)
                .map_err(coding("restoration unit"))?
            {
                let right = (rect.x() + rect.width()).min(size.width());
                let bottom = (rect.y() + rect.height()).min(size.height());
                if right <= rect.x() {
                    continue;
                }
                for y in rect.y()..bottom {
                    let rows = core::array::from_fn(|i| {
                        let start = (y + i) * padded_width + rect.x();
                        &padded[start..start + right - rect.x() + 2 * RADIUS]
                    });
                    let start = y * size.width();
                    accumulate_simd(
                        WienerRow {
                            rows,
                            source: &source[start + rect.x()..start + right],
                        },
                        &mut stats,
                    );
                }
            }
            Ok(stats)
        })
    }

    /// The one-class bank that minimizes the predicted squared error of
    /// `stats` plus its bits, or `None` when no filter lowers the cost.
    fn fit(&self, stats: &WienerStats) -> Result<Option<WienerNsFrameFilterBank>> {
        let view = CoreSeqView::from_sequence(self.seq)
            .ok_or(Error::Unsupported {
                what: "a sequence without a complete view",
            })?
            .restoration();
        let mut best: Option<(i128, WienerNsFrameFilterBank)> = None;
        for subset in 0..SUBSETS {
            let taps: Vec<usize> = (0..COEFFS)
                .filter(|&j| wiener_ns_tap_present(false, subset, j))
                .collect();
            let Some(weights) = solve(stats, &taps) else {
                continue;
            };
            let mut coeffs = [0i16; COEFFS];
            for (&j, &w) in taps.iter().zip(&weights) {
                let (min, max) = wiener_ns_coeff_range(false, j).unwrap_or((0, 0));
                coeffs[j] =
                    ((w * PREC).round() as i64).clamp(i64::from(min), i64::from(max)) as i16;
            }
            refine(stats, &taps, &mut coeffs);
            let bank = one_class_bank(subset, coeffs)?;
            let cost = rd(
                predicted_delta(stats, &coeffs),
                Bits::whole(bank_bits(&bank, view)?),
                self.lambda,
            );
            if cost < 0 && best.as_ref().is_none_or(|(best, _)| cost < *best) {
                best = Some((cost, bank));
            }
        }
        Ok(best.map(|(_, bank)| bank))
    }

    /// Each unit's cheapest type under the plane type of `params` (`core`
    /// its header), with the exact rate of the CDFs as they adapt in coding
    /// order. `sse` holds the PC-Wiener and Wiener-NS errors the plane can
    /// take; `off_bits` are the header bits with restoration off.
    fn decide(
        &self,
        params: &FrameFilterParams,
        core: &FrameHeaderCore,
        (units, off_sse, sse): (&Units<'_>, &[u64], [Option<&Vec<u64>>; 2]),
        off_bits: u64,
    ) -> Result<Choice> {
        let tool = match params.lr_luma_wiener_ns {
            None => FrameRestorationType::PcWiener,
            Some(choice) if choice.switchable => FrameRestorationType::Switchable,
            Some(_) => FrameRestorationType::WienerNonsep,
        };
        let header = core.consumed_bits.saturating_sub(off_bits);
        let mut cost = rd(0, Bits::whole(header as u32), self.lambda);
        let mut lr_on = BTreeMap::new();
        let mut cdfs: BTreeMap<(usize, LrSymbol), Vec<u16>> = BTreeMap::new();
        let kinds = [
            (LrUnitType::PcWiener, sse[0]),
            (LrUnitType::WienerNs, sse[1]),
        ];
        for (index, &(layout, unit)) in units.units.iter().enumerate() {
            let mut pick: Option<(i128, Option<LrUnitType>)> = None;
            let candidates = core::iter::once((None, off_sse[index])).chain(
                kinds
                    .iter()
                    .filter_map(|&(kind, sse)| Some((Some(kind), *sse?.get(index)?))),
            );
            for (kind, unit_sse) in candidates {
                let mut bits = Bits::default();
                for (symbol, value) in lr_unit_symbols(tool, 0, kind).map_err(coding("lr unit"))? {
                    bits += symbol_cost(self.cdf(&mut cdfs, layout, symbol)?, usize::from(value));
                }
                let delta = unit_sse as i64 - off_sse[index] as i64;
                let value = rd(delta, bits, self.lambda);
                if pick.is_none_or(|(best, _)| value < best) {
                    pick = Some((value, kind));
                }
            }
            let Some((value, kind)) = pick else {
                continue;
            };
            for (symbol, on) in lr_unit_symbols(tool, 0, kind).map_err(coding("lr unit"))? {
                adapt_cdf_u16(
                    self.cdf(&mut cdfs, layout, symbol)?,
                    Symbol::new(u8::from(on)),
                )
                .map_err(coding("lr CDF"))?;
            }
            cost += value;
            if let Some(kind) = kind {
                lr_on.insert(unit, kind);
            }
        }
        Ok(Choice {
            cost,
            params: *params,
            lr_on,
        })
    }

    /// The CDF row of `symbol` in tile `tile`, starting from the frame's CDFs.
    fn cdf<'c>(
        &self,
        cdfs: &'c mut BTreeMap<(usize, LrSymbol), Vec<u16>>,
        tile: usize,
        symbol: LrSymbol,
    ) -> Result<&'c mut [u16]> {
        if let std::collections::btree_map::Entry::Vacant(entry) = cdfs.entry((tile, symbol)) {
            entry.insert(self.start.lr_symbol_cdf(symbol).map_err(coding("lr CDF"))?);
        }
        cdfs.get_mut(&(tile, symbol))
            .map(Vec::as_mut_slice)
            .ok_or(Error::Unsupported { what: "an lr CDF" })
    }
}

/// The index of `A[i][j]`, `i <= j`, in [`WienerStats::a`].
const fn pair(i: usize, j: usize) -> usize {
    i * COEFFS - i * (i.saturating_sub(1)) / 2 + j - i
}

fn a_at(stats: &WienerStats, i: usize, j: usize) -> f64 {
    stats.a[pair(i.min(j), i.max(j))] as f64
}

/// The least-squares weights of the coefficients `taps`, by a Cholesky
/// factorization with a small ridge, or `None` for a singular system.
fn solve(stats: &WienerStats, taps: &[usize]) -> Option<Vec<f64>> {
    let n = taps.len();
    let trace: f64 = taps.iter().map(|&j| a_at(stats, j, j)).sum();
    if trace <= 0.0 {
        return None;
    }
    let ridge = trace / n as f64 * 1e-6;
    let mut l = vec![0.0; n * n];
    for r in 0..n {
        for c in 0..=r {
            let mut v = a_at(stats, taps[r], taps[c]) + if r == c { ridge } else { 0.0 };
            for k in 0..c {
                v -= l[r * n + k] * l[c * n + k];
            }
            if r == c {
                if v <= 0.0 {
                    return None;
                }
                l[r * n + r] = v.sqrt();
            } else {
                l[r * n + c] = v / l[c * n + c];
            }
        }
    }
    let mut y = vec![0.0; n];
    for r in 0..n {
        let s: f64 = (0..r).map(|k| l[r * n + k] * y[k]).sum();
        y[r] = (stats.b[taps[r]] as f64 - s) / l[r * n + r];
    }
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| l[k * n + r] * y[k]).sum();
        y[r] = (y[r] - s) / l[r * n + r];
    }
    Some(y)
}

/// Moves single coefficients of `taps` by one step while that lowers the
/// predicted squared error, within the coded ranges.
fn refine(stats: &WienerStats, taps: &[usize], coeffs: &mut [i16; COEFFS]) {
    for _ in 0..8 * COEFFS {
        let mut best = (0.0, 0, 0i16);
        for &k in taps {
            let (min, max) = wiener_ns_coeff_range(false, k).unwrap_or((0, 0));
            let row: f64 = (0..COEFFS)
                .map(|j| f64::from(coeffs[j]) * a_at(stats, k, j))
                .sum();
            for step in [-1i16, 1] {
                let next = coeffs[k] + step;
                if next < min || next > max {
                    continue;
                }
                let d = f64::from(step);
                let change = (2.0 * d * row + d * d * a_at(stats, k, k)) / (PREC * PREC)
                    - 2.0 * d * stats.b[k] as f64 / PREC;
                if change < best.0 {
                    best = (change, k, step);
                }
            }
        }
        if best.2 == 0 {
            return;
        }
        coeffs[best.1] += best.2;
    }
}

/// The predicted squared-error change of filtering the samples of `stats`
/// with `coeffs`: `c^T A c / 2^14 - 2 c.b / 2^7`, plus the `1/12` per sample
/// that rounding the output to integers adds.
fn predicted_delta(stats: &WienerStats, coeffs: &[i16; COEFFS]) -> i64 {
    if coeffs.iter().all(|&c| c == 0) {
        return 0;
    }
    let mut quadratic = 0.0;
    let mut linear = 0.0;
    for i in 0..COEFFS {
        let ci = f64::from(coeffs[i]);
        linear += ci * stats.b[i] as f64;
        for (j, &cj) in coeffs.iter().enumerate() {
            quadratic += ci * f64::from(cj) * a_at(stats, i, j);
        }
    }
    (quadratic / (PREC * PREC) - 2.0 * linear / PREC + stats.samples as f64 / 12.0).round() as i64
}

fn one_class_bank(subset: usize, coeffs: [i16; COEFFS]) -> Result<WienerNsFrameFilterBank> {
    let class = WienerNsFrameFilterClass {
        match_index: 0,
        merged: false,
        ref_bank: 0,
        subset: Some(subset as u8),
        wiener_ns_uv_sym: false,
        coeffs: WienerNsCoeffs::from_iter_checked(coeffs).unwrap_or_default(),
    };
    Ok(WienerNsFrameFilterBank {
        classes: splot_core::tile::InlineVec::from_iter_checked([class]).ok_or(
            Error::Unsupported {
                what: "a Wiener-NS bank",
            },
        )?,
    })
}

/// The bits of `bank` coded without reference filters.
fn bank_bits(bank: &WienerNsFrameFilterBank, view: CoreSeqRestorationView) -> Result<u32> {
    let mut writer = BitWriter::new();
    write_frame_wiener_ns_filter(&mut writer, 0, (0, &[]), view, bank)
        .map_err(coding("Wiener-NS bank"))?;
    Ok(writer.bit_len() as u32)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The solve recovers the weights of a target built from the features,
    /// and the predicted change of those coefficients matches a direct sum.
    #[test]
    fn solve_recovers_a_known_filter() {
        let mut state = 0x9e37_79b9u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            i64::from(state % 200) - 100
        };
        let truth: [i16; COEFFS] = [9, -3, 4, 0, -7, 2, 1, 0, -1, 2, 0, 0, 3, -2, 0, 1];
        let mut stats = WienerStats::default();
        let mut direct = 0i64;
        for _ in 0..4000 {
            let f: [i64; COEFFS] = core::array::from_fn(|_| next());
            let t = (f
                .iter()
                .zip(&truth)
                .map(|(f, &c)| f * i64::from(c))
                .sum::<i64>()
                + 64)
                .div_euclid(128);
            for i in 0..COEFFS {
                stats.b[i] += f[i] * t;
                for j in i..COEFFS {
                    stats.a[pair(i, j)] += f[i] * f[j];
                }
            }
            let fitted = f
                .iter()
                .zip(&truth)
                .map(|(f, &c)| f * i64::from(c))
                .sum::<i64>();
            direct += (t * 128 - fitted).pow(2) - (t * 128).pow(2);
        }
        let taps: Vec<usize> = (0..COEFFS).collect();
        let weights = solve(&stats, &taps).unwrap();
        for (w, &c) in weights.iter().zip(&truth) {
            assert!((w * PREC - f64::from(c)).abs() < 0.3, "{} vs {c}", w * PREC);
        }
        let predicted = predicted_delta(&stats, &truth) as f64;
        assert!((predicted - direct as f64 / 16384.0).abs() <= 1.0 + predicted.abs() * 1e-9);
    }
}
