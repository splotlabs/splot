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
use splot_recon::InterIntraMode;

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

/// The most elements one compound type codes: `comp_group_idx`,
/// `compound_type`, three wedge symbols and `wedge_sign`.
const MAX_TYPE_SYMBOLS: usize = 6;

/// The compound type elements of one block, in coding order, without a heap
/// allocation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TypeSymbols {
    items: [TypeSymbol; MAX_TYPE_SYMBOLS],
    len: usize,
}

impl TypeSymbols {
    fn push(&mut self, item: TypeSymbol) {
        if let Some(slot) = self.items.get_mut(self.len) {
            *slot = item;
            self.len += 1;
        }
    }
}

impl IntoIterator for TypeSymbols {
    type Item = TypeSymbol;
    type IntoIter = core::iter::Take<core::array::IntoIter<TypeSymbol, MAX_TYPE_SYMBOLS>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter().take(self.len)
    }
}

impl EncoderInterTile {
    /// Whether the sequence enables `cwp_idx` or masked compound.
    pub(super) const fn codes_compound_types(&self) -> bool {
        self.blend_tools.0
    }

    /// The `COMPOUND_AVERAGE` blend of the sequence (§ 7.13.3.14).
    pub(super) const fn average_blend(&self) -> mc::CompoundBlend {
        mc::CompoundBlend::average_with_implicit_mask(self.blend_tools.1)
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
        let average = self.average_blend();
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
    ) -> Result<TypeSymbols> {
        self.compound_type_blend(compound, reads)?;
        let mut out = TypeSymbols::default();
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

impl EncoderInterTile {
    /// Whether a single-reference block at `frontier` reads `inter_intra`,
    /// and whether it reads `wedge_interintra` (§ 5.20.7.14, § 5.20.7.15).
    pub(crate) fn interintra_reads(&self, frontier: &DecodeBlockFrontier) -> Result<(bool, bool)> {
        let (n4w, n4h) = block_size4(frontier)?;
        let b_size = frontier.b_size.index();
        let enabled = self
            .core
            .inter
            .as_ref()
            .and_then(|inter| inter.frame_enabled_motion_modes)
            .is_some_and(|modes| modes[splot_core::headers::frame::INTERINTRA]);
        let reads = enabled
            && b_size >= super::super::BLOCK_8X8
            && n4w.max(n4h) <= super::super::CHUNK_64_N4;
        Ok((reads, reads && super::super::WEDGE_USED_BY_BSIZE[b_size]))
    }

    /// Pushes the `inter_intra`, `interintra_mode`, `wedge_interintra` and
    /// wedge symbols of a single-reference block at `frontier` with
    /// `interintra` (§ 5.20.7.15).
    ///
    /// # Errors
    /// Returns the decoder's diagnostic for an inter-intra block the decoder
    /// does not read.
    pub(super) fn interintra_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        frontier: &DecodeBlockFrontier,
        interintra: Option<InterIntraPrediction>,
    ) -> Result<()> {
        let (reads, reads_wedge) = self.interintra_reads(frontier)?;
        if !reads {
            return match interintra {
                Some(_) => Err(invalid_type()),
                None => Ok(()),
            };
        }
        let bsize_group = super::super::SIZE_GROUP_LOOKUP[frontier.b_size.index()];
        out.push((
            TileCdfSelector::InterIntra { bsize_group },
            u8::from(interintra.is_some()),
        ));
        let Some(prediction) = interintra else {
            return Ok(());
        };
        let mode = match prediction.mode() {
            InterIntraMode::Dc => 0,
            InterIntraMode::Vertical => 1,
            InterIntraMode::Horizontal => 2,
            InterIntraMode::Smooth => 3,
        };
        out.push((TileCdfSelector::InterIntraMode { bsize_group }, mode));
        let wedge = match prediction {
            InterIntraPrediction::WedgeMask { wedge_index, .. } => Some(wedge_index),
            InterIntraPrediction::SmoothMask { .. } => None,
        };
        if reads_wedge {
            out.push((TileCdfSelector::WedgeInterIntra, u8::from(wedge.is_some())));
        }
        if let Some(index) = wedge {
            if !reads_wedge {
                return Err(invalid_type());
            }
            let mut symbols = TypeSymbols::default();
            wedge_symbols(&mut symbols, index)?;
            out.extend(
                symbols
                    .into_iter()
                    .filter_map(|(selector, value)| selector.map(|selector| (selector, value))),
            );
        }
        Ok(())
    }
}

impl EncoderInterTile {
    /// The § 5.20.7.6 parse input of the BAWP syntax of `single` at
    /// `frontier`.
    fn bawp_input(
        &self,
        frontier: &DecodeBlockFrontier,
        single: EncoderSingleMotion,
    ) -> Result<super::super::BawpParseInput> {
        let (n4w, n4h) = block_size4(frontier)?;
        let (avail_up, avail_left) = super::super::tile_neighbour_availability(
            frontier.r,
            frontier.c,
            self.tile_start.0,
            self.tile_start.1,
        );
        Ok(super::super::BawpParseInput {
            allow_bawp: !single.tip
                && self
                    .core
                    .inter_tail
                    .as_ref()
                    .is_some_and(|tail| tail.allow_bawp),
            reference_scaled: !single.tip
                && super::super::block_reference_is_scaled(
                    &self.core,
                    &self.reference,
                    &self.ref_frame_idx,
                    single.ref_frame,
                )?,
            frame_is_switch: self.core.frame_type == Some(FrameType::Switch),
            single_mode: single.mode,
            use_amvd: single.use_amvd,
            n4w,
            n4h,
            has_chroma: frontier.has_chroma,
            avail_up,
            avail_left,
        })
    }

