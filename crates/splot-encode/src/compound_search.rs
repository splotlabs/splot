// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound type search (§ 5.20.7.16, § 5.20.7.6 `cwp_idx`): the two
//! single-reference predictions of a compound survivor are made once, and
//! every `cwp_idx` weight, wedge with both signs and difference-weighted mask
//! with both `mask_type` values is scored in closed form from their
//! per-sample terms ([`crate::kernels::compound`]). The best of each family
//! that beats `COMPOUND_AVERAGE` goes to the exact trial of the caller.

use std::sync::OnceLock;

use super::*;

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
                for y in 0..height {
                    for x in 0..width {
                        let m = splot_recon::wedge_mask_plane_sample(
                            width, height, index, false, 0, 0, x, y,
                        )
                        .ok()?;
                        masks.push(u8::try_from(m).ok()?);
                    }
                }
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

impl TrialCoder<'_, '_> {
    /// The compound types of the candidate `trial` that beat its
    /// `COMPOUND_AVERAGE` in closed form: the best `cwp_idx` weight, the best
    /// wedge with its sign and the better difference-weighted mask, each
    /// without optical flow or vector refinement, with `4096` times their
    /// squared error; and that of the average.
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
        let rect = leaf_rect(leaf)?;
        let lists = [(base.refs.0, mvs[0]), (base.refs.1, mvs[1])];
        let [s, p0, p1] = motion.compound_inputs(self.syntax, lists, base.filter, rect)?;
        let (mut de, mut ee) = (vec![0; s.len()], vec![0; s.len()]);
        let [dd, sde, see] = kernels().compound_terms(&s, (&p0, &p1), &mut de, &mut ee);
        let average = blend_cost(dd, 32 * sde, 1024 * see);
        let mut out = Vec::new();
        let keep = |cost: i64, blend: CompoundType, best: &mut Option<(i64, CompoundType)>| {
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
            for (index, mask) in (0u8..).zip(masks.chunks_exact(s.len())) {
                let moments = kernels().mask_moments(mask, &de, &ee);
                let [mde, _, mmee] = moments;
                let plain = CompoundType::Wedge { index, sign: false };
                keep(blend_cost(dd, mde, mmee), plain, &mut best);
                let sign = CompoundType::Wedge { index, sign: true };
                keep(inverse_cost(dd, totals, moments), sign, &mut best);
            }
            out.extend(best.take());
        }
        if types.masked {
            let mut mask = vec![0u8; s.len()];
            let shift = u32::from(self.coder.bit_depth.bits() - 8);
            kernels().diff_mask(&p0, &p1, shift, &mut mask);
            let moments = kernels().mask_moments(&mask, &de, &ee);
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

/// Half a luma sample in `1/8` units: the step of the joint refinement.
const HALF_PEL: i32 = 4;

impl TrialCoder<'_, '_> {
    /// The `NEW_NEWMV` survivor `trial` after the joint refinement: in each
    /// round, each list in turn moves its vector by one step of half a sample
    /// (or the frame's difference step) to the neighbour with the least luma
    /// SATD of the average plus vector rate, keeping the other list's
    /// prediction; the rounds stop when no list moves, after two at most.
    pub(super) fn joint_refined(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        trial: CompoundModes,
    ) -> Result<CompoundModes> {
        if !self.preset.joint_newmv || trial.mode != CompoundMode::NewNew {
            return Ok(trial);
        }
        let bases = codable(self.syntax.compound_bases(leaf, &trial).map_err(Into::into))?;
        let Some([Some(base0), Some(base1)]) = bases else {
            return Ok(trial);
        };
        let bases = [base0, base1];
        let rect = leaf_rect(leaf)?;
        let step = self.syntax.mv_step().max(HALF_PEL);
        let refs = [trial.refs.0, trial.refs.1];
        let predict = |syntax: &IntraTileSyntax, list: usize, mv: MotionVector| {
            MotionSearch::luma_prediction(syntax, refs[list], rect, (mv, trial.filter))
        };
        let mut mvs = trial.mvs;
        let mut preds = [
            predict(self.syntax, 0, mvs[0])?,
            predict(self.syntax, 1, mvs[1])?,
        ];
        let rate =
            |mvs: &[MotionVector; 2]| motion.rate(mvs[0], bases[0]) + motion.rate(mvs[1], bases[1]);
        let mut best = motion.average_satd((&preds[0], &preds[1]), rect)? + rate(&mvs);
        for _ in 0..2 {
            let mut moved = false;
            for list in 0..2 {
                let mut pick = None;
                for (dr, dc) in [
                    (-1, -1),
                    (-1, 0),
                    (-1, 1),
                    (0, -1),
                    (0, 1),
                    (1, -1),
                    (1, 0),
                    (1, 1),
                ] {
                    let mut moved_mvs = mvs;
                    moved_mvs[list] = MotionVector {
                        row: mvs[list].row + dr * step,
                        col: mvs[list].col + dc * step,
                    };
                    let p = predict(self.syntax, list, moved_mvs[list])?;
                    let pair: (&[u16], &[u16]) = if list == 0 {
                        (&p, &preds[1])
                    } else {
                        (&preds[0], &p)
                    };
                    let cost = motion.average_satd(pair, rect)? + rate(&moved_mvs);
                    if cost < best {
                        best = cost;
                        pick = Some((moved_mvs, p));
                    }
                }
                if let Some((picked, p)) = pick {
                    (mvs, preds[list], moved) = (picked, p, true);
                }
            }
            if !moved {
                break;
            }
        }
        let refined = CompoundModes { mvs, ..trial };
        let codable_mvs = codable(
            self.syntax
                .inter_mvs(leaf, InterPrediction::Compound(refined))
                .map_err(Into::into),
        )?;
        Ok(if codable_mvs == Some(mvs) {
            refined
        } else {
            trial
        })
    }
}
