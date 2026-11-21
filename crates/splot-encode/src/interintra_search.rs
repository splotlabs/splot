// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter-intra (§ 5.20.7.15) and block adaptive weighted prediction
//! (§ 5.20.7.6) search: the cheapest single-reference `NEWMV` of a leaf
//! (and, with `bawp_search` 2, each `NEARMV`) also tries each smooth-mask
//! `interintra_mode` and each BAWP scale, scored by the decoder's own
//! prediction, and the best of each joins the RD trials.

use splot_decode::encoder_syntax::{Bawp, INTERINTRA_MODES, InterIntra};

use super::*;

impl TrialCoder<'_, '_> {
    /// `base` with the `interintra_mode` whose decoder luma prediction SATD
    /// plus mode rate is lowest, for the full RD trial of the leaf; `None`
    /// where the leaf reads no `inter_intra`.
    pub(super) fn interintra_choice(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        base: InterModes,
    ) -> Result<Option<InterModes>> {
        if !self.syntax.interintra_reads(leaf)?.0 {
            return Ok(None);
        }
        let variants = (0..INTERINTRA_MODES).map(|mode| InterModes {
            interintra: Some(InterIntra { mode, wedge: None }),
            ..base
        });
        self.cheapest_variant(leaf, motion, variants)
    }

    /// `base` with the BAWP scale (implicit, or explicit up or down) whose
    /// decoder luma prediction SATD plus mode rate is lowest, with chroma
    /// weighting where the block has chroma, for the full RD trial of the
    /// leaf; with `preset.bawp_search` also that scale without chroma
    /// weighting (`use_bawp_chroma = 0`), which luma cannot rank. Empty
    /// where the leaf reads no `use_bawp`.
    pub(super) fn bawp_choice(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        base: InterModes,
    ) -> Result<Vec<InterModes>> {
        if !codable(self.syntax.bawp_reads(leaf, base).map_err(Into::into))?.unwrap_or(false) {
            return Ok(Vec::new());
        }
        let variants = [None, Some(true), Some(false)].map(|explicit_scale| InterModes {
            bawp: Some(Bawp {
                explicit_scale,
                chroma: leaf.has_chroma(),
            }),
            ..base
        });
        let Some(best) = self.cheapest_variant(leaf, motion, variants)? else {
            return Ok(Vec::new());
        };
        let mut out = vec![best];
        if let (true, Some(bawp)) = (self.preset.bawp_search > 0, best.bawp)
            && bawp.chroma
        {
            out.push(InterModes {
                bawp: Some(Bawp {
                    chroma: false,
                    ..bawp
                }),
                ..best
            });
        }
        Ok(out)
    }

    /// The codable variant of `variants` whose decoder luma prediction SATD
    /// plus mode rate is lowest.
    fn cheapest_variant(
        &mut self,
        leaf: &LeafBlock,
        motion: &MotionSearch,
        variants: impl IntoIterator<Item = InterModes>,
    ) -> Result<Option<InterModes>> {
        let mut best: Option<(u64, InterModes)> = None;
        for modes in variants {
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
        Ok(best.map(|(_, modes)| modes))
    }
}