    /// Whether the single-reference block `single` at `frontier` reads
    /// `use_bawp` (§ 5.20.7.6).
    pub(crate) fn bawp_reads(
        &self,
        frontier: &DecodeBlockFrontier,
        single: EncoderSingleMotion,
    ) -> Result<bool> {
        Ok(super::super::bawp_signal_allowed(
            self.bawp_input(frontier, single)?,
        ))
    }

    /// Pushes the `use_bawp`, `explicit_bawp`, `explicit_bawp_scale` and
    /// `use_bawp_chroma` symbols of `single` at `frontier` (§ 5.20.7.6).
    ///
    /// # Errors
    /// Returns the decoder's diagnostic for BAWP the decoder does not read.
    pub(super) fn bawp_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        frontier: &DecodeBlockFrontier,
        single: EncoderSingleMotion,
    ) -> Result<()> {
        let allowed = self
            .core
            .inter_tail
            .as_ref()
            .is_some_and(|tail| tail.allow_bawp);
        if !allowed || !self.bawp_reads(frontier, single)? {
            return match single.bawp {
                Some(_) => Err(invalid_type()),
                None => Ok(()),
            };
        }
        out.push((TileCdfSelector::UseBawp, u8::from(single.bawp.is_some())));
        let Some(bawp) = single.bawp else {
            return Ok(());
        };
        let ctx = super::super::explicit_bawp_context(single.mode, single.use_amvd);
        out.push((
            TileCdfSelector::ExplicitBawp { ctx },
            u8::from(bawp.explicit_scale.is_some()),
        ));
        if let Some(positive) = bawp.explicit_scale {
            out.push((TileCdfSelector::ExplicitBawpScale, u8::from(positive)));
        }
        if frontier.has_chroma {
            out.push((TileCdfSelector::UseBawpChroma, u8::from(bawp.chroma)));
        } else if bawp.chroma {
            return Err(invalid_type());
        }
        Ok(())
    }

    /// Whether the BAWP template of `single` at `frontier` lies inside the
    /// luma plane and, with `chroma` (`use_bawp_chroma`), the chroma planes,
    /// as § 7.13.3.25 requires of a conformant stream.
    pub(crate) fn bawp_conformant(
        &mut self,
        frontier: &DecodeBlockFrontier,
        single: EncoderSingleMotion,
        chroma: bool,
    ) -> Result<bool> {
        let mv = self.motion_vector(frontier, single)?;
        let (n4w, n4h) = block_size4(frontier)?;
        let chroma = chroma && frontier.has_chroma && self.chroma != ChromaFormatIdc::Monochrome;
        let planes = [(0, 0), super::super::block_chroma_subsampling(self.chroma)];
        let block = BawpBlock {
            mi: (frontier.r, frontier.c),
            n4: (n4w, n4h),
            mi_size: (self.mi_rows, self.mi_cols),
        };
        Ok(planes[..=usize::from(chroma)]
            .iter()
            .all(|&subsampling| block.template_inside(subsampling, mv)))
    }

