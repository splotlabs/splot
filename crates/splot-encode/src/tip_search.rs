// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! TIP `NEWMV` search (§ 5.20.7.6 `tip_pred_mode`): the motion pyramids hold
//! no TIP frame, so each vector is scored by the decoder's own TIP luma
//! prediction (§ 7.13.3) plus the exact mode and difference rate.

use splot_decode::encoder_syntax::SingleMvs;

use super::*;

/// The pattern steps of the search, in `1/8` samples: two samples down to
/// one eighth; steps finer than the frame precision are skipped.
const STEPS: [i32; 5] = [16, 8, 4, 2, 1];
/// Moves per step before the search goes to the next finer step.
const MOVES: usize = 3;

#[cfg(test)]
thread_local! {
    /// Leaves on this thread whose TIP `NEWMV` search ended on a `NEARMV`
    /// vector and so added no `NEWMV` candidate.
    pub(crate) static TIP_NEWMV_ON_NEAR: core::cell::Cell<usize> =
        const { core::cell::Cell::new(0) };
}

impl TrialCoder<'_, '_> {
    /// The TIP `NEWMV` of `leaf` with the least decoder luma prediction SATD
    /// plus mode rate: a pattern search from the best `NEARMV` vector of the
    /// TIP stack `tip`, with the DRL base whose price is lowest at the end,
    /// and with `preset.tip_amvd` and adaptive MVD its `use_amvd` coding
    /// where that costs less;
    /// `None` where no TIP `NEWMV` is codable or the search ends on a
    /// `NEARMV` vector, which `NEARMV` codes at a lower rate.
    pub(super) fn tip_newmv(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        tip: &SingleMvs,
    ) -> Result<Option<InterModes>> {
        let ref_frame = self.syntax.reference_count();
        let modes = |mode, drl, mv| InterModes {
            ref_frame,
            mode,
            drl,
            mv,
            filter: InterpolationFilter::EightTap,
            precision: None,
            amvd: false,
            interintra: None,
            bawp: None,
        };
        let mut start = (u64::MAX, MotionVector::default());
        for (drl, &mv) in tip.near.iter().enumerate() {
            let near = InterPrediction::Single(modes(InterMode::Near, drl, mv));
            if let Some(cost) = self.tip_cost(leaf, motion, near)?
                && cost < start.0
            {
                start = (cost, mv);
            }
        }
        let mut best: Option<(u64, InterModes)> = None;
        let mut center = start.1;
        let step_min = self.syntax.mv_step();
        for step in STEPS.into_iter().filter(|&step| step >= step_min) {
            for _ in 0..MOVES {
                let mut moved = false;
                let candidates = core::iter::once((0, 0))
                    .chain(crate::motion_search::NEIGHBOURS)
                    .map(|(dr, dc)| MotionVector {
                        row: center.row + dr * step,
                        col: center.col + dc * step,
                    });
                for mv in candidates.collect::<Vec<_>>() {
                    let new = InterPrediction::Single(modes(InterMode::New, 0, mv));
                    let Some(InterPrediction::Single(coded)) =
                        codable(snap_prediction(self.syntax, leaf, new))?
                    else {
                        continue;
                    };
                    if best.is_some_and(|(_, seen)| seen.mv == coded.mv) {
                        continue;
                    }
                    let Some(cost) = self.tip_cost(leaf, motion, InterPrediction::Single(coded))?
                    else {
                        continue;
                    };
                    if best.is_none_or(|(c, _)| cost < c) {
                        moved = best.is_some();
                        best = Some((cost, coded));
                        center = coded.mv;
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        let Some((_, found)) = best.filter(|(_, found)| !tip.near.contains(&found.mv)) else {
            #[cfg(test)]
            TIP_NEWMV_ON_NEAR.set(TIP_NEWMV_ON_NEAR.get() + usize::from(best.is_some()));
            return Ok(None);
        };
        let mut cheapest: Option<(Bits, InterModes)> = None;
        for drl in 0..tip.new_bases.len() {
            let candidate = InterModes { drl, ..found };
            let mut sink = RateSink::adapting();
            if codable(
                self.syntax
                    .price_inter_modes(leaf, InterPrediction::Single(candidate), &mut sink)
                    .map_err(Into::into),
            )?
            .is_some()
                && cheapest.is_none_or(|(bits, _)| sink.bits < bits)
            {
                cheapest = Some((sink.bits, candidate));
            }
        }
        let Some((_, coded)) = cheapest else {
            return Ok(None);
        };
        if !self.preset.tip_amvd || !self.syntax.adaptive_mvd() {
            return Ok(Some(coded));
        }
        let amvd = InterPrediction::Single(InterModes {
            amvd: true,
            ..coded
        });
        let Some(amvd @ InterPrediction::Single(amvd_modes)) =
            codable(snap_prediction(self.syntax, leaf, amvd))?
        else {
            return Ok(Some(coded));
        };
        let plain = self.tip_cost(leaf, motion, InterPrediction::Single(coded))?;
        let adaptive = self.tip_cost(leaf, motion, amvd)?;
        Ok(Some(match (plain, adaptive) {
            (Some(plain), Some(adaptive)) if adaptive < plain => amvd_modes,
            (None, Some(_)) => amvd_modes,
            _ => coded,
        }))
    }

    /// The decoder luma prediction SATD plus mode rate of the TIP
    /// `prediction`, in SAD units; `None` when the leaf cannot code it.
    fn tip_cost(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        prediction: InterPrediction,
    ) -> Result<Option<u64>> {
        Ok(self
            .luma_score(leaf, prediction)?
            .map(|(satd, bits)| satd + motion.bits_cost(bits)))
    }
}
