// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound and skip-mode inter leaves of the encoder facade (§ 5.20.7.10,
//! § 5.20.7.12) with `COMPOUND_AVERAGE` prediction: the masked, weighted,
//! warped and refined compound tools stay off at sequence level.

use splot_recon::InterpolationFilter;

use super::inter::{InterModes, MotionVector};
use super::{EncoderSyntaxError, IntraTileSyntax, LeafBlock, Result, rule};
use crate::prediction::inter::{CompoundYMode, EncoderCompoundMotion, EncoderMotion, Mv};

/// `JOINT_NEWMV_SCALE_FACTOR_CNT` (§ 3): the `jmvd_scale_mode` values a
/// `JOINT_NEWMV` block codes.
pub const JMVD_SCALE_MODES: u8 = crate::prediction::inter::JMVD_SCALE_MODES;

/// The candidate failure of a prediction whose vectors the decoder cannot derive.
const UNDERIVED: &str = "a prediction whose vectors the decoder cannot derive";

/// A compound `YMode` (§ 5.20.7.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CompoundMode {
    /// `NEAR_NEARMV`: the DRL candidate pair.
    NearNear,
    /// `NEAR_NEWMV`: list 1 codes a difference.
    NearNew,
    /// `NEW_NEARMV`: list 0 codes a difference.
    NewNear,
    /// `GLOBAL_GLOBALMV`: the global motion vectors.
    GlobalGlobal,
    /// `JOINT_NEWMV`: one difference, projected onto the other list.
    JointNew,
    /// `NEW_NEWMV`: both lists code a difference.
    NewNew,
}

/// Every [`CompoundMode`].
pub const COMPOUND_MODES: [CompoundMode; 6] = [
    CompoundMode::NearNear,
    CompoundMode::NearNew,
    CompoundMode::NewNear,
    CompoundMode::GlobalGlobal,
    CompoundMode::JointNew,
    CompoundMode::NewNew,
];

/// The compound prediction of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CompoundModes {
    /// The reference list indices, one of [`IntraTileSyntax::compound_pairs`].
    pub refs: (usize, usize),
    /// `YMode`.
    pub mode: CompoundMode,
    /// `RefMvIdx` in `drl[0]`; `NEAR_NEARMV` and `NEAR_NEWMV` read one index
    /// per list from the second DRL. `GLOBAL_GLOBALMV` ignores both.
    pub drl: [usize; 2],
    /// The block vectors of the lists that code a difference (see
    /// [`IntraTileSyntax::compound_bases`]); the other entries are ignored.
    pub mvs: [MotionVector; 2],
    /// `jmvd_scale_mode`, below [`JMVD_SCALE_MODES`]; only `JOINT_NEWMV`
    /// codes it.
    pub jmvd_scale_mode: u8,
    /// `interp_filter`, as [`InterModes::filter`]; a refined block codes
    /// none and predicts with `EIGHTTAP_SHARP`.
    pub filter: InterpolationFilter,
    /// `use_optflow`, where [`IntraTileSyntax::compound_refinements`] says the
    /// block reads it.
    pub optflow: bool,
    /// `use_refinemv`, where [`IntraTileSyntax::compound_refinements`] says
    /// the block reads it.
    pub refinemv: bool,
}

/// The prediction of one inter leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum InterPrediction {
    /// A single-reference block.
    Single(InterModes),
    /// A compound block.
    Compound(CompoundModes),
    /// A skip-mode block: `NEAR_NEARMV` over the `skip_mode_frames` pair with
    /// skip-DRL index `drl`.
    SkipMode {
        /// The skip-DRL index, below [`IntraTileSyntax::drl_count`].
        drl: usize,
    },
}

impl IntraTileSyntax {
    /// Whether `leaf` may code a compound block: an inter leaf of a
    /// `reference_select` frame that `is_comp_ref_allowed`.
    #[must_use]
    pub fn compound_allowed(&self, leaf: &LeafBlock) -> bool {
        self.compound_tool(leaf, |(reference_select, _)| reference_select)
    }

