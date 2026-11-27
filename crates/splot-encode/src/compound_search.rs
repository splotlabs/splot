// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound type search (§ 5.20.7.16, § 5.20.7.6 `cwp_idx`): the two
//! single-reference predictions of a compound survivor are made once, and
//! every `cwp_idx` weight, wedge with both signs and difference-weighted mask
//! with both `mask_type` values is scored in closed form from their
//! per-sample terms ([`crate::kernels::compound`]). The best of each family
//! that beats `COMPOUND_AVERAGE` goes to the exact trial of the caller.

use std::sync::OnceLock;

use splot_decode::encoder_syntax::TransformBlock;

use super::*;

#[cfg(test)]
thread_local! {
    /// While `Some`, collects the closed-form average cost and the cost of
    /// the decoder's plain average prediction of each unrefined candidate.
    pub(crate) static TYPED_UNITS: core::cell::RefCell<Option<Vec<(i64, i64)>>> =
        const { core::cell::RefCell::new(None) };
    /// Refined candidates on this thread whose average reference is the
    /// decoder's refined prediction.
    pub(crate) static TYPED_REFINED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
    /// Compound types that beat the closed-form average but not the refined
    /// prediction, so the search drops them, on this thread.
    pub(crate) static TYPED_DROPPED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// `MAX_WEDGE_TYPES`: the `WedgeIndex` values of `Wedge_Angle_Dist_2_Index`
/// (§ 5.20.7.18).
const WEDGE_TYPES: usize = 68;
/// Luma block sides with wedge masks: 8, 16, 32 and 64.
const WEDGE_SIDES: usize = 4;

/// The § 7.13.3.27 wedge masks of each block size with `wedge_sign = 0`,
/// `WEDGE_TYPES` row-major masks per size, built on first use.
static WEDGE_MASKS: [OnceLock<Option<Box<[u8]>>>; WEDGE_SIDES * WEDGE_SIDES] =
    [const { OnceLock::new() }; WEDGE_SIDES * WEDGE_SIDES];

/// The wedge masks of a `width`x`height` luma block, `None` for a size
/// without them.
fn wedge_masks(width: usize, height: usize) -> Option<&'static [u8]> {
    let side = |n: usize| {
        (n.is_power_of_two() && (8..=64).contains(&n)).then(|| n.trailing_zeros() as usize - 3)
    };
    let slot = side(width)? * WEDGE_SIDES + side(height)?;
    WEDGE_MASKS[slot]
        .get_or_init(|| {
            let mut masks = Vec::with_capacity(WEDGE_TYPES * width * height);
            for index in 0..WEDGE_TYPES {
                let rows = splot_recon::wedge_mask_rows(width, height, index, false).ok()?;
                masks.extend(rows.flatten());
            }
            Some(masks.into_boxed_slice())
        })
        .as_deref()
}

/// `4096` times the squared error of the blend that gives `p0` the mask
/// `m / 64` and `p1` the rest, from `Σd²` and the mask moments
/// `[Σm·de, Σm²·ee]`.
const fn blend_cost(dd: i64, mde: i64, mmee: i64) -> i64 {
    4096 * dd - 128 * mde + mmee
}

/// [`blend_cost`] of the mask `64 - m`, from the totals `Σde`, `Σee` and
/// the moments `[Σm·de, Σm·ee, Σm²·ee]` of `m`.
const fn inverse_cost(dd: i64, (sde, see): (i64, i64), [mde, mee, mmee]: [i64; 3]) -> i64 {
    blend_cost(dd, 64 * sde - mde, 4096 * see - 128 * mee + mmee)
}

/// The reused buffers of [`TrialCoder::typed_variants`]: the source and the
/// two predictions of a block, their per-sample terms and a mask.
#[derive(Default)]
pub(super) struct CompoundScratch {
    source: Vec<u16>,
    p0: Vec<u16>,
    p1: Vec<u16>,
    de: Vec<i32>,
    ee: Vec<i32>,
    mask: Vec<u8>,
}

