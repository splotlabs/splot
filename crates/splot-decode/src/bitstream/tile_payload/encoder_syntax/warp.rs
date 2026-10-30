// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warp leaves of the encoder facade: `WARPMV` and `WARP_NEWMV` with the
//! `LOCALWARP`, `EXTENDWARP` and `DELTAWARP` models (§ 5.20.7.11,
//! § 5.20.7.14, § 5.20.7.7). The decoder's § 7.12 warp stack and § 7.13.3
//! derivations build every model; the encoder only names the choice.

use splot_recon::{PlaneId, PlaneRect};

use super::inter::MotionVector;
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, rule};
use crate::prediction::inter::{EncoderMotion, EncoderWarpMotion, EncoderWarpSource, Mv};

/// `MAX_WARP_REF_CANDIDATES` (§ 3): the `RefWarpIdx` values.
pub const WARP_CANDIDATES: usize = 4;
/// The largest low-precision `warp_delta_param` magnitude (§ 5.20.7.7).
pub const WARP_DELTA_LOW_MAX: i32 = 7;
/// The largest high-precision `warp_delta_param` magnitude (§ 5.20.7.7).
pub const WARP_DELTA_HIGH_MAX: i32 = 14;

/// How a warp leaf builds its model.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum WarpModel {
    /// `WARPMV`: the `RefWarpIdx` candidate supplies model and vector.
    Candidate,
    /// `WARP_NEWMV` with `LOCALWARP`: least squares over the neighbours.
    Local,
    /// `WARP_NEWMV` with `EXTENDWARP`: the neighbour model extended.
    Extend,
    /// `WARP_NEWMV` with `DELTAWARP`: the `RefWarpIdx` candidate plus the
    /// coded `warp_delta_param` values of model indices 2 to 5, each at most
    /// [`WARP_DELTA_LOW_MAX`] or, with `high_precision`,
    /// [`WARP_DELTA_HIGH_MAX`]. Only `RefWarpIdx` 0, and 1 with
    /// [`WarpOptions::six_param`], code values; indices 4 and 5 need
    /// `RefWarpIdx` 1. Other candidates take zeros.
    Delta {
        /// `warp_delta_precision`.
        high_precision: bool,
        /// The signed coded values.
        values: [i32; 4],
    },
}

/// The warp prediction of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct WarpModes {
    /// The reference list index, below [`IntraTileSyntax::reference_count`].
    pub ref_frame: usize,
    /// The model.
    pub model: WarpModel,
    /// `RefWarpIdx` of [`WarpModel::Candidate`] and [`WarpModel::Delta`],
    /// below [`WARP_CANDIDATES`]; the other models take `0`.
    pub warp_idx: usize,
    /// `RefMvIdx` of a `WARP_NEWMV`, below [`IntraTileSyntax::drl_count`];
    /// `WARPMV` takes `0`.
    pub drl: usize,
    /// The block vector: a `WARP_NEWMV` codes it as [`IntraTileSyntax::warp_base`]
    /// plus a multiple of its precision step; a `WARPMV` with `Some` codes
    /// `warpmv_with_mvd` (`RefWarpIdx` below 2) at the frame step.
    pub mv: Option<MotionVector>,
    /// The block `MvPrecision` of a `WARP_NEWMV`, one of
    /// [`IntraTileSyntax::newmv_precisions`]; `None` codes the frame precision.
    pub precision: Option<u8>,
}

/// The warp tools a leaf can code.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WarpOptions {
    /// `WARP_NEWMV`; a `force_integer_mv` frame codes `WARPMV` only.
    pub newmv: bool,
    /// [`WarpModel::Local`].
    pub local: bool,
    /// [`WarpModel::Extend`].
    pub extend: bool,
    /// `enable_six_param_warp_delta`.
    pub six_param: bool,
}

impl IntraTileSyntax {
    /// The warp tools `leaf` can code from list reference `ref_frame`;
    /// `None` where it codes no warp block.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for a reference `leaf` cannot code.
    pub fn warp_options(&self, leaf: &LeafBlock, ref_frame: usize) -> Result<Option<WarpOptions>> {
        if !self.inter_allowed(leaf) {
            return Ok(None);
        }
        let ref_frame = self.list_reference(ref_frame)?;
        Ok(self
            .inter_ref()?
            .tile
            .warp_options(&leaf.frontier, ref_frame)
            .map_err(rule("warp options"))?
            .map(|options| WarpOptions {
                newmv: options.newmv,
                local: options.local,
                extend: options.extend,
                six_param: options.six_param,
            }))
    }