    /// The decoder's BAWP record of `single` at `frontier`
    /// (§ 7.13.3.25).
    pub(super) fn bawp_syntax(
        &self,
        frontier: &DecodeBlockFrontier,
        single: EncoderSingleMotion,
    ) -> Result<BawpSyntax> {
        let Some(bawp) = single.bawp else {
            return Ok(BawpSyntax::default());
        };
        let input = self.bawp_input(frontier, single)?;
        let syntax = BawpSyntax {
            enabled: true,
            explicit: bawp.explicit_scale.is_some(),
            explicit_scale_positive: bawp.explicit_scale.unwrap_or(false),
            list_index: super::super::explicit_bawp_context(single.mode, single.use_amvd) as u8,
            ref_dist_gt4: false,
            chroma: bawp.chroma && frontier.has_chroma,
            avail_up: input.avail_up,
            avail_left: input.avail_left,
        };
        Ok(super::super::bawp_with_ref_distance(
            syntax,
            single.ref_frame,
            &self.ref_frame_idx,
            &self.reference,
            self.current_order_hint,
        ))
    }
}

/// A block for the § 7.13.3.25 template requirement, in luma 4x4 units:
/// its position, its size and the frame's `MiRows` and `MiCols`.
#[derive(Clone, Copy, Debug)]
struct BawpBlock {
    mi: (usize, usize),
    n4: (usize, usize),
    mi_size: (usize, usize),
}

impl BawpBlock {
    /// Whether the template of the plane with `(subX, subY)` under `mv`
    /// lies inside the plane: `refX >= 1`, `refY >= 1`,
    /// `refX + bw <= planeWidth` and `refY + bh <= planeHeight`.
    fn template_inside(self, (sx, sy): (u32, u32), mv: Mv) -> bool {
        let to_fullmv = crate::prediction::inter::bawp::to_fullmv;
        let ((row, col), (n4w, n4h), (mi_rows, mi_cols)) = (self.mi, self.n4, self.mi_size);
        let plane_width = (mi_cols * MI_SIZE) as i64 >> sx;
        let plane_height = (mi_rows * MI_SIZE) as i64 >> sy;
        let x = ((col * MI_SIZE) >> sx) as i64;
        let y = ((row * MI_SIZE) >> sy) as i64;
        let bw = (plane_width - x).min(((n4w * MI_SIZE) >> sx) as i64);
        let bh = (plane_height - y).min(((n4h * MI_SIZE) >> sy) as i64);
        let ref_y = ((row * MI_SIZE) as i64 + i64::from(to_fullmv(mv.row))) >> sy;
        let ref_x = ((col * MI_SIZE) as i64 + i64::from(to_fullmv(mv.col))) >> sx;
        ref_x >= 1 && ref_y >= 1 && ref_x + bw <= plane_width && ref_y + bh <= plane_height
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
fn wedge_symbols(out: &mut TypeSymbols, index: u8) -> Result<()> {
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
            let mut out = TypeSymbols::default();
            wedge_symbols(&mut out, index)?;
            let out: Vec<_> = out.into_iter().collect();
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
        assert!(wedge_symbols(&mut TypeSymbols::default(), 68).is_err());
        Ok(())
    }

    /// The § 7.13.3.25 template requirement at its boundaries: a 16x16 block
    /// at luma (16, 16) of a 64x48 frame accepts `refX == 1` and
    /// `refX + bw == planeWidth` (and the same for Y), refuses `refX == 0`
    /// and `refX + bw == planeWidth + 1`, in luma, 4:2:0 and 4:2:2 chroma.
    #[test]
    fn bawp_templates_stay_inside_the_plane() {
        let block = BawpBlock {
            mi: (4, 4),
            n4: (4, 4),
            mi_size: (12, 16),
        };
        let fullpel = |row: i32, col: i32| Mv {
            row: row * 8,
            col: col * 8,
        };
        for (sx, sy) in [(0u32, 0u32), (1, 1), (1, 0)] {
            let (x, y) = (16 >> sx, 16 >> sy);
            let (width, height) = (64 >> sx, 48 >> sy);
            let (bw, bh) = (16 >> sx, 16 >> sy);
            let shift = |plane: i32, s: u32| plane << s;
            let accepts = |dx: i32, dy: i32| {
                block.template_inside((sx, sy), fullpel(shift(dy, sy), shift(dx, sx)))
            };
            assert!(accepts(1 - x, 0) && accepts(0, 1 - y), "{sx}{sy} low edge");
            assert!(accepts(width - bw - x, 0) && accepts(0, height - bh - y));
            assert!(!accepts(-x, 0) && !accepts(0, -y), "{sx}{sy} zero");
            assert!(!accepts(width - bw - x + 1, 0) && !accepts(0, height - bh - y + 1));
        }
    }
}