impl TrialCoder<'_, '_> {
    /// The compound types of the candidate `trial` that beat its
    /// `COMPOUND_AVERAGE` in closed form: the best `cwp_idx` weight, the best
    /// wedge with its sign and the better difference-weighted mask, each
    /// without optical flow or vector refinement, with `4096` times their
    /// squared error; and that of the average. With `opfl_refine_all` the
    /// average of a candidate the decoder refines (`REFINE_ALL`, § 5.18.3.2)
    /// is its refined prediction, so a type must beat that.
    pub(super) fn typed_variants(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        trial: CompoundModes,
    ) -> Result<(i64, Vec<(i64, CompoundModes)>)> {
        let none = || Ok((0, Vec::new()));
        if self.preset.compound_types == 0 {
            return none();
        }
        let base = CompoundModes {
            optflow: false,
            refinemv: false,
            blend: CompoundType::default(),
            ..trial
        };
        let Some(types) = codable(self.syntax.compound_types(leaf, base).map_err(Into::into))?
        else {
            return none();
        };
        if !types.masked && types.cwp.is_none() {
            return none();
        }
        let prediction = InterPrediction::Compound(base);
        let Some(mvs) = codable(self.syntax.inter_mvs(leaf, prediction).map_err(Into::into))?
        else {
            return none();
        };
        let refined = self.preset.opfl_refine_all
            && codable(
                self.syntax
                    .compound_refinements(leaf, trial)
                    .map_err(Into::into),
            )?
            .is_some_and(|(.., refined)| refined);
        let refined = if refined {
            codable(
                self.syntax
                    .luma_prediction(leaf, InterPrediction::Compound(trial))
                    .map_err(Into::into),
            )?
        } else {
            None
        };
        let rect = leaf_rect(leaf)?;
        let lists = [(base.refs.0, mvs[0]), (base.refs.1, mvs[1])];
        let CompoundScratch {
            source: s,
            p0,
            p1,
            de,
            ee,
            mask,
        } = &mut self.compound;
        motion.compound_inputs(self.syntax, (lists, base.filter), rect, [s, p0, p1])?;
        de.clear();
        de.resize(s.len(), 0);
        ee.clear();
        ee.resize(s.len(), 0);
        let [dd, sde, see] = kernels().compound_terms(s, (p0, p1), de, ee);
        let closed = blend_cost(dd, 32 * sde, 1024 * see);
        let visible = self.coder.source.visible_luma()?;
        let average = match &refined {
            Some(blocks) => decoded_average_cost((s, p0, p1), rect, visible, blocks)?,
            None => closed,
        };
        #[cfg(test)]
        TYPED_REFINED.set(TYPED_REFINED.get() + usize::from(refined.is_some()));
        #[cfg(test)]
        if refined.is_none() && TYPED_UNITS.with_borrow(Option::is_some) {
            let plain = self
                .syntax
                .luma_prediction(leaf, prediction)
                .map_err(Into::into);
            if let Some(blocks) = codable(plain)? {
                let decoded = decoded_average_cost((s, p0, p1), rect, visible, &blocks)?;
                TYPED_UNITS.with_borrow_mut(|units| {
                    if let Some(units) = units {
                        units.push((closed, decoded));
                    }
                });
            }
        }
        let mut out = Vec::new();
        let keep = |cost: i64, blend: CompoundType, best: &mut Option<(i64, CompoundType)>| {
            #[cfg(test)]
            if cost < closed && cost >= average {
                TYPED_DROPPED.set(TYPED_DROPPED.get() + 1);
            }
            if cost < average && best.is_none_or(|(c, _)| cost < c) {
                *best = Some((cost, blend));
            }
        };
        let mut best = None;
        for (cwp, &weight) in (0u8..).zip(types.cwp.iter().flatten()).skip(1) {
            let m = 4 * i64::from(weight);
            keep(
                blend_cost(dd, m * sde, m * m * see),
                CompoundType::Average { cwp },
                &mut best,
            );
        }
        out.extend(best.take());
        let totals = (sde, see);
        if let Some(masks) = types.wedge.then(|| wedge_masks(rect.2, rect.3)).flatten() {
            for (index, wedge) in (0u8..).zip(masks.chunks_exact(s.len())) {
                let moments = kernels().mask_moments(wedge, de, ee);
                let [mde, _, mmee] = moments;
                let plain = CompoundType::Wedge { index, sign: false };
                keep(blend_cost(dd, mde, mmee), plain, &mut best);
                let sign = CompoundType::Wedge { index, sign: true };
                keep(inverse_cost(dd, totals, moments), sign, &mut best);
            }
            out.extend(best.take());
        }
        if types.masked {
            mask.clear();
            mask.resize(s.len(), 0);
            let shift = u32::from(self.coder.bit_depth.bits() - 8);
            kernels().diff_mask(p0, p1, shift, mask);
            let moments = kernels().mask_moments(mask, de, ee);
            let [mde, _, mmee] = moments;
            let plain = CompoundType::DiffWeighted { inverse: false };
            keep(blend_cost(dd, mde, mmee), plain, &mut best);
            let inverse = CompoundType::DiffWeighted { inverse: true };
            keep(inverse_cost(dd, totals, moments), inverse, &mut best);
            out.extend(best.take());
        }
        Ok((
            average,
            out.into_iter()
                .map(|(cost, blend)| (cost, CompoundModes { blend, ..base }))
                .collect(),
        ))
    }
}

