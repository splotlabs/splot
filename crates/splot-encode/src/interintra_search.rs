// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter-intra search (§ 5.20.7.15): the cheapest single-reference `NEWMV`
//! of a leaf also tries each `interintra_mode` with the smooth mask, scored
//! by the decoder's own prediction.

use splot_decode::encoder_syntax::{INTERINTRA_MODES, InterIntra};

use super::*;

impl TrialCoder<'_, '_> {
    /// `base` with the `interintra_mode` whose decoder luma prediction SATD
    /// plus mode rate is lowest, when it is lower than that of `base`
    /// itself; `None` where the leaf reads no `inter_intra`.
    pub(super) fn interintra_choice(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        base: InterModes,
    ) -> Result<Option<InterModes>> {
        if !self.syntax.interintra_reads(leaf)?.0 {
            return Ok(None);
        }
        let mut best: Option<(u64, InterModes)> = None;
        for mode in (0..=INTERINTRA_MODES).map(|m| m.checked_sub(1)) {
            let modes = InterModes {
                interintra: mode.map(|mode| InterIntra { mode, wedge: None }),
                ..base
            };
            let prediction = InterPrediction::Single(modes);
            let Some(blocks) = codable(
                self.syntax
                    .luma_prediction(leaf, prediction)
                    .map_err(Into::into),
            )?
            else {
                continue;
            };
            let mut cost = 0;
            for (tx, samples) in &blocks {
                cost += self.coder.source.satd(tx, samples)?;
            }
            let mut sink = RateSink::adapting();
            self.syntax.price_inter_modes(leaf, prediction, &mut sink)?;
            cost += motion.bits_cost(sink.bits);
            if best.is_none_or(|(c, _)| cost < c) {
                best = Some((cost, modes));
            }
        }
        Ok(best
            .map(|(_, modes)| modes)
            .filter(|modes| modes.interintra.is_some()))
    }
}
