// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compound type syntax of encoder leaves: `comp_group_idx`, `compound_type`,
//! the wedge, `wedge_sign` and `mask_type` (§ 5.20.7.16, § 5.20.7.18), and
//! `cwp_idx` (§ 5.20.7.6), gated by the decoder's own reads.

use super::super::compound_path::{
    CWP_WEIGHTING_FACTOR, CompoundBlendInput, CompoundBlendToolConfig, CompoundCwpInput,
    compound_cwp_same_side, compound_cwp_signal_allowed, compound_group_idx_context,
    compound_reads_group_idx, wedge_bits,
};
use super::super::warp::WEDGE_ANGLE_DIST_TO_INDEX;
use super::*;

/// `MAX_CWP_NUM` (§ 3): the `cwp_idx` values of a weighted block.
pub(crate) const MAX_CWP_NUM: u8 = 5;

/// The compound type of a compound leaf (§ 5.20.7.16) with its weight
/// (§ 5.20.7.6).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CompoundType {
    /// `COMPOUND_AVERAGE` with `cwp_idx` coding index `cwp`, below
    /// `MAX_CWP_NUM`; `0` is `CWP_EQUAL`.
    Average {
        /// The `Cwp_Weighting_Factor` column.
        cwp: u8,
    },
    /// `COMPOUND_WEDGE` with `WedgeIndex` `index` and `wedge_sign` `sign`.
    Wedge {
        /// `WedgeIndex`, an entry of `Wedge_Angle_Dist_2_Index`.
        index: u8,
        /// `wedge_sign`.
        sign: bool,
    },
    /// `COMPOUND_DIFFWTD` with `mask_type` `inverse`.
    DiffWeighted {
        /// `mask_type`.
        inverse: bool,
    },
}

impl Default for CompoundType {
    fn default() -> Self {
        Self::Average { cwp: 0 }
    }
}

/// The compound type syntax the decoder reads for one compound block.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CompoundTypeReads {
    /// The `comp_group_idx` context, where the block reads it.
    pub(crate) group_idx_ctx: Option<usize>,
    /// Whether a masked block may code `COMPOUND_WEDGE` (`Wedge_Bits != 0`).
    pub(crate) wedge: bool,
    /// `is_same_side()`, where an average block reads `cwp_idx`.
    pub(crate) cwp_same_side: Option<bool>,
}

impl CompoundTypeReads {
    /// The `Cwp_Weighting_Factor` row of the `cwp` index, where the block
    /// reads `cwp_idx`.
    pub(crate) fn cwp_weights(self) -> Option<[i16; MAX_CWP_NUM as usize]> {
        self.cwp_same_side
            .map(|same_side| CWP_WEIGHTING_FACTOR[usize::from(same_side)])
    }
}

/// One coded element of the compound type: an `S()` symbol, or with no
/// selector an `L(1)` literal.
pub(crate) type TypeSymbol = (Option<TileCdfSelector>, u8);

