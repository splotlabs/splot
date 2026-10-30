// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Warp leaves of the encoder syntax facade: `WARPMV` and `WARP_NEWMV` with
//! the `LOCALWARP`, `EXTENDWARP` and `DELTAWARP` models (§ 5.20.7.7,
//! § 5.20.7.11, § 5.20.7.14), coded with the symbols and resolved with the
//! models the decoder reads and derives.

use super::super::resolve::{WarpDeltaSyntax, WarpModelSource, WarpMotionSyntax};
use super::super::warp::warp_delta_scale;
use super::*;

/// `WARP_DELTA_NUM_SYMBOLS_HIGH` (§ 3).
const WARP_DELTA_NUM_SYMBOLS_HIGH: u8 = 8;

/// How a warp leaf builds its model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EncoderWarpSource {
    /// `WARP_NEWMV` with `LOCALWARP`: § 7.13.3.23 least squares.
    Local,
    /// `WARP_NEWMV` with `EXTENDWARP`: § 7.13.3.24 extension.
    Extend,
    /// `WARP_NEWMV` with `DELTAWARP`: the coded `warp_delta_param` values of
    /// model indices 2 to 5 with `warp_delta_precision` (§ 5.20.7.7).
    Delta {
        high_precision: bool,
        values: [i32; 4],
    },
    /// `WARPMV`: the warp candidate's model and vector.
    Candidate,
}

/// The warp choice of one leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EncoderWarpMotion {
    /// The reference list index, `ref_frame0`.
    pub(crate) ref_frame: i8,
    pub(crate) source: EncoderWarpSource,
    /// `RefMvIdx` of a `WARP_NEWMV`.
    pub(crate) ref_mv_idx: usize,
    /// `RefWarpIdx` of a `DELTAWARP` or `WARPMV`.
    pub(crate) ref_warp_idx: usize,
    /// The difference: every `WARP_NEWMV` codes one, a `WARPMV` with
    /// `warpmv_with_mvd`.
    pub(crate) mvd: Option<Mv>,
    /// The explicit block `MvPrecision` of a `WARP_NEWMV`.
    pub(crate) precision: Option<u8>,
}

/// The warp tools a leaf can code, see [`EncoderInterTile::warp_options`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EncoderWarpOptions {
    /// `WARP_NEWMV`: `force_integer_mv` frames code `WARPMV` only.
    pub(crate) newmv: bool,
    pub(crate) local: bool,
    pub(crate) extend: bool,
    /// `enable_six_param_warp_delta`: `RefWarpIdx` 1 codes four deltas.
    pub(crate) six_param: bool,
}

impl EncoderWarpSource {
    const fn model(self) -> WarpModelSource {
        match self {
            Self::Local => WarpModelSource::LocalSamples,
            Self::Extend => WarpModelSource::Extended,
            Self::Candidate => WarpModelSource::Candidate,
            Self::Delta { .. } => WarpModelSource::Delta(WarpDeltaSyntax {
                deltas: None,
                six_param: false,
            }),
        }
    }
}

impl EncoderInterTile {
    /// Whether the frame codes `allow_warpmv_mode`.
    pub(crate) fn allow_warpmv_mode(&self) -> bool {
        self.core
            .inter_tail
            .as_ref()
            .is_some_and(|tail| tail.allow_warpmv_mode)
    }

    /// Whether a non-TIP single-reference block at `frontier` reads `is_warp`.
    pub(crate) fn reads_is_warp(&self, frontier: &DecodeBlockFrontier) -> Result<bool> {
        let (n4w, n4h) = block_size4(frontier)?;
        Ok(self.allow_warpmv_mode() && n4w >= 2 && n4h >= 2)
    }

    /// The warp tools a block at `frontier` from list reference `ref_frame`
    /// can code; `None` where it reads no `is_warp`.
    pub(crate) fn warp_options(
        &self,
        frontier: &DecodeBlockFrontier,
        ref_frame: i8,
    ) -> Result<Option<EncoderWarpOptions>> {
        if !self.reads_is_warp(frontier)? {
            return Ok(None);
        }
        let block = self.block(frontier, (ref_frame, None))?;
        let found = find_mode_ctx(&self.grid, &block).warp_sample_found;
        let modes = self
            .core
            .inter
            .as_ref()
            .and_then(|inter| inter.frame_enabled_motion_modes)
            .unwrap_or_default();
        let newmv = !effective_force_integer_mv(&self.core);
        Ok(Some(EncoderWarpOptions {
            newmv,
            local: newmv && found && modes[splot_core::headers::frame::LOCALWARP],
            extend: newmv && found && modes[splot_core::headers::frame::EXTENDWARP],
            six_param: self
                .sequence
                .inter
                .as_ref()
                .is_some_and(|inter| inter.enable_six_param_warp_delta),
        }))
    }

