// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warp search of single-reference inter leaves and the frame-level warp
//! gate.
//!
//! Every candidate model is the decoder's own (§ 7.12 warp stack, § 7.13.3
//! derivations) through the facade; no model is fitted here. The search
//! scores the `WARPMV` candidates, the local, extended and zero-delta models
//! at the leaf's translational vector, then moves each `DELTAWARP` parameter
//! by one step, and the vector of the best `WARP_NEWMV` model by up to two
//! steps, where that lowers the luma SATD plus exact rate. The best models
//! that beat the translational cost enter the leaf's RD trials.
//!
//! The gate keeps, per temporal layer, a decaying count of warp leaves and
//! inter leaves of the frames that coded warp, and turns warp off for a
//! layer whose frames rarely pick it; such a layer probes again every
//! [`WARP_PROBE_FRAMES`] frames.

use splot_decode::encoder_syntax::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, MotionVector, WARP_CANDIDATES,
    WARP_DELTA_LOW_MAX, WarpModel, WarpModes,
};

use crate::motion_search::MotionSearch;
use crate::rd::coeff_rate::RateSink;
use crate::tile_writer::{InterPrediction, PartitionTree, Result, snap_mv};

/// The warp leaves per thousand inter leaves below which a layer stops coding
/// warp.
const WARP_MIN_PERMILLE: usize = 10;
/// The inter leaves a layer needs before the gate trusts its ratio.
const WARP_MIN_INTER: usize = 64;
/// A gated-off layer codes warp again after this many frames.
pub(crate) const WARP_PROBE_FRAMES: u8 = 4;
/// Temporal layers the gate tracks; deeper layers share the last entry.
const WARP_LAYERS: usize = 4;

/// The warp predictions of `leaf` from list reference `ref_frame` whose luma
/// SATD plus rate beats `beat`, the cost of the leaf's translational `NEWMV`
/// at `found`, best first and at most `keep`.
pub(crate) fn search(
    syntax: &mut IntraTileSyntax,
    motion: &MotionSearch,
    leaf: &LeafBlock,
    (ref_frame, found, beat): (usize, MotionVector, u64),
    keep: usize,
) -> Result<Vec<InterPrediction>> {
    let Some(options) = syntax.warp_options(leaf, ref_frame)? else {
        return Ok(Vec::new());
    };
    let mut scorer = Scorer {
        syntax,
        motion,
        leaf,
        rect: crate::mode_decision::leaf_rect(leaf)?,
        found,
        scored: Vec::new(),
    };
    let base = WarpModes {
        ref_frame,
        model: WarpModel::Candidate,
        warp_idx: 0,
        drl: 0,
        mv: None,
        precision: None,
    };
    for warp_idx in 0..WARP_CANDIDATES {
        scorer.score(WarpModes { warp_idx, ..base })?;
    }
    for warp_idx in 0..2 {
        scorer.score(WarpModes {
            warp_idx,
            mv: Some(found),
            ..base
        })?;
    }
    if options.newmv {
        let zero = WarpModel::Delta {
            high_precision: false,
            values: [0; 4],
        };
        let mut newmv = vec![(zero, 0)];
        if options.six_param {
            newmv.push((zero, 1));
        }
        if options.local {
            newmv.push((WarpModel::Local, 0));
        }
        if options.extend {
            newmv.push((WarpModel::Extend, 0));
        }
        for (model, warp_idx) in newmv {
            scorer.score(WarpModes {
                model,
                warp_idx,
                mv: Some(found),
                ..base
            })?;
        }
        scorer.refine_delta(options.six_param)?;
        scorer.refine_vector()?;
    }
    let mut scored = scorer.scored;
    scored.sort_by_key(|&(cost, _)| cost);
    let mut out: Vec<InterPrediction> = Vec::new();
    for (_, modes) in scored.into_iter().filter(|&(cost, _)| cost < beat) {
        let prediction = InterPrediction::Warp(modes);
        if out.len() < keep && !out.contains(&prediction) {
            out.push(prediction);
        }
    }
    Ok(out)
}

