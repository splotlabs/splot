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
    CdefStrengthSets, CoreSeqRestorationView, CoreSeqView, FrameFilterParams, FrameHeaderCore,
    FrameRestorationType, FrameWienerNs, WienerNsCoeffs, WienerNsFrameFilterBank,
    WienerNsFrameFilterClass, wiener_ns_coeff_range, wiener_ns_tap_present,
    write_frame_wiener_ns_filter,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::adapt_cdf_u16;
use splot_core::write::BitWriter;
use splot_decode::encoder_syntax::{
    LrLayout, LrSymbol, LrUnit, LrUnitType, WIENER_NS_CONFIG_UV, WIENER_NS_CONFIG_Y_PAIRS,
    apply_loop_filters, wiener_ns_chroma_luma_plane,
};
use splot_recon::{DecodedFrame, PlaneId};

use super::ccso::rd;
use super::{Search, coding};
use crate::error::{Error, Result};
use crate::intra_frame::map_tiles;
use crate::kernels::wiener::{WienerAccumulator, WienerStats};
use crate::preset::PresetParams;
use crate::rd::{Bits, symbol_cost};
use crate::tile_writer::filters::{FilterSyntax, lr_unit_symbols};

/// `WIENER_NS_PREC_BITS`.
const PREC: f64 = 128.0;
/// Coefficients of a luma and of a chroma filter.
const LUMA_COEFFS: usize = 16;
const CHROMA_COEFFS: usize = 18;
/// Chroma coefficients that weigh chroma taps (`WIENER_NS_SHORT_COEFFS`).
const CHROMA_SHORT: usize = 6;
/// Rows and columns the luma and the chroma taps reach.
const LUMA_RADIUS: usize = 4;
const CHROMA_RADIUS: usize = 2;

/// One frame-level choice: its cost relative to restoration off, its filter
/// parameters and the type of each restoring unit.
struct Choice {
    cost: i128,
    params: FrameFilterParams,
    lr_on: BTreeMap<LrUnit, LrUnitType>,
}

/// The restoration units of a frame in coding order, with the layout of
/// their tile.
struct Units {
    layouts: Vec<LrLayout>,
    units: Vec<(usize, LrUnit)>,
}

impl Units {
    /// The units every plane of the frame `core` codes.
    fn new(seq: &SequenceHeader, core: &FrameHeaderCore) -> Result<Self> {
        let layouts = LrLayout::tiles(seq, core).map_err(coding("restoration layout"))?;
        let mut units = Vec::new();
        for (index, layout) in layouts.iter().enumerate() {
            for unit in layout.all_units().map_err(coding("restoration layout"))? {
                units.push((index, unit));
            }
        }
        Ok(Self { layouts, units })
    }

    /// The units of `plane` with all of them restoring with `kind`.
    fn all(&self, plane: usize, kind: LrUnitType) -> Vec<(LrUnit, LrUnitType)> {
        self.units
            .iter()
            .filter(|(_, unit)| unit.plane == plane)
            .map(|&(_, unit)| (unit, kind))
            .collect()
    }
}

/// The type candidates of each unit, in [`Units`] order, with their squared
/// error change from restoration off.
type Options = Vec<Vec<(Option<LrUnitType>, i64)>>;

/// A plane padded by clamping, `radius` samples on every side.
struct Padded {
    samples: Vec<u16>,
    width: usize,
    radius: usize,
}

impl Padded {
    /// The `width x height` samples with row stride `stride`.
    fn new(
        samples: &[u16],
        stride: usize,
        (width, height): (usize, usize),
        radius: usize,
    ) -> Result<Self> {
        let mut padded = Vec::with_capacity((width + 2 * radius) * (height + 2 * radius));
        for y in 0..height + 2 * radius {
            let row = y.saturating_sub(radius).min(height - 1) * stride;
            let row = samples.get(row..row + width).ok_or(Error::Unsupported {
                what: "a restoration plane smaller than the frame",
            })?;
            padded.extend(core::iter::repeat_n(row[0], radius));
            padded.extend_from_slice(row);
            padded.extend(core::iter::repeat_n(row[width - 1], radius));
        }
        Ok(Self {
            samples: padded,
            width: width + 2 * radius,
            radius,
        })
    }

