// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless DPCM of the encoder facade (§ 5.20.5.5, § 5.20.5.6, § 7.14.4).
//!
//! The DPCM of a leaf is a side input, [`IntraTileSyntax::set_leaf_dpcm`],
//! that only a lossless frame reads, and the bodies stay out of line, so the
//! lossy plan and mode writers keep their code and [`IntraModes`] its size.

use splot_core::symbol_encoder::SymbolSink;
use splot_recon::DpcmDirection;

use super::super::cdf::TileCdfSelector;
use super::super::cdf::block_context::IntraJointMode;
use super::super::general_intra_block::{
    DPCM_HORIZONTAL_JOINT_MODE, DPCM_VERTICAL_JOINT_MODE, GeneralIntraBlockModes,
    GeneralIntraChromaBlockMode,
};
use super::intra::{ChromaMode, IntraModes, Mrl};
use super::{
    EncoderSyntaxError, IntraMode, IntraTileSyntax, LeafBlock, Result, TransformBlock, rule,
};

/// `use_dpcm_y` and `use_dpcm_uv` of one intra leaf of a lossless frame.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Dpcm {
    /// `use_dpcm_y`: the leaf's `V_PRED` or `H_PRED` luma, without angle
    /// delta, MRL, DIP or palette, codes `dpcm_mode_y` (§ 5.20.5.5).
    pub y: bool,
    /// `use_dpcm_uv`: the leaf's `V_PRED` or `H_PRED` chroma codes
    /// `dpcm_mode_uv` (§ 5.20.5.6).
    pub uv: bool,
}

impl Dpcm {
    /// No DPCM.
    pub const OFF: Self = Self {
        y: false,
        uv: false,
    };
}

const DPCM_MODE: EncoderSyntaxError = EncoderSyntaxError::Unsupported("DPCM mode");

/// The § 7.14.4 direction of the DPCM `mode`: `V_PRED` sums down the columns,
/// `H_PRED` along the rows; `dpcm_mode_y` / `dpcm_mode_uv` is 0 or 1.
fn direction(mode: IntraMode) -> Result<DpcmDirection> {
    match mode {
        IntraMode::Vertical => Ok(DpcmDirection::Vertical),
        IntraMode::Horizontal => Ok(DpcmDirection::Horizontal),
        _ => Err(DPCM_MODE),
    }
}

/// `dpcm_mode_y` / `dpcm_mode_uv` of `direction`.
fn mode_symbol(direction: DpcmDirection) -> u8 {
    u8::from(direction == DpcmDirection::Horizontal)
}

impl IntraTileSyntax {
    /// Whether the leaves code `use_dpcm_y` and `use_dpcm_uv`: every block is lossless.
    #[must_use]
    #[inline]
    pub const fn dpcm_allowed(&self) -> bool {
        self.tools.lossless
    }

    /// Sets the DPCM of the next intra leaf that a lossless frame plans; the
    /// plan takes it, so a later plan without a new call codes no DPCM.
    pub const fn set_leaf_dpcm(&mut self, dpcm: Dpcm) {
        self.next_dpcm = dpcm;
    }

    /// The § 7.14.4 DPCM direction of `tx` of the planned leaf, `None` without DPCM.
    #[must_use]
    #[inline]
    pub const fn dpcm(&self, tx: &TransformBlock) -> Option<DpcmDirection> {
        self.dpcm_leaf[if tx.plane == 0 { 0 } else { 1 }]
    }

    /// Takes the next leaf DPCM for `leaf` with `modes`, checks it against
    /// the syntax, and adds it to the decoder's `block_modes`.
    #[cold]
    #[inline(never)]
    pub(super) fn plan_dpcm(
        &mut self,
        leaf: &LeafBlock,
        modes: IntraModes,
        block_modes: &mut GeneralIntraBlockModes,
    ) -> Result<()> {
        let dpcm = core::mem::take(&mut self.next_dpcm);
        let luma = if dpcm.y {
            if leaf.is_chroma_part()
                || modes.angle_delta != 0
                || modes.mrl != Mrl::OFF
                || modes.dip.is_some()
                || modes.palette.is_some()
            {
                return Err(DPCM_MODE);
            }
            Some(direction(modes.y_mode)?)
        } else {
            None
        };
        let chroma = match modes.uv_mode {
            ChromaMode::Intra(mode) if dpcm.uv && leaf.has_chroma() => Some(direction(mode)?),
            _ if dpcm.uv => return Err(DPCM_MODE),
            _ => None,
        };
        self.dpcm_leaf = [luma, chroma];
        self.apply_dpcm(block_modes)
    }

    /// Adds the DPCM of the planned leaf to its decoder `block_modes`: the
    /// `IntraJointMode` that § 5.20.5.5 sets and the DPCM chroma mode.
    #[cold]
    #[inline(never)]
    pub(super) fn apply_dpcm(&self, block_modes: &mut GeneralIntraBlockModes) -> Result<()> {
        if let Some(luma) = self.dpcm_leaf[0] {
            let joint = match luma {
                DpcmDirection::Vertical => DPCM_VERTICAL_JOINT_MODE,
                DpcmDirection::Horizontal => DPCM_HORIZONTAL_JOINT_MODE,
            };
            block_modes.intra_joint_mode =
                IntraJointMode::try_new(joint).map_err(rule("DPCM joint mode"))?;
            block_modes.use_dpcm_y = 1;
            block_modes.dpcm_mode_y = mode_symbol(luma);
        }
        if let Some(chroma) = self.dpcm_leaf[1] {
            block_modes.chroma = Some(GeneralIntraChromaBlockMode::dpcm(mode_symbol(chroma)));
        }
        Ok(())
    }

    /// `use_dpcm_y` of the planned leaf, and whether it is set.
    #[cold]
    #[inline(never)]
    pub(super) fn write_use_dpcm_y(&mut self, symbols: &mut impl SymbolSink) -> Result<bool> {
        let dpcm = self.dpcm_leaf[0].is_some();
        self.put(symbols, TileCdfSelector::UseDpcmY, dpcm.into())?;
        Ok(dpcm)
    }

    /// `dpcm_mode_y` of the planned DPCM luma (§ 5.20.5.5).
    #[cold]
    #[inline(never)]
    pub(super) fn write_dpcm_mode_y(&mut self, symbols: &mut impl SymbolSink) -> Result<()> {
        let direction = self.dpcm_leaf[0].ok_or(DPCM_MODE)?;
        self.put(
            symbols,
            TileCdfSelector::DpcmModeY,
            mode_symbol(direction).into(),
        )
    }

    /// `use_dpcm_uv` and `dpcm_mode_uv` of the planned leaf (§ 5.20.5.6),
    /// and whether it is set.
    #[cold]
    #[inline(never)]
    pub(super) fn write_dpcm_uv(&mut self, symbols: &mut impl SymbolSink) -> Result<bool> {
        let dpcm = self.dpcm_leaf[1];
        self.put(symbols, TileCdfSelector::UseDpcmUv, dpcm.is_some().into())?;
        if let Some(direction) = dpcm {
            self.put(
                symbols,
                TileCdfSelector::DpcmModeUv,
                mode_symbol(direction).into(),
            )?;
        }
        Ok(dpcm.is_some())
    }
}