/// [`blend_cost`] of the average whose visible samples are the decoder's
/// luma prediction `blocks`: `4096·(s - d)²` there, and the closed-form
/// `(64·s - 32·p0 - 32·p1)²` of the search predictions outside the visible
/// `(width, height)`, where the decoder predicts nothing to compare.
fn decoded_average_cost(
    (s, p0, p1): (&[u16], &[u16], &[u16]),
    (left, top, width, height): (usize, usize, usize, usize),
    visible: (usize, usize),
    blocks: &[(TransformBlock, Vec<u16>)],
) -> Result<i64> {
    let outside = || TileWriterError::Shape("prediction block outside the leaf");
    let mut decoded = vec![None; width * height];
    for (tx, samples) in blocks {
        let (log2_w, _) = tx_log2(tx.tx_size)?;
        let (Some(x), Some(y)) = (tx.x.checked_sub(left), tx.y.checked_sub(top)) else {
            return Err(outside());
        };
        for (row, predicted) in (y..).zip(samples.chunks_exact(1 << log2_w)) {
            let start = row * width + x;
            let cells = decoded
                .get_mut(start..start + predicted.len())
                .filter(|_| x + predicted.len() <= width)
                .ok_or_else(outside)?;
            for (cell, &d) in cells.iter_mut().zip(predicted) {
                *cell = Some(d);
            }
        }
    }
    let mut cost = 0i64;
    let samples = decoded.iter().zip(s).zip(p0.iter().zip(p1));
    for (i, ((&d, &s), (&p0, &p1))) in samples.enumerate() {
        let s = i64::from(s);
        let shown = left + i % width < visible.0 && top + i / width < visible.1;
        cost += match (shown, d) {
            (true, Some(d)) => 4096 * (s - i64::from(d)).pow(2),
            (true, None) => {
                return Err(TileWriterError::Shape(
                    "visible sample without a prediction",
                ));
            }
            (false, _) => (64 * s - 32 * i64::from(p0) - 32 * i64::from(p1)).pow(2),
        };
    }
    Ok(cost)
}

