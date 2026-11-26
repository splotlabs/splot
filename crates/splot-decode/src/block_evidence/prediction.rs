// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::{
    DpcmDirection, InterIntraMode, InterpolationFilter, IntraCardinalDirection, PlaneRect,
};

pub use crate::bitstream::tile_payload::encoder_syntax::{InterMode, IntraMode};
use crate::bitstream::tile_payload::{
    CflMultiDirection, CflParams, GeneralIntraBlockModes, SupportedChromaMode,
};
pub use crate::prediction::inter::{CompoundMode, MotionVector};
use crate::residual::pipeline::{RectChromaPlan, RectLumaPlan};

/// Prediction facts from the decoded block syntax and its prediction plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockPrediction {
    /// Intra prediction from the current picture.
    Intra(BlockIntraPrediction),
    /// Inter prediction from one or more references.
    Inter(BlockInterPrediction),
    /// Prediction from a previously reconstructed region of the current picture.
    IntraBlockCopy,
    /// The decoder did not capture prediction facts for this block.
    Unavailable,
}

/// The intra prediction selected for each component of a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockIntraPrediction {
    /// Luma prediction. Absent in a chroma-only tree.
    pub luma: Option<BlockLumaPrediction>,
    /// Chroma prediction. Absent on blocks without a chroma predictor.
    pub chroma: Option<BlockChromaPrediction>,
}

/// The luma intra prediction selected by the decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockLumaPrediction {
    /// A conventional intra mode.
    Mode {
        /// The decoded luma mode.
        mode: IntraMode,
        /// Direction and reference-line facts for directional modes.
        direction: Option<BlockLumaDirection>,
        /// Lossless differential pulse-code modulation direction, if used.
        dpcm: Option<DpcmDirection>,
    },
    /// Prediction from a luma palette.
    Palette {
        /// Number of palette colors.
        color_count: u8,
    },
    /// DIP intra prediction.
    Dip {
        /// The DIP mode index.
        mode: u8,
        /// Whether the DIP predictor transposes its result.
        transpose: bool,
    },
}

/// Directional luma prediction and multiple-reference-line facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockLumaDirection {
    /// The decoded angle delta in steps of three degrees.
    pub angle_delta: i8,
    /// The mapped angle in degrees for the block prediction plan.
    /// Prediction units can use a different angle because of their own dimensions.
    pub angle_degrees: u16,
    /// The selected reference line index. Zero denotes the adjacent line.
    pub reference_line: u8,
    /// The above reference line index after superblock boundary adjustment.
    pub above_reference_line: u8,
    /// Whether the block prediction plan uses secondary reference-line blending.
    /// The decoder can disable this blend for individual prediction units.
    pub secondary_reference_line: bool,
}

/// The chroma intra prediction selected by the decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockChromaPrediction {
    /// A conventional intra mode.
    Mode {
        /// The decoded chroma mode.
        mode: IntraMode,
        /// The directional chroma angle delta in steps of three degrees.
        angle_delta: Option<i8>,
        /// Lossless differential pulse-code modulation direction, if used.
        dpcm: Option<DpcmDirection>,
    },
    /// Chroma-from-luma prediction.
    Cfl(BlockChromaFromLuma),
}

/// The decoder's chroma-from-luma parameter selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockChromaFromLuma {
    /// The block supplies alpha values for U and V.
    Explicit {
        /// Alpha for U.
        alpha_u: i8,
        /// Alpha for V.
        alpha_v: i8,
    },
    /// The decoder derives the alpha values.
    DerivedAlpha,
    /// Multi-hypothesis prediction with a direct spatial hypothesis.
    MultiDirect,
    /// Multi-hypothesis prediction with an above spatial hypothesis.
    MultiAbove,
    /// Multi-hypothesis prediction with a left spatial hypothesis.
    MultiLeft,
}

/// Inter prediction facts available before motion resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockInterPrediction {
    /// The decoded inter mode.
    pub mode: BlockInterMode,
    /// The available reference identities in predictor order.
    /// The second entry is absent for single-reference prediction.
    pub references: [Option<BlockReference>; 2],
    /// The block's motion-vector precision before any later refinement.
    pub precision: Option<BlockMotionPrecision>,
    /// Motion-vector differences decoded from syntax, in predictor order.
    /// Each component uses eighths of a luma sample.
    /// A projected joint difference on the other reference is absent.
    pub motion_differences: [Option<MotionVector>; 2],
    /// The interpolation filter selected for translational prediction.
    pub interpolation: InterpolationFilter,
    /// Intra mode of the inter-intra predictor, if present.
    pub inter_intra: Option<InterIntraMode>,
    /// Whether the block uses block-adaptive weighted prediction.
    pub bawp: bool,
    /// Whether the block uses the inter skip mode.
    pub skip_mode: bool,
    /// Whether the block uses adaptive motion-vector differences.
    pub adaptive_mvd: bool,
}

