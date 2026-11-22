// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lossless DPCM of the encoder facade (§ 5.20.5.5, § 5.20.5.6, § 7.14.4).
//!
//! The bodies stay out of line so the lossy mode writers keep their code.

use splot_core::symbol_encoder::SymbolSink;
use splot_recon::DpcmDirection;

use super::super::cdf::TileCdfSelector;
use super::super::cdf::block_context::IntraJointMode;
use super::super::general_intra_block::{DPCM_HORIZONTAL_JOINT_MODE, DPCM_VERTICAL_JOINT_MODE};
use super::intra::{ChromaMode, IntraModes, Mrl};
use super::{EncoderSyntaxError, IntraMode, IntraTileSyntax, Result, TransformBlock, rule};

/// DPCM in a leaf that is not lossless.
pub(super) const DPCM_LOSSY: EncoderSyntaxError =
    EncoderSyntaxError::Unsupported("DPCM in a lossy block");

const DPCM_MODE: EncoderSyntaxError = EncoderSyntaxError::Unsupported("DPCM mode");

/// `dpcm_mode_y` / `dpcm_mode_uv` of `mode`: `V_PRED` is 0, `H_PRED` is 1.
pub(super) fn dpcm_mode(mode: IntraMode) -> Result<u8> {
    match mode {
        IntraMode::Vertical => Ok(0),
        IntraMode::Horizontal => Ok(1),
        _ => Err(DPCM_MODE),
    }
}

/// The `IntraJointMode` that § 5.20.5.5 sets for the DPCM luma `y_mode`.
#[cold]
#[inline(never)]
pub(super) fn dpcm_joint_mode(y_mode: IntraMode) -> Result<IntraJointMode> {
    let joint = match dpcm_mode(y_mode)? {
        0 => DPCM_VERTICAL_JOINT_MODE,
        _ => DPCM_HORIZONTAL_JOINT_MODE,
    };
    IntraJointMode::try_new(joint).map_err(rule("DPCM joint mode"))
}

impl IntraTileSyntax {
    /// Whether the leaves code `use_dpcm_y` and `use_dpcm_uv`: every block is lossless.
    #[must_use]
    #[inline]
    pub const fn dpcm_allowed(&self) -> bool {
        self.tools.lossless
    }

    /// The § 7.14.4 DPCM direction of `tx` of the planned leaf, `None` without DPCM.
    #[must_use]
    #[inline]
    pub const fn dpcm(&self, tx: &TransformBlock) -> Option<DpcmDirection> {
        self.dpcm_leaf[if tx.plane == 0 { 0 } else { 1 }]
    }

    /// The luma and chroma DPCM directions of `modes` in a lossless frame,
    /// checked against the syntax.
    #[cold]
    #[inline(never)]
    pub(super) fn dpcm_directions(&self, modes: IntraModes) -> Result<[Option<DpcmDirection>; 2]> {
        if !self.tools.lossless {
            return Err(DPCM_LOSSY);
        }
        let direction = |mode| {
            Ok(match dpcm_mode(mode)? {
                0 => DpcmDirection::Vertical,
                _ => DpcmDirection::Horizontal,
            })
        };
        let luma = if modes.dpcm_y {
            if modes.angle_delta != 0
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
            ChromaMode::Intra(mode) if modes.dpcm_uv => Some(direction(mode)?),
            _ if modes.dpcm_uv => return Err(DPCM_MODE),
            _ => None,
        };
        Ok([luma, chroma])
    }

    /// `dpcm_mode_y` of a DPCM luma (§ 5.20.5.5).
    #[cold]
    #[inline(never)]
    pub(super) fn write_dpcm_mode_y(
        &mut self,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        self.dpcm_directions(modes)?;
        self.put(
            symbols,
            TileCdfSelector::DpcmModeY,
            dpcm_mode(modes.y_mode)?.into(),
        )
    }

    /// `dpcm_mode_uv` of a DPCM chroma (§ 5.20.5.6).
    #[cold]
    #[inline(never)]
    pub(super) fn write_dpcm_uv(
        &mut self,
        modes: IntraModes,
        symbols: &mut impl SymbolSink,
    ) -> Result<()> {
        let ChromaMode::Intra(mode) = modes.uv_mode else {
            return Err(DPCM_MODE);
        };
        self.put(
            symbols,
            TileCdfSelector::DpcmModeUv,
            dpcm_mode(mode)?.into(),
        )
    }
}