    /// Whether `leaf` may code a skip-mode block: an inter leaf of a
    /// `skip_mode_present` frame that `is_comp_ref_allowed`.
    #[must_use]
    pub fn skip_mode_allowed(&self, leaf: &LeafBlock) -> bool {
        self.compound_tool(leaf, |(_, skip_mode_present)| skip_mode_present)
    }

    /// The reference pairs a compound block can code (§ 5.20.7.11).
    #[must_use]
    pub fn compound_pairs(&self) -> Vec<(usize, usize)> {
        self.inter.as_ref().map_or_else(Vec::new, |inter| {
            inter
                .tile
                .compound_pairs()
                .into_iter()
                .map(|(first, second)| (first as usize, second as usize))
                .collect()
        })
    }

    /// The motion vectors the decoder derives for `prediction` at `leaf`;
    /// the second is zero for a single-reference block.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for a prediction `leaf`
    /// cannot code, including one whose vectors the decoder cannot derive.
    pub fn inter_mvs(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
    ) -> Result<[MotionVector; 2]> {
        let motion = self.motion(leaf, prediction)?;
        let tile = &mut self.inter_state()?.tile;
        Ok(tile
            .motion_vectors(&leaf.frontier, motion)
            .map_err(|_| EncoderSyntaxError::Unsupported(UNDERIVED))?
            .map(MotionVector::from))
    }