    /// The vector a difference of `modes` at `leaf` adds to: the DRL
    /// candidate of a `WARP_NEWMV` at its precision, the candidate vector of a
    /// `WARPMV` at the frame precision.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code.
    pub fn warp_base(&mut self, leaf: &LeafBlock, modes: WarpModes) -> Result<MotionVector> {
        let motion = EncoderMotion::Warp(EncoderWarpMotion {
            mvd: Some(Mv::default()),
            ..self.warp_choice(leaf, modes)?
        });
        let tile = &mut self.inter_state()?.tile;
        Ok(tile
            .motion_vectors(&leaf.frontier, motion)
            .map_err(rule("warp vector"))?[0]
            .into())
    }

    /// The difference step of `modes` in `1/8` samples: the frame step of a
    /// `WARPMV`, the step of its block precision of a `WARP_NEWMV`.
    #[must_use]
    pub fn warp_step(&self, modes: &WarpModes) -> i32 {
        let precision = match modes.model {
            WarpModel::Candidate => None,
            _ => modes.precision,
        };
        1 << super::MV_PRECISION_EIGHTH_PEL
            .saturating_sub(precision.unwrap_or_else(|| self.mv_precision()))
    }

    /// The decoder's row-major luma prediction of `modes` at `leaf`, for a
    /// warp search. The prediction overwrites the leaf's samples and drops
    /// the planned leaf.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code or a leaf
    /// that leaves the frame storage.
    pub fn warp_prediction(&mut self, leaf: &LeafBlock, modes: WarpModes) -> Result<Vec<u16>> {
        let motion = EncoderMotion::Warp(self.warp_motion(leaf, modes)?);
        self.clear_inter_plan();
        let (width, height) = leaf.dimensions()?;
        let rect = PlaneRect::new(leaf.frontier.c * 4, leaf.frontier.r * 4, width, height)
            .map_err(rule("warp prediction rect"))?;
        let state = self.inter.as_mut().ok_or(EncoderSyntaxError::Unsupported(
            "inter syntax in an intra frame",
        ))?;
        state
            .tile
            .predict(&leaf.frontier, motion, &mut self.recon.workspace)
            .map_err(rule("warp prediction"))?;
        let mut out = Vec::with_capacity(width * height);
        for row in self
            .recon
            .workspace
            .rect_rows(PlaneId::Y, rect)
            .map_err(|_| EncoderSyntaxError::Unsupported("a warp leaf outside the frame"))?
        {
            out.extend_from_slice(row);
        }
        Ok(out)
    }

    /// The decoder's warp syntax of `modes` at `leaf`, its symbols checked.
    pub(super) fn warp_motion(
        &mut self,
        leaf: &LeafBlock,
        modes: WarpModes,
    ) -> Result<EncoderWarpMotion> {
        let mut motion = self.warp_choice(leaf, modes)?;
        if let Some(target) = modes.mv {
            let base = self.warp_base(leaf, modes)?;
            let step = self.warp_step(&modes);
            let mvd = Mv {
                row: target.row - base.row,
                col: target.col - base.col,
            };
            if mvd.row % step != 0 || mvd.col % step != 0 {
                return Err(EncoderSyntaxError::Unsupported(
                    "a warp vector off the precision lattice",
                ));
            }
            motion.mvd = Some(mvd);
        }
        Ok(motion)
    }

    /// The warp syntax of `modes` with a zero difference, checked against the
    /// decoder's reads.
    fn warp_choice(&self, leaf: &LeafBlock, modes: WarpModes) -> Result<EncoderWarpMotion> {
        if !self.inter_allowed(leaf) {
            return Err(EncoderSyntaxError::Unsupported("a warp block here"));
        }
        let source = match modes.model {
            WarpModel::Candidate => EncoderWarpSource::Candidate,
            WarpModel::Local => EncoderWarpSource::Local,
            WarpModel::Extend => EncoderWarpSource::Extend,
            WarpModel::Delta {
                high_precision,
                values,
            } => EncoderWarpSource::Delta {
                high_precision,
                values,
            },
        };
        let newmv = modes.model != WarpModel::Candidate;
        let motion = EncoderWarpMotion {
            ref_frame: self.list_reference(modes.ref_frame)?,
            source,
            ref_mv_idx: modes.drl,
            ref_warp_idx: modes.warp_idx,
            mvd: (newmv || modes.mv.is_some()).then_some(Mv::default()),
            precision: modes.precision,
        };
        let tile = &self.inter_ref()?.tile;
        let checked = tile
            .mode_symbols(&leaf.frontier, EncoderMotion::Warp(motion))
            .and_then(|_| tile.warp_tail_symbols(&leaf.frontier, motion));
        checked
            .map_err(|_| EncoderSyntaxError::Unsupported("warp modes this block cannot code"))?;
        Ok(motion)
    }

    /// The decoder's list reference of index `ref_frame`.
    fn list_reference(&self, ref_frame: usize) -> Result<i8> {
        i8::try_from(ref_frame)
            .ok()
            .filter(|_| ref_frame < self.reference_count())
            .ok_or(EncoderSyntaxError::Unsupported("reference index"))
    }
}