    /// The decoder's parse record of `warp` at `frontier` and `block`.
    pub(super) fn warp_syntax(
        &self,
        block: &MvBlockContext,
        warp: EncoderWarpMotion,
    ) -> Result<WarpMotionSyntax> {
        let source = match warp.source {
            EncoderWarpSource::Delta {
                high_precision,
                values,
            } => {
                let six_param = self
                    .sequence
                    .inter
                    .as_ref()
                    .is_some_and(|inter| inter.enable_six_param_warp_delta)
                    && warp.ref_warp_idx == 1;
                let coded = six_param || warp.ref_warp_idx == 0;
                let mut deltas = [0; 4];
                for (delta, &value) in deltas.iter_mut().zip(&values) {
                    let magnitude = u8::try_from(value.unsigned_abs())
                        .ok()
                        .filter(|&m| m <= delta_limit(high_precision))
                        .ok_or(DecodeHeaderStateError::InvalidInterWarpModelState)?;
                    *delta = warp_delta_scale(magnitude, value < 0, high_precision);
                }
                if !coded && values != [0; 4] || !six_param && values[2..] != [0; 2] {
                    return Err(DecodeHeaderStateError::InvalidInterWarpModelState.into());
                }
                WarpModelSource::Delta(WarpDeltaSyntax {
                    deltas: coded.then_some(deltas),
                    six_param,
                })
            }
            other => other.model(),
        };
        Ok(WarpMotionSyntax {
            source,
            ref_mv_idx: warp.ref_mv_idx,
            ref_warp_idx: warp.ref_warp_idx,
            mvd: warp.mvd,
            extend_delta: find_mode_ctx(&self.grid, block).extend_delta,
            derive_wrl: block.bw4 >= 2 && block.bh4 >= 2,
            use_temporal_first: self.temporal_first(warp.ref_frame),
        })
    }