/// The block's inter prediction mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockInterMode {
    /// One reference predictor.
    Single(InterMode),
    /// Two reference predictors.
    Compound(CompoundMode),
    /// A warp mode with its model source.
    Warp(BlockWarpSource),
    /// A block refresh unit infers a zero motion vector.
    BlockRefresh,
}

/// Source of a warp model for a block in a warp mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockWarpSource {
    /// The decoder fits a model from local samples.
    Local,
    /// The decoder extends a model from a neighbor.
    Extended,
    /// The block codes a delta from a candidate model.
    Delta,
    /// The block uses a candidate model.
    Candidate,
}

/// The motion-vector precision that applies to the block syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockMotionPrecision {
    /// Eight whole luma samples.
    EightPel,
    /// Four whole luma samples.
    FourPel,
    /// Two whole luma samples.
    TwoPel,
    /// One luma sample.
    OnePel,
    /// One half of a luma sample.
    HalfPel,
    /// One quarter of a luma sample.
    QuarterPel,
    /// One eighth of a luma sample.
    EighthPel,
}

impl BlockMotionPrecision {
    pub(crate) const fn from_decoder(value: u8) -> Option<Self> {
        use crate::prediction::inter::read_mv::{
            MV_PRECISION_EIGHT_PEL, MV_PRECISION_EIGHTH_PEL, MV_PRECISION_FOUR_PEL,
            MV_PRECISION_HALF_PEL, MV_PRECISION_ONE_PEL, MV_PRECISION_QUARTER_PEL,
            MV_PRECISION_TWO_PEL,
        };

        Some(match value {
            MV_PRECISION_EIGHT_PEL => Self::EightPel,
            MV_PRECISION_FOUR_PEL => Self::FourPel,
            MV_PRECISION_TWO_PEL => Self::TwoPel,
            MV_PRECISION_ONE_PEL => Self::OnePel,
            MV_PRECISION_HALF_PEL => Self::HalfPel,
            MV_PRECISION_QUARTER_PEL => Self::QuarterPel,
            MV_PRECISION_EIGHTH_PEL => Self::EighthPel,
            _ => return None,
        })
    }
}

/// The identity of an inter predictor's reference within its coded frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockReference {
    /// A reference list entry and its slot in the decoded picture buffer.
    Frame {
        /// Index in the coded frame's reference list.
        list_index: u8,
        /// Slot at the time the decoder processed the coded frame.
        slot: u32,
        /// Whether the reference uses scaling, if dimensions are available.
        scaled: Option<bool>,
    },
    /// A synthetic temporal interpolated prediction reference.
    Tip,
}

/// Motion facts the decoder resolved for a block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockMotion {
    /// Resolved inter motion at block granularity.
    Inter(BlockInterMotion),
    /// Resolved copy motion within the current picture.
    IntraBlockCopy(BlockCopyMotion),
}

/// The resolved motion of an inter block before subblock refinement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockInterMotion {
    /// Resolved base vectors in predictor order, in eighths of a luma sample.
    pub vectors: [Option<MotionVector>; 2],
    /// Affine models passed to prediction in native AV2 WarpParams units.
    pub warp: [Option<[i32; 6]>; 2],
    /// Whether the predictor uses subblock derivation after this motion stage.
    pub refinement: BlockMotionRefinement,
    /// The resolved blend for an ordinary two-reference block, if present.
    pub blend: Option<BlockCompoundBlend>,
}

/// Subblock motion steps in the inter prediction plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockMotionRefinement {
    /// The decoder does not plan a subsequent subblock refinement.
    None,
    /// The predictor derives motion for a synthetic TIP reference.
    Tip,
    /// The predictor runs compound subblock derivation.
    Compound {
        /// Whether the plan enables refine-MV.
        refine_mv: bool,
        /// Whether the plan permits optical flow.
        optical_flow: bool,
    },
}

/// The compound blending operation and its resolved parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BlockCompoundBlend {
    /// Average, with optional implicit mask and compound weight.
    Average {
        /// Whether the blend uses an implicit mask.
        implicit_mask: bool,
        /// The native AV2 compound weight for the first predictor.
        weight: i16,
    },
    /// Difference-weighted mask.
    DifferenceWeighted {
        /// Whether the mask is inverted.
        inverse: bool,
    },
    /// Wedge mask.
    Wedge {
        /// The wedge index.
        index: u8,
        /// Whether the mask is inverted.
        sign: bool,
    },
}

/// A same-picture block vector and its direct luma source, if present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BlockCopyMotion {
    /// The resolved block vector in eighths of a luma sample.
    pub vector: MotionVector,
    /// The precision of the decoded block-vector difference, if present.
    pub precision: Option<BlockMotionPrecision>,
    /// The decoded block-vector difference in eighths of a luma sample, if present.
    pub difference: Option<MotionVector>,
    /// Whether morph prediction uses the copied samples.
    pub morph: bool,
    /// The exact source rectangle of an integer luma copy.
    /// A fractional block vector has no direct-copy rectangle.
    pub source: Option<PlaneRect>,
}