    /// The sample at `(x + dx, y + dy)`.
    fn at(&self, x: usize, y: usize, (dy, dx): (isize, isize)) -> i32 {
        let row = (y + self.radius) as isize + dy;
        let col = (x + self.radius) as isize + dx;
        i32::from(self.samples[row as usize * self.width + col as usize])
    }
}

impl Search<'_> {
    /// The loop restoration of `pc` (luma PC-Wiener at its unit size) or of a
    /// Wiener-NS plane, whichever costs least, then the Wiener-NS filter of
    /// each chroma plane, with the CDEF and CCSO choices of `syntax`;
    /// restoration off returns no units. The deblocking, CDEF and CCSO
    /// output is filtered once; each candidate then runs the decoder's
    /// restoration alone on it, which reads that output also for the two rows
    /// past a stripe that the frame chain takes before CDEF.
    pub(super) fn restoration_rd(
        &self,
        pc: &FrameFilterParams,
        syntax: &FilterSyntax,
        preset: &PresetParams,
    ) -> Result<(FrameFilterParams, BTreeMap<LrUnit, LrUnitType>)> {
        let core = self.header(pc)?;
        let units = Units::new(self.seq, &core)?;
        let off = apply_loop_filters(
            self.seq,
            &core,
            self.recorder,
            self.recon,
            (&syntax.cdef_indices, &syntax.ccso_blocks),
            &[],
        )
        .map_err(coding("loop filters"))?;
        let off_bits = self
            .header(&FrameFilterParams {
                lr_luma_unit_size: None,
                ..*pc
            })?
            .consumed_bits;
        let deltas = |params: &FrameFilterParams, units: &Units, plane, kind| -> Result<Vec<i64>> {
            let frame = self.restored(params, &off, &units.all(plane, kind))?;
            let sse = self.unit_sse(&frame, units)?;
            let off = self.unit_sse(&off, units)?;
            Ok(sse
                .iter()
                .zip(&off)
                .map(|(&a, &b)| a as i64 - b as i64)
                .collect())
        };
        let pc_delta = deltas(pc, &units, 0, LrUnitType::PcWiener)?;
        let luma_options = |kinds: &[(LrUnitType, &Vec<i64>)]| -> Options {
            (0..units.units.len())
                .map(|index| {
                    core::iter::once((None, 0))
                        .chain(
                            kinds
                                .iter()
                                .map(|&(kind, delta)| (Some(kind), delta[index])),
                        )
                        .collect()
                })
                .collect()
        };
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
        let (cost, lr_on) = self.decide(
            &core,
            &units,
            &luma_options(&[(LrUnitType::PcWiener, &pc_delta)]),
            off_bits,
        )?;
        consider(Choice {
            cost,
            params: *pc,
            lr_on,
        });
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
            let wns_delta = deltas(&params, &units, 0, LrUnitType::WienerNs)?;
            let options = luma_options(&[
                (LrUnitType::PcWiener, &pc_delta),
                (LrUnitType::WienerNs, &wns_delta),
            ]);
            let (cost, lr_on) = self.decide(&self.header(&params)?, &units, &options, off_bits)?;
            let taken = units
                .units
                .iter()
                .map(|(_, unit)| lr_on.get(unit) == Some(&LrUnitType::WienerNs))
                .collect();
            consider(Choice {
                cost,
                params,
                lr_on,
            });
            let params = wiener(rst_ref_pic_idx, bank, false);
            let options = luma_options(&[(LrUnitType::WienerNs, &wns_delta)]);
            let (cost, lr_on) = self.decide(&self.header(&params)?, &units, &options, off_bits)?;
            consider(Choice {
                cost,
                params,
                lr_on,
            });
            Ok(taken)
        };
        if let Some(index) = self.temporal_bank(&core, 0) {
            try_bank(Some(index), empty_bank())?;
        }
        let stats = self.plane_stats(&off, &units, 0)?;
        let mut taken = vec![true; stats.len()];
        for _ in 0..preset.wiener_ns {
            let Some(bank) = self.fit(&sum_taken(&stats, &taken), 0)? else {
                break;
            };
            taken = try_bank(None, bank)?;
            if !taken.contains(&true) {
                break;
            }
        }
        if preset.wiener_ns_chroma > 0 && off.plane(PlaneId::U).is_some() {
            best = self.chroma(best, &off, preset, off_bits)?;
        }
        Ok((best.params, best.lr_on))
    }

    /// `best` with the cheapest Wiener-NS filter of U and then of V, each
    /// against keeping the plane off; every other unit keeps its type.
    fn chroma(
        &self,
        mut best: Choice,
        off: &DecodedFrame<u16>,
        preset: &PresetParams,
        off_bits: u64,
    ) -> Result<Choice> {
        let format = off.info().pixel_format();
        let shift = format.subsampling_x().max(format.subsampling_y());
        let luma_size = best.params.lr_luma_unit_size.unwrap_or(64);
        if best.lr_on.is_empty() {
            best.params.lr_luma_unit_size = None;
            best.params.lr_luma_wiener_ns = None;
        }
        best.params.lr_chroma_unit_size = Some(luma_size >> shift);
        let probe = FrameFilterParams {
            lr_chroma_wiener_ns: [Some(choice(
                None,
                &one_class_bank(0, false, &[0; CHROMA_COEFFS])?,
            )); 2],
            ..best.params
        };
        let probe_units = Units::new(self.seq, &self.header(&probe)?)?;
        let forced = |units: &Units, lr_on: &BTreeMap<LrUnit, LrUnitType>, plane| -> Options {
            units
                .units
                .iter()
                .map(|(_, unit)| {
                    let kind = lr_on.get(unit).copied();
                    if unit.plane == plane {
                        vec![(None, 0)]
                    } else {
                        vec![(kind, 0)]
                    }
                })
                .collect()
        };
        let base_params = FrameFilterParams {
            lr_chroma_wiener_ns: [None; 2],
            ..best.params
        };
        let base_units = Units::new(self.seq, &self.header(&base_params)?)?;
        let (mut cost, _) = self.decide(
            &self.header(&base_params)?,
            &base_units,
            &forced(&base_units, &best.lr_on, usize::MAX),
            off_bits,
        )?;
        let mut current = Choice {
            cost,
            params: base_params,
            lr_on: best.lr_on.clone(),
        };
        for plane in 1..3 {
            let stats = self.plane_stats(off, &probe_units, plane)?;
            let mut try_bank = |rst_ref_pic_idx, bank| -> Result<Vec<bool>> {
                let mut params = current.params;
                params.lr_chroma_wiener_ns[plane - 1] = Some(choice(rst_ref_pic_idx, &bank));
                let core = self.header(&params)?;
                let units = Units::new(self.seq, &core)?;
                let frame = self.restored(&params, off, &units.all(plane, LrUnitType::WienerNs))?;
                let on = self.unit_sse(&frame, &units)?;
                let before = self.unit_sse(off, &units)?;
                let mut options = forced(&units, &current.lr_on, plane);
                for (index, (_, unit)) in units.units.iter().enumerate() {
                    if unit.plane == plane {
                        let delta = on[index] as i64 - before[index] as i64;
                        options[index].push((Some(LrUnitType::WienerNs), delta));
                    }
                }
                let (value, lr_on) = self.decide(&core, &units, &options, off_bits)?;
                let taken = units
                    .units
                    .iter()
                    .filter(|(_, unit)| unit.plane == plane)
                    .map(|(_, unit)| lr_on.contains_key(unit))
                    .collect();
                if value < cost {
                    cost = value;
                    current = Choice {
                        cost,
                        params,
                        lr_on,
                    };
                }
                Ok(taken)
            };
            if let Some(index) = self.temporal_bank(&self.header(&base_params)?, plane) {
                try_bank(Some(index), empty_bank())?;
            }
            let mut taken = vec![true; stats.len()];
            for _ in 0..preset.wiener_ns_chroma {
                let Some(bank) = self.fit(&sum_taken(&stats, &taken), plane)? else {
                    break;
                };
                taken = try_bank(None, bank)?;
                if !taken.contains(&true) {
                    break;
                }
            }
        }
        if current.params.lr_chroma_wiener_ns == [None; 2] {
            return Ok(best);
        }
        Ok(current)
    }

    /// `off` restored alone by the decoder with the restoration of `params`
    /// on the units of `lr_on`.
    fn restored(
        &self,
        params: &FrameFilterParams,
        off: &DecodedFrame<u16>,
        lr_on: &[(LrUnit, LrUnitType)],
    ) -> Result<DecodedFrame<u16>> {
        let core = self.header(&FrameFilterParams {
            deblocking: [false; 4],
            deblocking_delta_q: [0; 4],
            cdef_strengths: CdefStrengthSets::empty(),
            ccso: Default::default(),
            ..*params
        })?;
        apply_loop_filters(self.seq, &core, self.recorder, off, (&[], &[]), lr_on)
            .map_err(coding("loop filters"))
    }

    /// `rst_ref_pic_idx` of the primary reference when it holds frame
    /// filters of `plane` (either chroma plane for a chroma plane).
    fn temporal_bank(&self, core: &FrameHeaderCore, plane: usize) -> Option<u8> {
        let inter = core.inter.as_ref()?;
        let position = inter.primary_ref_frame?;
        if u32::from(position) >= inter.num_total_refs? {
            return None;
        }
        let slot = *inter.ref_frame_idx.get(usize::from(position))? as usize;
        let counts = self.refs.header_view().lr_frame_filter_class_counts?;
        let counts = counts.get(slot)?;
        let held = if plane == 0 {
            counts[0] > 0
        } else {
            counts[1] > 0 || counts[2] > 0
        };
        held.then_some(position)
    }

    /// The units' squared error in `frame`, in [`Units`] order.
    fn unit_sse(&self, frame: &DecodedFrame<u16>, units: &Units) -> Result<Vec<u64>> {
        units
            .units
            .iter()
            .map(|&(layout, unit)| {
                let rects = units.layouts[layout]
                    .unit_rects(unit)
                    .map_err(coding("restoration unit"))?;
                Ok(rects
                    .iter()
                    .map(|r| self.sse(frame, unit.plane, (r.x(), r.y(), r.width(), r.height())))
                    .sum())
            })
            .collect()
    }

    /// The normal equations of each unit of `plane`, in [`Units`] order, on
    /// the visible samples of `off`. A chroma unit also reads the § 7.20.3
    /// luma samples of its positions.
    fn plane_stats(
        &self,
        off: &DecodedFrame<u16>,
        units: &Units,
        plane: usize,
    ) -> Result<Vec<WienerStats>> {
        let missing = || Error::Unsupported {
            what: "a restoration plane the frame does not have",
        };
        let ids = [PlaneId::Y, PlaneId::U, PlaneId::V];
        let coded = off.coded_luma_size();
        let format = off.info().pixel_format();
        let (sub_x, sub_y) = if plane == 0 {
            (0, 0)
        } else {
            (format.subsampling_x(), format.subsampling_y())
        };
        let size = (
            coded.width().div_ceil(1 << sub_x),
            coded.height().div_ceil(1 << sub_y),
        );
        let samples = off.plane(ids[plane]).ok_or_else(missing)?;
        let radius = if plane == 0 {
            LUMA_RADIUS
        } else {
            CHROMA_RADIUS
        };
        let padded = Padded::new(samples.samples(), samples.stride_samples(), size, radius)?;
        let luma = if plane == 0 {
            None
        } else {
            let luma = off.plane(PlaneId::Y).ok_or_else(missing)?;
            let cfl = self
                .seq
                .intra
                .as_ref()
                .map_or(0, |intra| intra.cfl_ds_filter_index);
            let plane = wiener_ns_chroma_luma_plane(
                (luma.samples(), luma.stride_samples()),
                size,
                (sub_x, sub_y),
                cfl,
                (
                    (coded.width().div_ceil(8) * 8).min(luma.storage_size().width()) - 1,
                    (coded.height().div_ceil(8) * 8).min(luma.storage_size().height()) / 4,
                ),
                off.bit_depth().max_sample(),
            )
            .map_err(coding("restoration luma"))?;
            Some(Padded::new(&plane, size.0, size, CHROMA_RADIUS)?)
        };
        let source_size = self.source.plane_sizes()[plane];
        let source = &self.source.planes()[plane];
        let n = if plane == 0 {
            LUMA_COEFFS
        } else {
            CHROMA_COEFFS
        };
        let max = u64::from(off.bit_depth().max_sample());
        let jobs: Vec<_> = units
            .units
            .iter()
            .filter(|(_, unit)| unit.plane == plane)
            .copied()
            .collect();
        map_tiles(self.pool, jobs, |(layout, unit)| {
            let mut stats = WienerStats::new(n);
            let mut lanes = WienerAccumulator::new(n, 4 * max * max);
            let mut features = vec![Vec::new(); n];
            let mut target = Vec::new();
            for rect in units.layouts[layout]
                .unit_rects(unit)
                .map_err(coding("restoration unit"))?
            {
                let right = (rect.x() + rect.width()).min(source_size.width());
                let bottom = (rect.y() + rect.height()).min(source_size.height());
                for y in rect.y()..bottom {
                    for row in &mut features {
                        row.clear();
                    }
                    target.clear();
                    for x in rect.x()..right {
                        let m = padded.at(x, y, (0, 0));
                        target.push(i32::from(source[y * source_size.width() + x]) - m);
                        if let Some(luma) = &luma {
                            let centre = luma.at(x, y, (0, 0));
                            for (k, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
                                if k % 2 == 0 {
                                    features[k / 2].push(
                                        padded.at(x, y, (dy, dx)) + padded.at(x, y, (-dy, -dx))
                                            - 2 * m,
                                    );
                                }
                                features[CHROMA_SHORT + k].push(luma.at(x, y, (dy, dx)) - centre);
                            }
                        } else {
                            for (k, &(dy, dx, _)) in WIENER_NS_CONFIG_Y_PAIRS.iter().enumerate() {
                                features[k].push(
                                    padded.at(x, y, (dy, dx)) + padded.at(x, y, (-dy, -dx)) - 2 * m,
                                );
                            }
                        }
                    }
                    let rows: Vec<&[i32]> = features.iter().map(Vec::as_slice).collect();
                    lanes.add(&rows, &target, &mut stats);
                }
            }
            lanes.flush(&mut stats);
            Ok(stats)
        })
    }

    /// The one-class bank of `plane` that minimizes the predicted squared
    /// error of `stats` plus its bits, or `None` when no filter lowers the cost.
    fn fit(&self, stats: &WienerStats, plane: usize) -> Result<Option<WienerNsFrameFilterBank>> {
        let view = CoreSeqView::from_sequence(self.seq)
            .ok_or(Error::Unsupported {
                what: "a sequence without a complete view",
            })?
            .restoration();
        let chroma = plane > 0;
        let shapes: &[(usize, bool)] = if chroma {
            &[(0, false), (1, false), (1, true), (2, false), (2, true)]
        } else {
            &[(0, false), (1, false), (2, false), (3, false)]
        };
        let mut best: Option<(i128, WienerNsFrameFilterBank)> = None;
        for &(subset, sym) in shapes {
            let groups = tap_groups(stats.len(), chroma, subset, sym);
            let Some(weights) = solve(stats, &groups) else {
                continue;
            };
            let mut coeffs = vec![0i16; stats.len()];
            for (group, &w) in groups.iter().zip(&weights) {
                let (min, max) = wiener_ns_coeff_range(chroma, group[0]).unwrap_or((0, 0));
                let value = ((w * PREC).round() as i64).clamp(i64::from(min), i64::from(max));
                for &j in group {
                    coeffs[j] = value as i16;
                }
            }
            refine(stats, chroma, &groups, &mut coeffs);
            let bank = one_class_bank(subset, sym, &coeffs)?;
            let cost = rd(
                predicted_delta(stats, &coeffs),
                Bits::whole(bank_bits(&bank, plane, view)?),
                self.lambda,
            );
            if cost < 0 && best.as_ref().is_none_or(|(best, _)| cost < *best) {
                best = Some((cost, bank));
            }
        }
        Ok(best.map(|(_, bank)| bank))
    }

    /// Each unit's cheapest candidate of `options` under the plane types of
    /// `core`, with the exact rate of the CDFs as they adapt in coding order,
    /// and the total cost with the header bits above `off_bits`.
    fn decide(
        &self,
        core: &FrameHeaderCore,
        units: &Units,
        options: &Options,
        off_bits: u64,
    ) -> Result<(i128, BTreeMap<LrUnit, LrUnitType>)> {
        let tools: [FrameRestorationType; 3] = core::array::from_fn(|plane| {
            core.lr_params
                .as_ref()
                .and_then(|lr| lr.planes.get(plane))
                .map_or(FrameRestorationType::None, |p| p.restoration_type)
        });
        let header = core.consumed_bits.saturating_sub(off_bits);
        let mut cost = rd(0, Bits::whole(header as u32), self.lambda);
        let mut lr_on = BTreeMap::new();
        let mut cdfs: BTreeMap<(usize, LrSymbol), Vec<u16>> = BTreeMap::new();
        for (&(layout, unit), candidates) in units.units.iter().zip(options) {
            let tool = tools[unit.plane];
            let mut pick: Option<(i128, Option<LrUnitType>)> = None;
            for &(kind, delta) in candidates {
                let mut bits = Bits::default();
                for (symbol, value) in
                    lr_unit_symbols(tool, unit.plane, kind).map_err(coding("lr unit"))?
                {
                    bits += symbol_cost(self.cdf(&mut cdfs, layout, symbol)?, usize::from(value));
                }
                let value = rd(delta, bits, self.lambda);
                if pick.is_none_or(|(best, _)| value < best) {
                    pick = Some((value, kind));
                }
            }
            let Some((value, kind)) = pick else {
                continue;
            };
            for (symbol, on) in
                lr_unit_symbols(tool, unit.plane, kind).map_err(coding("lr unit"))?
            {
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
        Ok((cost, lr_on))
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

/// A plane choice of `bank`, or of the bank of reference `rst_ref_pic_idx`.
const fn choice(rst_ref_pic_idx: Option<u8>, bank: &WienerNsFrameFilterBank) -> FrameWienerNs {
    FrameWienerNs {
        switchable: false,
        rst_ref_pic_idx,
        bank: *bank,
    }
}

/// A bank of one class with every coefficient `0`, coded with subset `0`.
fn empty_bank() -> WienerNsFrameFilterBank {
    WienerNsFrameFilterBank {
        classes: splot_core::tile::InlineVec::default(),
    }
}

/// The sum of the statistics of the units `taken`.
fn sum_taken(stats: &[WienerStats], taken: &[bool]) -> WienerStats {
    let mut total = WienerStats::new(stats.first().map_or(LUMA_COEFFS, WienerStats::len));
    for (unit, _) in stats.iter().zip(taken).filter(|&(_, &on)| on) {
        total.add(unit);
    }
    total
}

/// The coefficients `wiener_ns_length` subset `subset` codes, grouped so that
/// a group shares one value: pairs of luma taps of a chroma filter with
/// `wiener_ns_uv_sym`, single coefficients otherwise.
fn tap_groups(n: usize, chroma: bool, subset: usize, sym: bool) -> Vec<Vec<usize>> {
    let mut groups = Vec::new();
    let mut j = 0;
    while j < n {
        if !wiener_ns_tap_present(chroma, subset, j) {
            j += 1;
        } else if sym && j >= CHROMA_SHORT && j + 1 < n {
            groups.push(vec![j, j + 1]);
            j += 2;
        } else {
            groups.push(vec![j]);
            j += 1;
        }
    }
    groups
}

fn a_at(stats: &WienerStats, i: usize, j: usize) -> f64 {
    stats.a(i, j) as f64
}

/// `A` and `b` of the group features: each group feature is the sum of the
/// features of its coefficients.
fn group_a(stats: &WienerStats, g: &[usize], h: &[usize]) -> f64 {
    g.iter()
        .flat_map(|&i| h.iter().map(move |&j| a_at(stats, i, j)))
        .sum()
}

fn group_b(stats: &WienerStats, g: &[usize]) -> f64 {
    g.iter().map(|&i| stats.b[i] as f64).sum()
}

/// The least-squares weights of `groups`, by a Cholesky factorization with
/// a small ridge, or `None` for a singular system.
fn solve(stats: &WienerStats, groups: &[Vec<usize>]) -> Option<Vec<f64>> {
    let n = groups.len();
    let trace: f64 = groups.iter().map(|g| group_a(stats, g, g)).sum();
    if n == 0 || trace <= 0.0 {
        return None;
    }
    let ridge = trace / n as f64 * 1e-6;
    let mut l = vec![0.0; n * n];
    for r in 0..n {
        for c in 0..=r {
            let mut v = group_a(stats, &groups[r], &groups[c]) + if r == c { ridge } else { 0.0 };
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
        y[r] = (group_b(stats, &groups[r]) - s) / l[r * n + r];
    }
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| l[k * n + r] * y[k]).sum();
        y[r] = (y[r] - s) / l[r * n + r];
    }
    Some(y)
}

/// Moves the value of one group by one step while that lowers the predicted
/// squared error, within the coded ranges.
fn refine(stats: &WienerStats, chroma: bool, groups: &[Vec<usize>], coeffs: &mut [i16]) {
    for _ in 0..8 * coeffs.len() {
        let mut best = (0.0, 0, 0i16);
        for (index, group) in groups.iter().enumerate() {
            let (min, max) = wiener_ns_coeff_range(chroma, group[0]).unwrap_or((0, 0));
            let row: f64 = group
                .iter()
                .map(|&k| {
                    coeffs
                        .iter()
                        .enumerate()
                        .map(|(j, &c)| f64::from(c) * a_at(stats, k, j))
                        .sum::<f64>()
                })
                .sum();
            for step in [-1i16, 1] {
                let next = coeffs[group[0]] + step;
                if next < min || next > max {
                    continue;
                }
                let d = f64::from(step);
                let change = (2.0 * d * row + d * d * group_a(stats, group, group)) / (PREC * PREC)
                    - 2.0 * d * group_b(stats, group) / PREC;
                if change < best.0 {
                    best = (change, index, step);
                }
            }
        }
        if best.2 == 0 {
            return;
        }
        for &j in &groups[best.1] {
            coeffs[j] += best.2;
        }
    }
}

/// The predicted squared-error change of filtering the samples of `stats`
/// with `coeffs`: `c^T A c / 2^14 - 2 c.b / 2^7`, plus the `1/12` per sample
/// that rounding the output to integers adds.
fn predicted_delta(stats: &WienerStats, coeffs: &[i16]) -> i64 {
    if coeffs.iter().all(|&c| c == 0) {
        return 0;
    }
    let mut quadratic = 0.0;
    let mut linear = 0.0;
    for (i, &ci) in coeffs.iter().enumerate() {
        let ci = f64::from(ci);
        linear += ci * stats.b[i] as f64;
        for (j, &cj) in coeffs.iter().enumerate() {
            quadratic += ci * f64::from(cj) * a_at(stats, i, j);
        }
    }
    (quadratic / (PREC * PREC) - 2.0 * linear / PREC + stats.samples as f64 / 12.0).round() as i64
}

fn one_class_bank(subset: usize, sym: bool, coeffs: &[i16]) -> Result<WienerNsFrameFilterBank> {
    let bad = || Error::Unsupported {
        what: "a Wiener-NS bank",
    };
    let class = WienerNsFrameFilterClass {
        match_index: 0,
        merged: false,
        ref_bank: 0,
        subset: Some(subset as u8),
        wiener_ns_uv_sym: sym,
        coeffs: WienerNsCoeffs::from_iter_checked(coeffs.iter().copied()).ok_or_else(bad)?,
    };
    Ok(WienerNsFrameFilterBank {
        classes: splot_core::tile::InlineVec::from_iter_checked([class]).ok_or_else(bad)?,
    })
}

/// The bits of `bank` of `plane` coded without reference filters.
fn bank_bits(
    bank: &WienerNsFrameFilterBank,
    plane: usize,
    view: CoreSeqRestorationView,
) -> Result<u32> {
    let mut writer = BitWriter::new();
    write_frame_wiener_ns_filter(&mut writer, plane, (0, &[]), view, bank)
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
            i32::from((state % 200) as u16) - 100
        };
        let truth: [i16; LUMA_COEFFS] = [9, -3, 4, 0, -7, 2, 1, 0, -1, 2, 0, 0, 3, -2, 0, 1];
        let mut features = vec![Vec::new(); LUMA_COEFFS];
        let mut target = Vec::new();
        let mut direct = 0i64;
        for _ in 0..4000 {
            let f: [i32; LUMA_COEFFS] = core::array::from_fn(|_| next());
            let fitted: i64 = f
                .iter()
                .zip(&truth)
                .map(|(&f, &c)| i64::from(f) * i64::from(c))
                .sum();
            let t = (fitted + 64).div_euclid(128);
            for (row, &f) in features.iter_mut().zip(&f) {
                row.push(f);
            }
            target.push(t as i32);
            direct += (t * 128 - fitted).pow(2) - (t * 128).pow(2);
        }
        let mut stats = WienerStats::new(LUMA_COEFFS);
        let rows: Vec<&[i32]> = features.iter().map(Vec::as_slice).collect();
        let mut lanes = WienerAccumulator::new(LUMA_COEFFS, 1 << 20);
        lanes.add(&rows, &target, &mut stats);
        lanes.flush(&mut stats);
        stats.samples = 0;
        let groups = tap_groups(LUMA_COEFFS, false, 3, false);
        let weights = solve(&stats, &groups).unwrap();
        for (w, &c) in weights.iter().zip(&truth) {
            assert!((w * PREC - f64::from(c)).abs() < 0.3, "{} vs {c}", w * PREC);
        }
        let predicted = predicted_delta(&stats, &truth) as f64;
        assert!((predicted - direct as f64 / 16384.0).abs() <= 1.0 + predicted.abs() * 1e-9);
    }

    /// Symmetric chroma groups pair the luma taps of each subset.
    #[test]
    fn chroma_groups_pair_the_luma_taps() {
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 0, true).len(), 6);
        let sym = tap_groups(CHROMA_COEFFS, true, 1, true);
        assert_eq!(sym[6..], [vec![6, 7], vec![8, 9]]);
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 2, true).len(), 12);
        assert_eq!(tap_groups(CHROMA_COEFFS, true, 2, false).len(), 18);
    }
}
