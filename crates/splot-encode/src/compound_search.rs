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
    /// The compound types of the survivor `trial` that beat its
    /// `COMPOUND_AVERAGE` in closed form: the best `cwp_idx` weight, the best
    /// wedge with its sign and the better difference-weighted mask, each
    /// without optical flow or vector refinement.
    pub(super) fn typed_variants(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        trial: CompoundModes,
    ) -> Result<Vec<CompoundModes>> {
        if self.preset.compound_types == 0 {
            return Ok(Vec::new());
        }
        let base = CompoundModes {
            optflow: false,
            refinemv: false,
            blend: CompoundType::default(),
            ..trial
        };
        let Some(types) = codable(self.syntax.compound_types(leaf, base).map_err(Into::into))?
        else {
            return Ok(Vec::new());
        };
        if !types.masked && types.cwp.is_none() {
            return Ok(Vec::new());
        }
        let prediction = InterPrediction::Compound(base);
        let Some(mvs) = codable(self.syntax.inter_mvs(leaf, prediction).map_err(Into::into))?
        else {
            return Ok(Vec::new());
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
        Ok(out
            .into_iter()
            .map(|(_, blend)| CompoundModes { blend, ..base })
            .collect())
    }
}