    /// The warp mode symbols of `warp` at `frontier` from `is_warp` to the
    /// block precision, in coding order (§ 5.20.7.11, § 5.20.7.14).
    pub(super) fn warp_symbols(
        &self,
        out: &mut Vec<(TileCdfSelector, u8)>,
        frontier: &DecodeBlockFrontier,
        (neighbour, mode_ctx, max_drl): (&BlockNeighbourContext, &ModeContext, usize),
        warp: EncoderWarpMotion,
    ) -> Result<()> {
        let invalid =
            || -> crate::DecodeError { DecodeHeaderStateError::InvalidInterBlockModeState.into() };
        let options = self
            .warp_options(frontier, warp.ref_frame)?
            .ok_or_else(invalid)?;
        out.push((
            TileCdfSelector::IsWarp {
                ctx: mode_ctx.warp_mv_count,
            },
            1,
        ));
        let candidate = warp.source == EncoderWarpSource::Candidate;
        if !candidate && !options.newmv {
            return Err(invalid());
        }
        if options.newmv {
            out.push((TileCdfSelector::WarpMv, u8::from(candidate)));
        }
        if !candidate {
            let extend = warp.source == EncoderWarpSource::Extend;
            let local = warp.source == EncoderWarpSource::Local;
            if options.extend {
                let ctx = neighbour.use_extend_warp_ctx();
                out.push((TileCdfSelector::UseExtendWarp { ctx }, u8::from(extend)));
            }
            if options.local && !extend {
                let ctx = neighbour.use_local_warp_ctx();
                out.push((TileCdfSelector::UseLocalWarp { ctx }, u8::from(local)));
            }
            if (extend && !options.extend) || (local && !options.local) || warp.mvd.is_none() {
                return Err(invalid());
            }
        }
        if matches!(
            warp.source,
            EncoderWarpSource::Candidate | EncoderWarpSource::Delta { .. }
        ) {
            if warp.ref_warp_idx >= MAX_WARP_REF_CANDIDATES {
                return Err(invalid());
            }
            for ctx in 0..MAX_WARP_REF_CANDIDATES - 1 {
                let more = warp.ref_warp_idx > ctx;
                out.push((TileCdfSelector::WarpIdx { ctx }, u8::from(more)));
                if !more {
                    break;
                }
            }
        } else if warp.ref_warp_idx != 0 {
            return Err(invalid());
        }
        if candidate {
            if warp.ref_warp_idx < 2 {
                out.push((TileCdfSelector::WarpWithMvd, u8::from(warp.mvd.is_some())));
            } else if warp.mvd.is_some() {
                return Err(invalid());
            }
            return if warp.ref_mv_idx == 0 && warp.precision.is_none() {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let selector = |idx| TileCdfSelector::DrlMode {
            idx,
            ctx: mode_ctx.new_mv_context,
        };
        super::compound::drl_symbols(out, selector, 0, max_drl, warp.ref_mv_idx)?;
        self.precision_symbols(
            out,
            neighbour,
            true,
            (self.block_precision(EncoderMotion::Warp(warp)), false),
        )
    }

    /// The symbols `warp` at `frontier` codes after its difference: the
    /// `DELTAWARP` parameters (§ 5.20.7.7), then `warp_inter_intra = 0` of a
    /// `WARPMV` (§ 5.20.7.15).
    pub(crate) fn warp_tail_symbols(
        &self,
        frontier: &DecodeBlockFrontier,
        warp: EncoderWarpMotion,
    ) -> Result<Vec<(TileCdfSelector, u8)>> {
        let block = self.block(frontier, (warp.ref_frame, None))?;
        let mut out = Vec::new();
        match (warp.source, self.warp_syntax(&block, warp)?.source) {
            (
                EncoderWarpSource::Delta {
                    high_precision,
                    values,
                },
                WarpModelSource::Delta(WarpDeltaSyntax {
                    deltas: Some(_),
                    six_param,
                }),
            ) => {
                let block_size = frontier.b_size.index();
                out.push((
                    TileCdfSelector::WarpPrecision { block_size },
                    u8::from(high_precision),
                ));
                let count = if six_param { 4 } else { 2 };
                for (param, &value) in values.iter().enumerate().take(count) {
                    let index_type = usize::from(param == 1 || param == 2);
                    let magnitude = u8::try_from(value.unsigned_abs())
                        .map_err(|_| DecodeHeaderStateError::InvalidInterWarpModelState)?;
                    let low = magnitude.min(WARP_DELTA_NUM_SYMBOLS_LOW - 1);
                    out.push((TileCdfSelector::WarpDeltaParamLow { index_type }, low));
                    if high_precision && low == WARP_DELTA_NUM_SYMBOLS_LOW - 1 {
                        out.push((
                            TileCdfSelector::WarpDeltaParamHigh { index_type },
                            magnitude - low,
                        ));
                    }
                    if value != 0 {
                        out.push((TileCdfSelector::WarpDeltaParamSign, u8::from(value < 0)));
                    }
                }
            }
            (EncoderWarpSource::Candidate, _) => {
                let (n4w, n4h) = (block.bw4, block.bh4);
                if n4w >= 2 && n4h >= 2 && n4w.max(n4h) <= CHUNK_64_N4 {
                    let bsize_group = SIZE_GROUP_LOOKUP[frontier.b_size.index()];
                    out.push((TileCdfSelector::WarpInterIntra { bsize_group }, 0));
                }
            }
            _ => {}
        }
        Ok(out)
    }
}

/// The largest coded `warp_delta_param` magnitude at a precision.
const fn delta_limit(high_precision: bool) -> u8 {
    if high_precision {
        WARP_DELTA_NUM_SYMBOLS_LOW - 1 + WARP_DELTA_NUM_SYMBOLS_HIGH - 1
    } else {
        WARP_DELTA_NUM_SYMBOLS_LOW - 1
    }
}

impl EncoderInterTile {
    /// The vector and § 7.13.3 model the decoder resolves for `warp` at
    /// `frontier`, not published.
    pub(crate) fn warp_resolution(
        &mut self,
        frontier: &DecodeBlockFrontier,
        warp: EncoderWarpMotion,
    ) -> Result<(Mv, [i32; 6])> {
        let syntax = self.inter_syntax(frontier, EncoderMotion::Warp(warp), true)?;
        let resolved = self.resolve_motion(&syntax)?;
        let model =
            resolved.warp_params[0].ok_or(DecodeHeaderStateError::InvalidInterWarpModelState)?;
        Ok((resolved.mv[0], model))
    }

    /// The warp bank and the warp model of each 4x4 unit of a warp frame,
    /// for the debug rollback check.
    #[cfg(debug_assertions)]
    pub(crate) fn warp_debug_state(&self) -> (WarpParamBank, Vec<Option<[i32; 6]>>) {
        let models = if self.allow_warpmv_mode() {
            let units = (0..self.mi_rows).flat_map(|r| (0..self.mi_cols).map(move |c| (r, c)));
            units.map(|(r, c)| self.grid.warp_model_at(r, c)).collect()
        } else {
            Vec::new()
        };
        (self.warp_bank.clone(), models)
    }
}
