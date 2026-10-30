// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warp candidates of single-reference inter leaves: the decoder's own warp
//! models (§ 7.12 warp stack, § 7.13.3 derivations) from the facade, at the
//! leaf's translational search result. No model is fitted here.

use splot_decode::encoder_syntax::{
    EncoderSyntaxError, IntraTileSyntax, LeafBlock, MotionVector, WarpModel, WarpModes,
};

use crate::tile_writer::{InterPrediction, Result, snap_mv};

/// The warp predictions `leaf` can code from list reference `ref_frame`: the
/// first `WARPMV` candidate, and the `LOCALWARP`, `EXTENDWARP` and zero-delta
/// `DELTAWARP` models at the searched vector `found`.
pub(crate) fn candidates(
    syntax: &mut IntraTileSyntax,
    leaf: &LeafBlock,
    ref_frame: usize,
    found: MotionVector,
) -> Result<Vec<InterPrediction>> {
    let Some(options) = syntax.warp_options(leaf, ref_frame)? else {
        return Ok(Vec::new());
    };
    let mut models = vec![WarpModel::Candidate];
    if options.newmv {
        models.push(WarpModel::Delta {
            high_precision: false,
            values: [0; 4],
        });
    }
    if options.local {
        models.push(WarpModel::Local);
    }
    if options.extend {
        models.push(WarpModel::Extend);
    }
    let mut out = Vec::new();
    for model in models {
        let mut modes = WarpModes {
            ref_frame,
            model,
            warp_idx: 0,
            drl: 0,
            mv: None,
            precision: None,
        };
        if model != WarpModel::Candidate {
            let base = match syntax.warp_base(leaf, modes) {
                Ok(base) => base,
                Err(EncoderSyntaxError::Unsupported(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            modes.mv = Some(snap_mv(found, base, syntax.warp_step(&modes)));
        }
        out.push(InterPrediction::Warp(modes));
    }
    Ok(out)
}