    /// The base vector of each list of `modes` that codes a difference at
    /// `leaf`: its vector is this plus a multiple of [`Self::mv_step`] per
    /// component. `JOINT_NEWMV` codes the difference of one list only.
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError::Unsupported`] for modes `leaf` cannot
    /// code, including modes whose vectors the decoder cannot derive.
    pub fn compound_bases(
        &mut self,
        leaf: &LeafBlock,
        modes: &CompoundModes,
    ) -> Result<[Option<MotionVector>; 2]> {
        let motion = self.compound_choice(leaf, modes)?;
        let tile = &mut self.inter_state()?.tile;
        let underived = |_| EncoderSyntaxError::Unsupported(UNDERIVED);
        let lists = tile.coded_lists(&motion).map_err(underived)?;
        let base = tile
            .motion_vectors(&leaf.frontier, EncoderMotion::Compound(motion))
            .map_err(underived)?;
        Ok([0, 1].map(|list| lists[list].then(|| base[list].into())))
    }

    /// The decoder's motion syntax of `prediction` at `leaf`.
    pub(super) fn motion(
        &mut self,
        leaf: &LeafBlock,
        prediction: InterPrediction,
    ) -> Result<EncoderMotion> {
        Ok(match prediction {
            InterPrediction::Single(modes) => {
                EncoderMotion::Single(self.single_motion(leaf, modes)?)
            }
            InterPrediction::Compound(modes) => {
                let mut motion = self.compound_choice(leaf, &modes)?;
                if motion.y_mode.has_newmv() {
                    let step = self.mv_step();
                    let targets = modes.mvs.map(Mv::from);
                    let tile = &mut self.inter_state()?.tile;
                    motion.mvd = tile
                        .compound_mvd(&leaf.frontier, motion, targets)
                        .map_err(rule("motion vector"))?;
                    if motion
                        .mvd
                        .iter()
                        .any(|d| d.row % step != 0 || d.col % step != 0)
                    {
                        return Err(EncoderSyntaxError::Unsupported(
                            "a NEWMV vector off the precision lattice",
                        ));
                    }
                }
                EncoderMotion::Compound(motion)
            }
            InterPrediction::SkipMode { drl } => {
                if !self.skip_mode_allowed(leaf) || drl >= self.drl_count() {
                    return Err(EncoderSyntaxError::Unsupported("a skip-mode block here"));
                }
                let tile = &self.inter_ref()?.tile;
                EncoderMotion::Compound(EncoderCompoundMotion {
                    ref_frames: tile
                        .skip_mode_pair(&leaf.frontier)
                        .map_err(rule("skip-mode references"))?,
                    y_mode: CompoundYMode::NearNear,
                    skip_mode: true,
                    ref_mv_idx: drl,
                    independent_idx: None,
                    mvd: [Mv::default(); 2],
                    jmvd_scale_mode: 0,
                    interp: InterpolationFilter::EightTapSharp,
                    use_optflow: false,
                    use_refinemv: false,
                })
            }
        })
    }

    /// The compound syntax of `modes` at `leaf` with zero differences,
    /// checked against the decoder's reads.
    fn compound_choice(
        &mut self,
        leaf: &LeafBlock,
        modes: &CompoundModes,
    ) -> Result<EncoderCompoundMotion> {
        let pair = (modes.refs.0, modes.refs.1);
        if !self.compound_allowed(leaf) || !self.compound_pairs().contains(&pair) {
            return Err(EncoderSyntaxError::Unsupported("a compound block here"));
        }
        let y_mode = match modes.mode {
            CompoundMode::NearNear => CompoundYMode::NearNear,
            CompoundMode::NearNew => CompoundYMode::NearNew,
            CompoundMode::NewNear => CompoundYMode::NewNear,
            CompoundMode::GlobalGlobal => CompoundYMode::GlobalGlobal,
            CompoundMode::JointNew => CompoundYMode::JointNew,
            CompoundMode::NewNew => CompoundYMode::NewNew,
        };
        let second_drl = !modes.optflow && y_mode.has_second_drl();
        let motion = EncoderCompoundMotion {
            ref_frames: (pair.0 as i8, pair.1 as i8),
            y_mode,
            skip_mode: false,
            ref_mv_idx: if second_drl || !y_mode.reads_drl_idx() {
                0
            } else {
                modes.drl[0]
            },
            independent_idx: second_drl.then_some(modes.drl),
            mvd: [Mv::default(); 2],
            jmvd_scale_mode: if y_mode == CompoundYMode::JointNew {
                modes.jmvd_scale_mode
            } else {
                0
            },
            interp: modes.filter,
            use_optflow: modes.optflow,
            use_refinemv: modes.refinemv,
        };
        let tile = &self.inter_ref()?.tile;
        tile.mode_symbols(&leaf.frontier, EncoderMotion::Compound(motion))
            .map_err(|_| {
                EncoderSyntaxError::Unsupported("compound modes this block cannot code")
            })?;
        Ok(motion)
    }

    /// Whether the compound block `modes` at `leaf` reads `use_optflow`, and
    /// whether it reads `use_refinemv` with the `use_optflow` of `modes`
    /// (§ 5.20.7.6, § 5.20.7.17).
    ///
    /// # Errors
    /// Returns [`EncoderSyntaxError`] for modes `leaf` cannot code.
    pub fn compound_refinements(
        &mut self,
        leaf: &LeafBlock,
        modes: CompoundModes,
    ) -> Result<(bool, bool)> {
        let plain = CompoundModes {
            optflow: false,
            refinemv: false,
            ..modes
        };
        let motion = EncoderCompoundMotion {
            use_optflow: modes.optflow,
            ..self.compound_choice(leaf, &plain)?
        };
        let refinement = self
            .inter_ref()?
            .tile
            .compound_refinement(&leaf.frontier, motion)
            .map_err(|_| EncoderSyntaxError::Unsupported("use_optflow on this block"))?;
        Ok((
            refinement.reads_optflow,
            refinement.refinemv_selector.is_some(),
        ))
    }

    fn compound_tool(&self, leaf: &LeafBlock, tool: impl Fn((bool, bool)) -> bool) -> bool {
        self.inter_allowed(leaf)
            && self.inter.as_ref().is_some_and(|inter| {
                inter
                    .tile
                    .reads_compound_syntax(&leaf.frontier)
                    .is_ok_and(tool)
            })
    }
}