/// The scored warp candidates of one leaf.
struct Scorer<'a, 'b> {
    syntax: &'a mut IntraTileSyntax,
    motion: &'b MotionSearch,
    leaf: &'b LeafBlock,
    rect: (usize, usize, usize, usize),
    found: MotionVector,
    scored: Vec<(u64, WarpModes)>,
}

impl Scorer<'_, '_> {
    /// Scores `modes`, its vector snapped to its lattice, by luma SATD plus
    /// the rate of its mode, difference and warp syntax; `None` where the
    /// leaf cannot code it.
    fn score(&mut self, mut modes: WarpModes) -> Result<Option<u64>> {
        match self.try_score(&mut modes) {
            Ok(cost) => {
                self.scored.push((cost, modes));
                Ok(Some(cost))
            }
            Err(EncoderSyntaxError::Unsupported(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn try_score(
        &mut self,
        modes: &mut WarpModes,
    ) -> core::result::Result<u64, EncoderSyntaxError> {
        if let Some(target) = modes.mv {
            let base = self.syntax.warp_base(self.leaf, *modes)?;
            modes.mv = Some(snap_mv(target, base, self.syntax.warp_step(modes)));
        }
        let prediction = InterPrediction::Warp(*modes);
        let mut sink = RateSink::adapting();
        self.syntax
            .price_inter_modes(self.leaf, prediction, &mut sink)?;
        self.syntax
            .price_interp_filter(self.leaf, prediction, &mut sink)?;
        let predicted = self.syntax.warp_prediction(self.leaf, *modes)?;
        let satd = self
            .motion
            .satd(self.rect, &predicted)
            .map_err(|_| EncoderSyntaxError::Unsupported("a warp leaf outside the source"))?;
        Ok(satd + self.motion.bits_cost(sink.bits))
    }

    /// Moves the vector of the cheapest `WARP_NEWMV` candidate by one
    /// difference step to the neighbour that lowers its cost, twice.
    fn refine_vector(&mut self) -> Result<()> {
        let Some(&(mut best, mut modes)) = self
            .scored
            .iter()
            .filter(|(_, modes)| modes.model != WarpModel::Candidate)
            .min_by_key(|&&(cost, _)| cost)
        else {
            return Ok(());
        };
        let step = self.syntax.warp_step(&modes);
        for _ in 0..2 {
            let Some(center) = modes.mv else {
                return Ok(());
            };
            let mut improved = None;
            for (dr, dc) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (-1, 1),
                (1, -1),
                (1, 1),
            ] {
                let candidate = WarpModes {
                    mv: Some(MotionVector {
                        row: center.row + dr * step,
                        col: center.col + dc * step,
                    }),
                    ..modes
                };
                if let Some(cost) = self.score(candidate)?
                    && cost < best
                {
                    (best, improved) = (cost, Some(candidate));
                }
            }
            match improved {
                Some(candidate) => modes = candidate,
                None => return Ok(()),
            }
        }
        Ok(())
    }

    /// Moves each coded parameter of the cheapest `DELTAWARP` candidate by
    /// one low-precision step where that lowers its cost: two parameters,
    /// four with `six_param` on `RefWarpIdx` 1.
    fn refine_delta(&mut self, six_param: bool) -> Result<()> {
        let Some(&(mut best, mut modes)) = self
            .scored
            .iter()
            .filter(|(_, modes)| matches!(modes.model, WarpModel::Delta { .. }))
            .min_by_key(|&&(cost, _)| cost)
        else {
            return Ok(());
        };
        let count = if six_param && modes.warp_idx == 1 {
            4
        } else {
            2
        };
        modes.mv = Some(self.found);
        for axis in 0..count {
            let WarpModel::Delta { values, .. } = modes.model else {
                return Ok(());
            };
            let mut improved = None;
            for step in [-1, 1] {
                let mut moved = values;
                moved[axis] += step;
                if moved[axis].abs() > WARP_DELTA_LOW_MAX {
                    continue;
                }
                let candidate = WarpModes {
                    model: WarpModel::Delta {
                        high_precision: false,
                        values: moved,
                    },
                    ..modes
                };
                if let Some(cost) = self.score(candidate)?
                    && cost < best
                {
                    (best, improved) = (cost, Some(candidate));
                }
            }
            if let Some(candidate) = improved {
                modes = candidate;
            }
        }
        Ok(())
    }
}

/// How one frame used warp: its temporal layer, whether it coded warp, and
/// its warp leaves and inter leaves.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WarpUse {
    pub(crate) layer: u8,
    pub(crate) enabled: bool,
    pub(crate) leaves: (usize, usize),
}

/// The frame-level warp gate: per layer, the decayed warp and inter leaf
/// counts of the frames that coded warp and the frames since one did.
#[derive(Clone, Debug, Default)]
pub(crate) struct WarpGate {
    layers: [(usize, usize, u8); WARP_LAYERS],
}

impl WarpGate {
    /// Whether a frame of `layer` codes warp.
    pub(crate) fn allows(&self, layer: u8) -> bool {
        let (warp, inter, skipped) = self.layers[usize::from(layer).min(WARP_LAYERS - 1)];
        inter < WARP_MIN_INTER
            || warp * 1000 >= inter * WARP_MIN_PERMILLE
            || skipped >= WARP_PROBE_FRAMES
    }

    /// Adds the frame `used`.
    pub(crate) fn record(&mut self, used: WarpUse) {
        let (warp, inter, skipped) = &mut self.layers[usize::from(used.layer).min(WARP_LAYERS - 1)];
        if used.enabled {
            *warp = *warp / 2 + used.leaves.0;
            *inter = *inter / 2 + used.leaves.1;
            *skipped = 0;
        } else {
            *skipped = skipped.saturating_add(1);
        }
    }
}

/// The warp leaves and the inter leaves of `tree`.
pub(crate) fn leaf_counts(tree: &PartitionTree) -> (usize, usize) {
    match tree {
        PartitionTree::Leaf(choice) => (
            usize::from(matches!(choice.inter, Some(InterPrediction::Warp(_)))),
            usize::from(choice.inter.is_some()),
        ),
        PartitionTree::Split { children, .. } => children
            .iter()
            .map(leaf_counts)
            .fold((0, 0), |(w, i), (dw, di)| (w + dw, i + di)),
        PartitionTree::Sdp { luma, chroma } => {
            let (a, b) = (leaf_counts(luma), leaf_counts(chroma));
            (a.0 + b.0, a.1 + b.1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer keeps warp while it wins, stops when it does not, and probes
    /// again after [`WARP_PROBE_FRAMES`] frames without warp.
    #[test]
    fn the_gate_follows_the_warp_win_ratio() {
        let mut gate = WarpGate::default();
        assert!(gate.allows(1));
        let frame = |enabled, leaves| WarpUse {
            layer: 1,
            enabled,
            leaves,
        };
        gate.record(frame(true, (20, 200)));
        assert!(gate.allows(1) && gate.allows(0));
        gate.record(frame(true, (0, 400)));
        gate.record(frame(true, (0, 400)));
        assert!(!gate.allows(1));
        assert!(gate.allows(2), "layers keep their own counts");
        for _ in 1..WARP_PROBE_FRAMES {
            gate.record(frame(false, (0, 0)));
            assert!(!gate.allows(1));
        }
        gate.record(frame(false, (0, 0)));
        assert!(gate.allows(1), "probe");
        gate.record(frame(true, (0, 400)));
        assert!(!gate.allows(1));
    }
}