impl BlockLumaPrediction {
    #[cold]
    pub(crate) fn from_plan(plan: RectLumaPlan, modes: &GeneralIntraBlockModes) -> Self {
        let direction = match plan {
            RectLumaPlan::Palette { palette, .. } => {
                return Self::Palette {
                    color_count: palette.size() as u8,
                };
            }
            RectLumaPlan::Dip {
                mode, transpose, ..
            } => {
                return Self::Dip { mode, transpose };
            }
            RectLumaPlan::Cardinal { direction, .. } => {
                Some((cardinal_angle(direction), 0, 0, false))
            }
            RectLumaPlan::Middle { p_angle, .. }
            | RectLumaPlan::OneSidedAbove { p_angle, .. }
            | RectLumaPlan::OneSidedLeft { p_angle, .. } => Some((p_angle, 0, 0, false)),
            RectLumaPlan::CardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                ..
            } => Some((
                cardinal_angle(direction),
                mrl_index,
                above_mrl_index,
                secondary_mrl,
            )),
            RectLumaPlan::MiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                ..
            }
            | RectLumaPlan::OneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                ..
            }
            | RectLumaPlan::OneSidedLeftMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                ..
            } => Some((p_angle, mrl_index, above_mrl_index, secondary_mrl)),
            RectLumaPlan::Dc { .. } | RectLumaPlan::Paeth { .. } | RectLumaPlan::Smooth { .. } => {
                None
            }
        };
        Self::Mode {
            mode: modes.y_mode,
            direction: direction.map(
                |(
                    angle_degrees,
                    reference_line,
                    above_reference_line,
                    secondary_reference_line,
                )| {
                    BlockLumaDirection {
                        angle_delta: modes.angle_delta_y,
                        angle_degrees,
                        reference_line,
                        above_reference_line,
                        secondary_reference_line,
                    }
                },
            ),
            dpcm: modes.luma_dpcm_direction(),
        }
    }
}

const fn cardinal_angle(direction: IntraCardinalDirection) -> u16 {
    match direction {
        IntraCardinalDirection::Vertical => 90,
        IntraCardinalDirection::Horizontal => 180,
    }
}

impl BlockChromaPrediction {
    #[cold]
    pub(crate) fn from_plan(plan: RectChromaPlan) -> Self {
        let (mode, angle_delta, dpcm) = match plan {
            RectChromaPlan::Mode(mode, dpcm) => (mode, None, dpcm),
            RectChromaPlan::Directional {
                mode,
                angle_delta_uv,
                dpcm,
            } => (mode, Some(angle_delta_uv), dpcm),
            RectChromaPlan::Cfl { params, .. } => {
                return Self::Cfl(match params {
                    CflParams::Explicit { alpha_u, alpha_v } => {
                        BlockChromaFromLuma::Explicit { alpha_u, alpha_v }
                    }
                    CflParams::DerivedAlpha => BlockChromaFromLuma::DerivedAlpha,
                    CflParams::Multi { direction } => match direction {
                        CflMultiDirection::Direct => BlockChromaFromLuma::MultiDirect,
                        CflMultiDirection::Above => BlockChromaFromLuma::MultiAbove,
                        CflMultiDirection::Left => BlockChromaFromLuma::MultiLeft,
                    },
                });
            }
        };
        let mode = match mode {
            SupportedChromaMode::Dc => IntraMode::Dc,
            SupportedChromaMode::Smooth => IntraMode::Smooth,
            SupportedChromaMode::SmoothVertical => IntraMode::SmoothVertical,
            SupportedChromaMode::SmoothHorizontal => IntraMode::SmoothHorizontal,
            SupportedChromaMode::Paeth => IntraMode::Paeth,
            SupportedChromaMode::Vertical | SupportedChromaMode::VerticalFollow => {
                IntraMode::Vertical
            }
            SupportedChromaMode::Horizontal | SupportedChromaMode::HorizontalFollow => {
                IntraMode::Horizontal
            }
            SupportedChromaMode::D45 | SupportedChromaMode::D45Follow => IntraMode::D45,
            SupportedChromaMode::D67 | SupportedChromaMode::D67Follow => IntraMode::D67,
            SupportedChromaMode::D113 | SupportedChromaMode::D113Follow => IntraMode::D113,
            SupportedChromaMode::D135 | SupportedChromaMode::D135Follow => IntraMode::D135,
            SupportedChromaMode::D157 | SupportedChromaMode::D157Follow => IntraMode::D157,
            SupportedChromaMode::D203 | SupportedChromaMode::D203Follow => IntraMode::D203,
        };
        Self::Mode {
            mode,
            angle_delta,
            dpcm,
        }
    }
}