impl EncoderInterTile {
    /// Whether the sequence enables `cwp_idx` or masked compound.
    pub(super) fn codes_compound_types(&self) -> bool {
        self.sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_cwp || inter.enable_masked_compound)
    }

    /// The compound type syntax the decoder reads for `compound` at
    /// `frontier` whose refinement reads `use_refinemv` as `refinemv` with
    /// `switchable` (§ 5.20.7.16, § 5.20.7.6).
    pub(super) fn compound_type_reads(
        &self,
        frontier: &DecodeBlockFrontier,
        compound: EncoderCompoundMotion,
        (refinemv, switchable): (bool, bool),
    ) -> Result<CompoundTypeReads> {
        if !self.codes_compound_types() {
            return Ok(CompoundTypeReads::default());
        }
        let (n4w, n4h) = block_size4(frontier)?;
        let tools = CompoundBlendToolConfig::from_sequence(&self.sequence);
        let (ref0, ref1) = compound.ref_frames;
        let input = CompoundBlendInput {
            use_optflow: compound.use_optflow,
            joint_amvd: false,
            switchable_refinemv_on: refinemv && switchable,
            n4w,
            n4h,
            block_size_index: frontier.b_size.index(),
            comp_group_idx_ctx: 0,
        };
        let group_idx_ctx = if compound_reads_group_idx(tools, input) {
            let neighbour = block_neighbour_ctx(
                &self.grid,
                &self.block(frontier, (SINGLE_REF_FRAME0, None))?,
            );
            Some(compound_group_idx_context(
                &neighbour,
                &self.reference,
                &self.ref_frame_idx,
                &self.core,
                ref0,
                ref1,
                self.num_total_refs(),
            )?)
        } else {
            None
        };
        let cwp_enabled = self
            .sequence
            .inter
            .as_ref()
            .is_some_and(|inter| inter.enable_cwp);
        let cwp = CompoundCwpInput {
            y_mode: compound.y_mode,
            jmvd_scale_mode: compound.jmvd_scale_mode,
            use_optflow: compound.use_optflow,
            use_refinemv: refinemv,
            motion_simple: true,
            ref_frame0: ref0,
            ref_frame1: ref1,
            blend: mc::CompoundBlend::default(),
        };
        let cwp_same_side = compound_cwp_signal_allowed(cwp_enabled, cwp)
            .then(|| {
                compound_cwp_same_side(&self.core, &self.reference, &self.ref_frame_idx, ref0, ref1)
            })
            .transpose()?;
        Ok(CompoundTypeReads {
            group_idx_ctx,
            wedge: wedge_bits(frontier.b_size.index()) != 0,
            cwp_same_side,
        })
    }

    /// The decoder's blend of `compound` under `reads`.
    ///
    /// # Errors
    /// Returns the decoder's diagnostic for a type the block does not read.
    pub(super) fn compound_type_blend(
        &self,
        compound: EncoderCompoundMotion,
        reads: CompoundTypeReads,
    ) -> Result<mc::CompoundBlend> {
        let implicit = CompoundBlendToolConfig::from_sequence(&self.sequence).implicit_mask;
        let average = mc::CompoundBlend::average_with_implicit_mask(implicit);
        let masked = reads.group_idx_ctx.is_some();
        match compound.blend {
            CompoundType::Average { cwp: 0 } => Ok(average),
            CompoundType::Average { cwp } if cwp < MAX_CWP_NUM => {
                let same_side = reads.cwp_same_side.ok_or_else(invalid_type)?;
                Ok(average.average_with_cwp_weight(
                    CWP_WEIGHTING_FACTOR[usize::from(same_side)][usize::from(cwp)],
                ))
            }
            CompoundType::Wedge { index, sign } if masked && reads.wedge => {
                wedge_position(index)?;
                Ok(mc::CompoundBlend::Wedge { index, sign })
            }
            CompoundType::DiffWeighted { inverse } if masked => {
                Ok(mc::CompoundBlend::DiffWeighted { inverse })
            }
            _ => Err(invalid_type()),
        }
    }

    /// The elements `compound` codes for its type under `reads`, in coding
    /// order: `comp_group_idx`, `compound_type`, the wedge and `wedge_sign` or
    /// `mask_type`, then the `cwp_idx` symbols.
    ///
    /// # Errors
    /// Returns the decoder's diagnostic for a type the block does not read.
    pub(super) fn compound_type_symbols(
        &self,
        compound: EncoderCompoundMotion,
        reads: CompoundTypeReads,
    ) -> Result<Vec<TypeSymbol>> {
        self.compound_type_blend(compound, reads)?;
        let mut out = Vec::new();
        if let Some(ctx) = reads.group_idx_ctx {
            let masked = !matches!(compound.blend, CompoundType::Average { .. });
            out.push((
                Some(TileCdfSelector::CompGroupIdx { ctx }),
                u8::from(masked),
            ));
        }
        match compound.blend {
            CompoundType::Average { cwp } => {
                if reads.cwp_same_side.is_some() {
                    for idx in 0..usize::from(MAX_CWP_NUM - 1) {
                        let more = idx < usize::from(cwp);
                        out.push((Some(TileCdfSelector::CwpIdx { idx }), u8::from(more)));
                        if !more {
                            break;
                        }
                    }
                }
            }
            CompoundType::Wedge { index, sign } => {
                out.push((Some(TileCdfSelector::CompoundType), 0));
                wedge_symbols(&mut out, index)?;
                out.push((None, u8::from(sign)));
            }
            CompoundType::DiffWeighted { inverse } => {
                if reads.wedge {
                    out.push((Some(TileCdfSelector::CompoundType), 1));
                }
                out.push((None, u8::from(inverse)));
            }
        }
        Ok(out)
    }
}

/// `(wedgeAngle, wedgeDist)` of `WedgeIndex` `index` (§ 5.20.7.18).
fn wedge_position(index: u8) -> Result<(u8, u8)> {
    (0u8..)
        .zip(WEDGE_ANGLE_DIST_TO_INDEX)
        .find_map(|(angle, row)| {
            (0u8..)
                .zip(row)
                .find(|&(_, entry)| i16::from(entry) == i16::from(index))
                .map(|(dist, _)| (angle, dist))
        })
        .ok_or_else(invalid_type)
}

/// `wedge_quad`, `wedge_angle` and `wedge_dist1` or `wedge_dist2` of
/// `WedgeIndex` `index` (§ 5.20.7.18).
pub(super) fn wedge_symbols(out: &mut Vec<TypeSymbol>, index: u8) -> Result<()> {
    let (angle, dist) = wedge_position(index)?;
    let quad = angle / QUAD_WEDGE_ANGLES;
    out.push((Some(TileCdfSelector::WedgeQuad), quad));
    out.push((
        Some(TileCdfSelector::WedgeAngle {
            quad: usize::from(quad),
        }),
        angle % QUAD_WEDGE_ANGLES,
    ));
    if angle >= H_WEDGE_ANGLES || matches!(angle, WEDGE_90 | WEDGE_0) {
        out.push((Some(TileCdfSelector::WedgeDist2), dist - 1));
    } else {
        out.push((Some(TileCdfSelector::WedgeDist1), dist));
    }
    Ok(())
}

fn invalid_type() -> crate::DecodeError {
    DecodeHeaderStateError::InvalidInterBlockModeState.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `WedgeIndex` codes the `wedge_quad`, `wedge_angle` and distance
    /// symbols whose § 5.20.7.18 lookup is that index; 68 and above are
    /// refused.
    #[test]
    fn wedge_symbols_read_back_to_their_index() -> Result<()> {
        for index in 0..68u8 {
            let mut out = Vec::new();
            wedge_symbols(&mut out, index)?;
            assert_eq!(out.len(), 3, "{index}: {out:?}");
            let (quad, angle, (selector, dist)) = (out[0].1, out[1].1, out[2]);
            let angle = quad * QUAD_WEDGE_ANGLES + angle;
            let dist2 = selector == Some(TileCdfSelector::WedgeDist2);
            let dist = dist + u8::from(dist2);
            assert_eq!(
                dist2,
                angle >= H_WEDGE_ANGLES || matches!(angle, WEDGE_90 | WEDGE_0)
            );
            assert_eq!(
                WEDGE_ANGLE_DIST_TO_INDEX[usize::from(angle)][usize::from(dist)],
                index as i8
            );
        }
        assert!(wedge_symbols(&mut Vec::new(), 68).is_err());
        Ok(())
    }
}
