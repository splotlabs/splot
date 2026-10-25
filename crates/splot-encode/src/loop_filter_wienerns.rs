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
use std::ops::Range;

use splot_core::headers::frame::DECODE_NUM_FILTER_CLASSES;
use splot_core::headers::frame::{
    CdefStrengthSets, FrameFilterParams, FrameHeaderCore, FrameRestorationType, FrameWienerNs,
    WienerNsFrameFilterBank,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::adapt_cdf_u16;
use splot_decode::encoder_syntax::{
    LrLayout, LrSymbol, LrUnit, LrUnitType, PcWienerClassMap, WIENER_NS_CONFIG_UV,
    WIENER_NS_CONFIG_Y_PAIRS, apply_loop_filters, pc_wiener_cell_classes,
    wiener_ns_chroma_luma_plane,
};
use splot_recon::PC_WIENER_LUT_CLASSES;
use splot_recon::{DecodedFrame, PlaneId};

use super::ccso::rd;
use super::{Search, coding};
use crate::error::{Error, Result};
use crate::intra_frame::map_tiles;
use crate::kernels::wiener::{WienerAccumulator, WienerClassAccumulator, WienerStats};
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

    /// Row `y + dy` from column `-radius`.
    fn row(&self, y: usize, dy: isize) -> &[u16] {
        let start = ((y + self.radius) as isize + dy) as usize * self.width;
        &self.samples[start..start + self.width]
    }

    /// `source - m` over the columns `span` of row `y`, `source` from column `0`.
    fn target(&self, y: usize, span: Range<usize>, source: &[u16], out: &mut Vec<i32>) {
        let centre = &self.row(y, 0)[self.radius..];
        out.clear();
        out.extend(
            source[span.clone()]
                .iter()
                .zip(&centre[span])
                .map(|(&s, &m)| i32::from(s) - i32::from(m)),
        );
    }

    /// The pair feature `s(dy, dx) + s(-dy, -dx) - 2m` over `span` of row `y`.
    fn pair(&self, y: usize, span: Range<usize>, (dy, dx): (isize, isize), out: &mut Vec<i32>) {
        let radius = self.radius as isize;
        let (plus, minus, centre) = (self.row(y, dy), self.row(y, -dy), self.row(y, 0));
        let (ahead, behind) = ((radius + dx) as usize, (radius - dx) as usize);
        out.clear();
        out.extend(span.map(|x| {
            i32::from(plus[x + ahead]) + i32::from(minus[x + behind])
                - 2 * i32::from(centre[x + self.radius])
        }));
    }

    /// The tap difference `s(dy, dx) - m` over `span` of row `y`.
    fn diff(&self, y: usize, span: Range<usize>, (dy, dx): (isize, isize), out: &mut Vec<i32>) {
        let (tap, centre) = (self.row(y, dy), self.row(y, 0));
        let (p, c) = ((self.radius as isize + dx) as usize, self.radius);
        out.clear();
        out.extend(span.map(|x| i32::from(tap[x + p]) - i32::from(centre[x + c])));
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
        (preset, pre_cdef): (&PresetParams, Option<&DecodedFrame<u16>>),
    ) -> Result<(FrameFilterParams, BTreeMap<LrUnit, LrUnitType>)> {
        let core = self.header(pc)?;
        let units = Units::new(self.seq, &core)?;
        let off = apply_loop_filters(
            self.seq,
            &core,
            self.recorder,
            self.recon,
            (&syntax.cdef_indices, &syntax.ccso_blocks, None),
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
        let counts: Vec<u8> = DECODE_NUM_FILTER_CLASSES
            .iter()
            .copied()
            .filter(|&count| count > 1 && count <= preset.wiener_ns_classes)
            .collect();
        let map = if counts.is_empty() || preset.wiener_ns == 0 {
            None
        } else {
            Some(
                pc_wiener_cell_classes(
                    self.seq,
                    &core,
                    self.recorder,
                    (pre_cdef.unwrap_or(&off), &off),
                )
                .map_err(coding("PC-Wiener classes"))?,
            )
        };
        let (stats, mut classes) = match &map {
            Some(map) => {
                let (singles, classes) = self.luma_class_stats(&off, &units, map, None)?;
                (singles, Some(classes))
            }
            None => (self.plane_stats(&off, &units, 0)?, None),
        };
        let mut taken = vec![true; stats.len()];
        for pass in 0..preset.wiener_ns {
            let mut pick = self.fit(&sum_taken(&stats, &taken), 0)?;
            if let Some(map) = &map
                && pass > 0
            {
                classes = Some(self.luma_class_stats(&off, &units, map, Some(&taken))?.1);
            }
            if let Some(classes) = &classes
                && let Some((cost, bank)) = self.fit_classes(classes, &counts)?
                && pick.as_ref().is_none_or(|(single, _)| cost < *single)
            {
                pick = Some((cost, bank));
            }
            let Some((_, bank)) = pick else {
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
    /// against keeping the plane off; every other unit keeps its type. The
    /// chroma units are the luma units at chroma scale, but at least the
    /// stripe height, and must not cross a tile edge (§ 6.17.7.7).
    fn chroma(
        &self,
        mut best: Choice,
        off: &DecodedFrame<u16>,
        preset: &PresetParams,
        off_bits: u64,
    ) -> Result<Choice> {
        let format = off.info().pixel_format();
        let shift = format.subsampling_x().max(format.subsampling_y());
        let original = best.params;
        let luma_size = best.params.lr_luma_unit_size.unwrap_or(64);
        let chroma_size = (luma_size >> shift).max(64 >> format.subsampling_y());
        if !units_fit_tiles(&self.header(&best.params)?, chroma_size << shift) {
            return Ok(best);
        }
        if best.lr_on.is_empty() {
            best.params.lr_luma_unit_size = None;
            best.params.lr_luma_wiener_ns = None;
        }
        best.params.lr_chroma_unit_size = Some(chroma_size);
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
                let Some((_, bank)) = self.fit(&sum_taken(&stats, &taken), plane)? else {
                    break;
                };
                taken = try_bank(None, bank)?;
                if !taken.contains(&true) {
                    break;
                }
            }
        }
        if current.params.lr_chroma_wiener_ns == [None; 2] {
            best.params = original;
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
        apply_loop_filters(self.seq, &core, self.recorder, off, (&[], &[], None), lr_on)
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
        let coded = off.coded_luma_size();
        let format = off.info().pixel_format();
        let max = u64::from(off.bit_depth().max_sample());
        let max_product = 4 * max * max;
        let source_size = self.source.plane_sizes()[plane];
        let source = &self.source.planes()[plane];
        let jobs: Vec<_> = units
            .units
            .iter()
            .filter(|(_, unit)| unit.plane == plane)
            .copied()
            .collect();
        if plane == 0 {
            let padded = luma_padded(off)?;
            return map_tiles(self.pool, jobs, |(layout, unit)| {
                let mut stats = WienerStats::new(LUMA_COEFFS);
                let mut lanes = WienerAccumulator::<LUMA_COEFFS>::new(max_product);
                let mut features: [Vec<i32>; LUMA_COEFFS] = Default::default();
                let mut target = Vec::new();
                for (y, span) in unit_rows(units, layout, unit, source_size)? {
                    let row = &source[y * source_size.width()..];
                    luma_row(&padded, y, span, row, (&mut features, &mut target));
                    let rows = core::array::from_fn(|k| features[k].as_slice());
                    lanes.add(&rows, &target, &mut stats);
                }
                lanes.flush(&mut stats);
                Ok(stats)
            });
        }
        let (sub_x, sub_y) = (format.subsampling_x(), format.subsampling_y());
        let size = (
            coded.width().div_ceil(1 << sub_x),
            coded.height().div_ceil(1 << sub_y),
        );
        let ids = [PlaneId::Y, PlaneId::U, PlaneId::V];
        let samples = off.plane(ids[plane]).ok_or_else(missing)?;
        let padded = Padded::new(
            samples.samples(),
            samples.stride_samples(),
            size,
            CHROMA_RADIUS,
        )?;
        let luma = off.plane(PlaneId::Y).ok_or_else(missing)?;
        let cfl = self
            .seq
            .intra
            .as_ref()
            .map_or(0, |intra| intra.cfl_ds_filter_index);
        let luma = wiener_ns_chroma_luma_plane(
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
        let luma = Padded::new(&luma, size.0, size, CHROMA_RADIUS)?;
        map_tiles(self.pool, jobs, |(layout, unit)| {
            let mut stats = WienerStats::new(CHROMA_COEFFS);
            let mut lanes = WienerAccumulator::<CHROMA_COEFFS>::new(max_product);
            let mut features: [Vec<i32>; CHROMA_COEFFS] = Default::default();
            let mut target = Vec::new();
            for (y, span) in unit_rows(units, layout, unit, source_size)? {
                let row = &source[y * source_size.width()..];
                padded.target(y, span.clone(), row, &mut target);
                for (k, &(dy, dx, _)) in WIENER_NS_CONFIG_UV.iter().enumerate() {
                    if k % 2 == 0 {
                        padded.pair(y, span.clone(), (dy, dx), &mut features[k / 2]);
                    }
                    luma.diff(y, span.clone(), (dy, dx), &mut features[CHROMA_SHORT + k]);
                }
                let rows = core::array::from_fn(|k| features[k].as_slice());
                lanes.add(&rows, &target, &mut stats);
            }
            lanes.flush(&mut stats);
            Ok(stats)
        })
    }

    /// The normal equations of each luma unit, in [`Units`] order, and of each
    /// § 7.20.4 class of `map` over the units `taken` (all when `None`). One
    /// worker-pool job codes a row of units, so the class sums stay one set
    /// per unit row.
    fn luma_class_stats(
        &self,
        off: &DecodedFrame<u16>,
        units: &Units,
        map: &PcWienerClassMap,
        taken: Option<&[bool]>,
    ) -> Result<(Vec<WienerStats>, Vec<WienerStats>)> {
        let unclassified = || Error::Unsupported {
            what: "a luma sample without a PC-Wiener class",
        };
        let max = u64::from(off.bit_depth().max_sample());
        let source_size = self.source.plane_sizes()[0];
        let source = &self.source.planes()[0];
        let padded = luma_padded(off)?;
        let luma_units: Vec<(usize, LrUnit)> = units
            .units
            .iter()
            .filter(|(_, unit)| unit.plane == 0)
            .copied()
            .collect();
        let mut rows: Vec<Vec<(usize, usize, LrUnit)>> = Vec::new();
        for (index, &(layout, unit)) in luma_units.iter().enumerate() {
            if taken.is_some_and(|taken| !taken.get(index).copied().unwrap_or(false)) {
                continue;
            }
            match rows.last_mut() {
                Some(row)
                    if row.last().is_some_and(|&(_, last, prior)| {
                        last == layout && prior.row == unit.row
                    }) =>
                {
                    row.push((index, layout, unit));
                }
                _ => rows.push(vec![(index, layout, unit)]),
            }
        }
        let jobs = map_tiles(self.pool, rows, |row| {
            let mut lanes =
                WienerClassAccumulator::<LUMA_COEFFS>::new(PC_WIENER_LUT_CLASSES, 4 * max * max);
            let mut unit_classes = vec![WienerStats::new(LUMA_COEFFS); PC_WIENER_LUT_CLASSES];
            let mut totals = vec![WienerStats::new(LUMA_COEFFS); PC_WIENER_LUT_CLASSES];
            let mut singles = Vec::with_capacity(row.len());
            let mut features: [Vec<i32>; LUMA_COEFFS] = Default::default();
            let (mut target, mut cells) = (Vec::new(), Vec::new());
            for (index, layout, unit) in row {
                for (y, span) in unit_rows(units, layout, unit, source_size)? {
                    cells.clear();
                    for x in span.clone().step_by(4) {
                        cells.push(map.class(x, y).ok_or_else(unclassified)?);
                    }
                    let line = &source[y * source_size.width()..];
                    luma_row(&padded, y, span, line, (&mut features, &mut target));
                    let rows = core::array::from_fn(|k| features[k].as_slice());
                    lanes.add(&rows, &target, &cells, &mut unit_classes);
                }
                let mut single = WienerStats::new(LUMA_COEFFS);
                for class in lanes.flush(&mut unit_classes) {
                    single.add(&unit_classes[class]);
                    totals[class].add(&unit_classes[class]);
                    unit_classes[class] = WienerStats::new(LUMA_COEFFS);
                }
                singles.push((index, single));
            }
            Ok((singles, totals))
        })?;
        let mut singles = vec![WienerStats::new(LUMA_COEFFS); luma_units.len()];
        let mut totals = vec![WienerStats::new(LUMA_COEFFS); PC_WIENER_LUT_CLASSES];
        for (row_singles, row_totals) in jobs {
            for (index, single) in row_singles {
                singles[index] = single;
            }
            for (total, class) in totals.iter_mut().zip(&row_totals) {
                total.add(class);
            }
        }
        Ok((singles, totals))
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

/// Whether units of `size` luma samples end on every internal tile edge of
/// the frame `core` (§ 6.17.7.7 `check_ru_size`).
fn units_fit_tiles(core: &FrameHeaderCore, size: u32) -> bool {
    core.tile_info.as_ref().is_some_and(|tiles| {
        [&tiles.mi_col_starts[..], &tiles.mi_row_starts[..]]
            .iter()
            .all(|starts| {
                starts
                    .windows(2)
                    .take(starts.len().saturating_sub(2))
                    .all(|pair| ((pair[1] - pair[0]) * 4) % size == 0)
            })
    })
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

/// The luma of `off` over its coded size, padded for the luma taps.
fn luma_padded(off: &DecodedFrame<u16>) -> Result<Padded> {
    let luma = off.plane(PlaneId::Y).ok_or(Error::Unsupported {
        what: "a frame without luma",
    })?;
    let coded = off.coded_luma_size();
    Padded::new(
        luma.samples(),
        luma.stride_samples(),
        (coded.width(), coded.height()),
        LUMA_RADIUS,
    )
}

/// The luma features and target of the samples `span` of row `y`.
fn luma_row(
    padded: &Padded,
    y: usize,
    span: Range<usize>,
    source: &[u16],
    (features, target): (&mut [Vec<i32>; LUMA_COEFFS], &mut Vec<i32>),
) {
    padded.target(y, span.clone(), source, target);
    for (k, &(dy, dx, _)) in WIENER_NS_CONFIG_Y_PAIRS.iter().enumerate() {
        padded.pair(y, span.clone(), (dy, dx), &mut features[k]);
    }
}

/// The visible rows of `unit` and their column spans.
fn unit_rows(
    units: &Units,
    layout: usize,
    unit: LrUnit,
    size: splot_recon::PlaneSize,
) -> Result<Vec<(usize, Range<usize>)>> {
    let mut rows = Vec::new();
    for rect in units.layouts[layout]
        .unit_rects(unit)
        .map_err(coding("restoration unit"))?
    {
        let right = (rect.x() + rect.width()).min(size.width());
        for y in rect.y()..(rect.y() + rect.height()).min(size.height()) {
            rows.push((y, rect.x()..right));
        }
    }
    Ok(rows)
}

#[path = "loop_filter_wienerns_fit.rs"]
mod fit;
use fit::one_class_bank;