impl TrialCoder<'_, '_> {
    /// The list-1 vector of a same-reference `NEW_NEWMV` whose list 0 is
    /// `first`: the neighbour of `first` at one or one half sample whose
    /// two-vector average has the least luma SATD; `None` without one inside
    /// the reference.
    pub(super) fn same_ref_partner(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        ref_frame: usize,
        first: MotionVector,
    ) -> Result<Option<MotionVector>> {
        let rect = leaf_rect(leaf)?;
        let mut best: Option<(u64, MotionVector)> = None;
        for step in [FULL_PEL, FULL_PEL / 2] {
            for (dr, dc) in crate::motion_search::NEIGHBOURS {
                let partner = MotionVector {
                    row: first.row + dr * step,
                    col: first.col + dc * step,
                };
                let lists = [(ref_frame, first), (ref_frame, partner)];
                let Some(satd) = codable(motion.prediction_satd(
                    self.syntax,
                    &lists,
                    InterpolationFilter::EightTap,
                    rect,
                ))?
                else {
                    continue;
                };
                if best.is_none_or(|(cost, _)| satd < cost) {
                    best = Some((satd, partner));
                }
            }
        }
        Ok(best.map(|(_, partner)| partner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Σ(64·s - (m·p0 + (64 - m)·p1))²`: 4096 times the squared error of
    /// the blend that gives `p0` the weight `m / 64`.
    fn explicit(s: &[u16], (p0, p1): (&[u16], &[u16]), weight: impl Fn(usize) -> i64) -> i64 {
        (0..s.len())
            .map(|i| {
                let m = weight(i);
                let r = 64 * i64::from(s[i]) - (m * i64::from(p0[i]) + (64 - m) * i64::from(p1[i]));
                r * r
            })
            .sum()
    }

    /// The closed-form cost of every wedge with both signs, both
    /// difference-weighted masks and every `cwp_idx` weight of both
    /// `Cwp_Weighting_Factor` rows equals the squared error of the blend the
    /// decoder's mask or weight gives (§ 7.13.3.14, § 7.13.3.27,
    /// § 7.13.3.28), on asymmetric 10-bit samples of a 16x8 block.
    #[test]
    fn closed_form_costs_equal_the_blend_error() {
        let (width, height) = (16, 8);
        let mut state = 0x9E37_79B9u32;
        let mut next = |range: u32| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state % range) as u16
        };
        let s: Vec<u16> = (0..width * height).map(|_| 200 + next(600)).collect();
        let p0: Vec<u16> = (0..width * height).map(|_| next(1024)).collect();
        let p1: Vec<u16> = (0..width * height).map(|_| 300 + next(500)).collect();
        let (mut de, mut ee) = (vec![0; s.len()], vec![0; s.len()]);
        let [dd, sde, see] = kernels().compound_terms(&s, (&p0, &p1), &mut de, &mut ee);
        let preds = (&p0[..], &p1[..]);
        let masks = wedge_masks(width, height).unwrap();
        for (index, mask) in masks.chunks_exact(s.len()).enumerate() {
            let inverse = splot_recon::wedge_mask_rows(width, height, index, true).unwrap();
            for (&m, &inverse) in mask.iter().zip(inverse.flatten()) {
                assert_eq!(64 - m, inverse);
            }
            let moments = kernels().mask_moments(mask, &de, &ee);
            let plain = explicit(&s, preds, |i| i64::from(mask[i]));
            assert_eq!(blend_cost(dd, moments[0], moments[2]), plain, "{index}");
            let signed = explicit(&s, preds, |i| 64 - i64::from(mask[i]));
            assert_eq!(inverse_cost(dd, (sde, see), moments), signed, "{index}");
        }
        let mut mask = vec![0u8; s.len()];
        kernels().diff_mask(&p0, &p1, 2, &mut mask);
        let moments = kernels().mask_moments(&mask, &de, &ee);
        let inverse = explicit(&s, preds, |i| 64 - i64::from(mask[i]));
        assert_eq!(inverse_cost(dd, (sde, see), moments), inverse);
        for weight in [8i64, 12, 4, 10, 6, 20, -4] {
            let blend = explicit(&s, preds, |_| 4 * weight);
            let m = 4 * weight;
            assert_eq!(blend_cost(dd, m * sde, m * m * see), blend, "{weight}");
        }
    }
}
