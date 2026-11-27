// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The frame-level tools, frame headers and OBUs of the planned frames.

use splot_core::headers::frame::{
    CoreSeqView, EncodedFrameHeader, EncoderHeaderError, FrameFilterParams, FrameHeaderCore,
    FrameTiles, InterFrameParams, InterpolationFilter, KeyFrameTxParams, REFS_PER_FRAME,
    TipGlobalMv, build_inter_frame_header, build_intra_only_frame_header, build_key_frame_header,
    build_open_loop_key_frame_header, build_tip_output_frame_header, encode_inter_frame_obu,
    encode_key_frame_temporal_unit, encode_show_existing_frame_obu, encode_temporal_delimiter_obu,
    encode_tip_output_frame_obu,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::types::TemporalLayerId;
use splot_decode::encoder_syntax::EncoderReferences;

use crate::error::{Error, Result};
use crate::preset::{PresetContext, PresetParams};
use crate::rate_control::FrameType;

use super::GopFrame;

/// The frame-level tool choices of `preset` in `context` for a frame at `qindex`: the
/// transform parameters, whether inter frames code compound and skip-mode
/// blocks, whether their motion vectors are eighth-pel, whether they take
/// temporal motion vectors, whether they code switchable interpolation
/// filters, their TIP weight, whether they force integer vectors, their
/// `opfl_refine_type`, and the `tiles`.
pub(crate) fn frame_tools(
    (preset, context): (&PresetParams, PresetContext),
    tiles: &FrameTiles,
    qindex: u32,
) -> FrameTools {
    (
        crate::tx_choice::key_frame_tx_params(preset, context),
        preset.compound_candidates > 0,
        (
            !preset.integer_mv
                && preset.subpel_depth == 3
                && qindex <= u32::from(preset.eighth_pel_max_qindex),
            preset.mv_prediction,
            preset.interp_filter_search > 0,
            (preset.tip > 0 && preset.mv_prediction).then_some(TIP_EQUAL_WEIGHT),
            preset.integer_mv,
            match (preset.opfl, preset.opfl_refine_all) {
                (false, _) => 0,
                (true, false) => REFINE_SWITCHABLE,
                (true, true) => REFINE_ALL,
            },
            preset.warp > 0,
            [None; REFS_PER_FRAME],
            preset.global_motion_predict,
            preset.explicit_refs.checked_sub(1).filter(|&refs| refs > 0),
        ),
        *tiles,
    )
}

/// `(qindex, TipGlobalMv, TipInterpFilter, TIP deblocking)` of a
/// `TIP_FRAME_AS_OUTPUT` frame; see [`GopFrame::tip_output_unit`].
pub(crate) type TipOutputChoice = (u32, TipGlobalMv, Option<InterpolationFilter>, bool);

/// `opfl_refine_type` `REFINE_SWITCHABLE` (§ 3): compound blocks code
/// `use_optflow`.
const REFINE_SWITCHABLE: u8 = 1;

/// `opfl_refine_type` `REFINE_ALL` (§ 3): every compound block that allows
/// it refines with optical flow.
const REFINE_ALL: u8 = 2;

/// `tip_global_wtd_index` of `Tip_Weighting_Factor` 8 (`CWP_EQUAL`), the
/// plain average of the two TIP references (§ 7.13.3.1).
const TIP_EQUAL_WEIGHT: u8 = 0;

/// `(tx, compound, (high_precision_mv, use_ref_frame_mvs,
/// switchable_interp_filter, tip_weight, force_integer_mv,
/// opfl_refine_type, warp, global_motion, global_motion_predict,
/// explicit_refs), tiles)` of [`frame_tools`].
pub(crate) type FrameTools = (
    KeyFrameTxParams,
    bool,
    (
        bool,
        bool,
        bool,
        Option<u8>,
        bool,
        u8,
        bool,
        [Option<[i32; 6]>; REFS_PER_FRAME],
        bool,
        Option<u8>,
    ),
    FrameTiles,
);

/// A temporal delimiter and the `OBU_REGULAR_SEF`, or the `OBU_LEADING_SEF`
/// when `leading`, that outputs `slot`, with the explicit `order_hint` when
/// given, and the film grain of `grain_seed`, in the temporal layer `tlayer`
/// of the frame it shows; with `multi_frame_header` it codes `cur_mfh_id = 1`.
pub(crate) fn show_existing_unit(
    view: &CoreSeqView,
    slot: u32,
    order_hint: Option<u32>,
    (grain_seed, multi_frame_header, leading): (Option<u16>, bool, bool),
    tlayer: TemporalLayerId,
) -> Result<Vec<u8>> {
    let mut unit = encode_temporal_delimiter_obu().map_err(header_error)?;
    let flags = (multi_frame_header, leading, tlayer);
    unit.extend(
        encode_show_existing_frame_obu(view, slot, order_hint, grain_seed, flags)
            .map_err(header_error)?,
    );
    Ok(unit)
}

/// The `grain_seed` of coded frame `frame_index`. Coded and show-existing
/// frames take the even and odd steps of one bijection of `0..65535`, so no
/// two output frames within 32768 of each other share a seed, and no seed is
/// zero, which would keep the § 7.21.7.2 random register at zero.
pub(crate) fn coded_grain_seed(frame_index: u64) -> u16 {
    grain_seed(frame_index.wrapping_mul(2))
}

/// The `grain_seed` of the `n`-th show-existing frame; see [`coded_grain_seed`].
pub(crate) fn shown_grain_seed(n: u64) -> u16 {
    grain_seed(n.wrapping_mul(2).wrapping_add(1))
}

fn grain_seed(step: u64) -> u16 {
    (1 + step % 65535 * 7919 % 65535) as u16
}

impl GopFrame {
    /// The frame header at `qindex` with `filters`, parsed with `refs`; an
    /// inter frame with `compound` sets `reference_select` and
    /// `skip_mode_present`. The header codes the low bits of the planned
    /// `OrderHint`, so the `OrderHint` the decoder derives must equal it.
    pub(crate) fn header(
        &self,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        qindex: u32,
        filters: &FrameFilterParams,
        (tx, compound, mv_tools, tiles): FrameTools,
    ) -> Result<FrameHeaderCore> {
        let core = match self.frame_type {
            FrameType::Key if self.open_loop => {
                let params = InterFrameParams {
                    tx,
                    ..self.inter_params(qindex, filters, (tx, compound, mv_tools, tiles))
                };
                build_open_loop_key_frame_header(view, &refs.header_view(), &params)
                    .map_err(header_error)?
                    .core
            }
            FrameType::Key => {
                build_key_frame_header(view, qindex, self.order_hint, filters, tx, tiles)
                    .map_err(header_error)?
            }
            FrameType::Inter => {
                let params = self.inter_params(qindex, filters, (tx, compound, mv_tools, tiles));
                inter_header(view, refs, &params, self.intra_only)?.core
            }
        };
        match core.display_order_hint() {
            Some(derived) if derived == self.order_hint => Ok(core),
            derived => Err(Error::OrderHintMismatch {
                planned: self.order_hint,
                derived,
            }),
        }
    }

    /// The OBUs of the frame `core` built by [`Self::header`] with
    /// `(qindex, filters, tools)`, whose `tile_groups` tile groups carry
    /// `tiles`, after a temporal delimiter when the frame opens its temporal
    /// unit; a key frame repeats the sequence header, followed by `key_obus`,
    /// its content interpretation, metadata and film grain OBUs. The
    /// header codes the smallest `TileSizeBytes` that fits the tiles.
    pub(crate) fn temporal_unit(
        &self,
        seq: &SequenceHeader,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        (qindex, filters, (tx, compound, mv_tools, layout)): (u32, &FrameFilterParams, FrameTools),
        core: &FrameHeaderCore,
        (tiles, key_obus, tile_groups): (&[&[u8]], &[u8], u32),
    ) -> Result<Vec<u8>> {
        let longest = tiles.split_last().map_or(0, |(_, rest)| {
            rest.iter()
                .map(|tile| tile.len().saturating_sub(1))
                .max()
                .unwrap_or(0)
        });
        let size_bytes = (1..4u8).find(|&n| longest >> (8 * n) == 0).unwrap_or(4);
        let layout = FrameTiles {
            size_bytes,
            ..layout
        };
        let mut expected = core.clone();
        if let Some(info) = expected.tile_info.as_mut() {
            info.tile_size_bytes = info.tile_size_bytes.map(|_| u32::from(size_bytes));
        }
        if self.frame_type == FrameType::Key {
            if !self.opens_temporal_unit {
                return Err(Error::Unsupported {
                    what: "a key frame after another frame of its temporal unit",
                });
            }
            let tools = (tx, compound, mv_tools, layout);
            let core = self.header(view, refs, qindex, filters, tools)?;
            return encode_key_frame_temporal_unit(
                seq,
                &core,
                true,
                (tiles, tile_groups),
                key_obus,
            )
            .map_err(header_error);
        }
        let header = inter_header(
            view,
            refs,
            &self.inter_params(qindex, filters, (tx, compound, mv_tools, layout)),
            self.intra_only,
        )?;
        if header.core != expected {
            return Err(Error::Unsupported {
                what: "an inter frame header that changed after coding",
            });
        }
        let mut unit = if self.opens_temporal_unit {
            encode_temporal_delimiter_obu().map_err(header_error)?
        } else {
            Vec::new()
        };
        unit.extend(encode_inter_frame_obu(&header, tiles, tile_groups).map_err(header_error)?);
        Ok(unit)
    }

    /// The header and OBUs of the frame as a `TIP_FRAME_AS_OUTPUT` frame with
    /// `tip_global_wtd_index` `weight`, the film grain of `grain_seed` and the
    /// chroma of the plane quantizer deltas `plane_delta_q` over
    /// the reference map `ref_frame_idx` references, after a temporal
    /// delimiter when the frame opens its temporal unit; `tip` is its
    /// `(qindex, TipGlobalMv, TipInterpFilter, TIP deblocking)`, the qindex
    /// coded only with `enable_tip_explicit_qp`.
    pub(crate) fn tip_output_unit(
        &self,
        view: &CoreSeqView,
        refs: &EncoderReferences,
        ref_frame_idx: &[u32],
        (weight, grain_seed, plane_delta_q): (u8, Option<u16>, [i8; 3]),
        multi_frame_header: bool,
        (qindex, mv, filter, deblocking): TipOutputChoice,
    ) -> Result<(FrameHeaderCore, Vec<u8>)> {
        let params = InterFrameParams {
            base_q_idx: qindex,
            tip_global_mv: mv,
            tip_filter: filter,
            tip_deblocking: deblocking,
            order_hint: self.order_hint,
            refresh_frame_flags: self.refresh_frame_flags,
            show_frame: self.show_frame,
            tip_weight: Some(weight),
            tip_hole_fill: true,
            leading: self.leading,
            tlayer: self.tlayer,
            tx: KeyFrameTxParams {
                grain_seed,
                plane_delta_q,
                multi_frame_header,
                ..KeyFrameTxParams::default()
            },
            ..InterFrameParams::default()
        };
        let header =
            build_tip_output_frame_header(view, &refs.header_view(), &params, ref_frame_idx)
                .map_err(header_error)?;
        if header.core.display_order_hint() != Some(self.order_hint) {
            return Err(Error::OrderHintMismatch {
                planned: self.order_hint,
                derived: header.core.display_order_hint(),
            });
        }
        let mut unit = if self.opens_temporal_unit {
            encode_temporal_delimiter_obu().map_err(header_error)?
        } else {
            Vec::new()
        };
        unit.extend(encode_tip_output_frame_obu(&header).map_err(header_error)?);
        Ok((header.core, unit))
    }

    /// The inter frame parameters; `tip_hole_fill` is always set here and
    /// in [`Self::tip_output_unit`], because the sequence flag
    /// `enable_tip_hole_fill` (`PresetParams::tip_hole_fill`) gates whether
    /// `allow_tip_hole_fill` is coded.
    fn inter_params(
        &self,
        qindex: u32,
        filters: &FrameFilterParams,
        (
            tx,
            compound,
            (
                high_precision_mv,
                use_ref_frame_mvs,
                switchable_interp_filter,
                tip_weight,
                force_integer_mv,
                opfl_refine_type,
                warp,
                global_motion,
                global_motion_predict,
                explicit_refs,
            ),
            tiles,
        ): FrameTools,
    ) -> InterFrameParams {
        InterFrameParams {
            base_q_idx: qindex,
            order_hint: self.order_hint,
            refresh_frame_flags: self.refresh_frame_flags,
            show_frame: self.show_frame,
            filters: *filters,
            tx: KeyFrameTxParams {
                allow_intrabc: !self.intra_only && inter_intrabc(),
                ..tx
            },
            tiles,
            reference_select: compound,
            skip_mode_present: compound,
            high_precision_mv,
            use_ref_frame_mvs,
            switchable_interp_filter,
            primary_slot: self.primary_slot,
            tip_weight,
            tip_global_mv: TipGlobalMv::default(),
            tip_filter: None,
            tip_deblocking: false,
            tip_hole_fill: true,
            force_integer_mv,
            opfl_refine_type,
            warp,
            global_motion,
            global_motion_predict,
            explicit_refs,
            implicit_output: self.implicit_output,
            leading: self.leading,
            tlayer: self.tlayer,
        }
    }
}

/// [`choose_inter_header`] with the parameters of each `reuse_ccso` plane
/// loaded from its reference, as the decoder loads them (§ 5.18.7.12); the
/// `INTRA_ONLY_FRAME` header of `params` when `intra_only`.
fn inter_header(
    view: &CoreSeqView,
    refs: &EncoderReferences,
    params: &InterFrameParams,
    intra_only: bool,
) -> Result<EncodedFrameHeader> {
    if intra_only {
        return build_intra_only_frame_header(view, &refs.header_view(), params)
            .map_err(header_error);
    }
    let mut header = choose_inter_header(view, refs, params).map_err(header_error)?;
    refs.resolve_ccso_reuse(&mut header.core)
        .map_err(header_error)?;
    Ok(header)
}

/// The inter header of `params` with an explicit `primary_ref_frame`; with
/// `choose_primary_ref_frame` the `PRIMARY_REF_CHOOSE` header instead when
/// the reference the decoder derives (§ 5.18.2) is the explicit one, which
/// keeps the CDFs and every `DerivedPrimaryRefFrame` use.
fn choose_inter_header(
    view: &CoreSeqView,
    refs: &EncoderReferences,
    params: &InterFrameParams,
) -> core::result::Result<EncodedFrameHeader, EncoderHeaderError> {
    let explicit = InterFrameParams {
        tx: KeyFrameTxParams {
            choose_primary_ref_frame: false,
            ..params.tx
        },
        ..*params
    };
    let header = build_inter_frame_header(view, &refs.header_view(), &explicit)?;
    if !params.tx.choose_primary_ref_frame {
        return Ok(header);
    }
    let chosen = build_inter_frame_header(view, &refs.header_view(), params)?;
    let coded = header
        .core
        .inter
        .as_ref()
        .and_then(|inter| inter.primary_ref_frame);
    Ok(if refs.primary_ref_frame(&chosen.core) == coded {
        chosen
    } else {
        header
    })
}

fn header_error<E: std::error::Error + Send + Sync + 'static>(source: E) -> Error {
    Error::Coding {
        stage: "frame header",
        source: Box::new(source),
    }
}

/// Whether inter frames code `allow_intrabc`: not yet outside tests.
#[cfg(not(test))]
const fn inter_intrabc() -> bool {
    false
}

#[cfg(test)]
fn inter_intrabc() -> bool {
    INTER_INTRABC.get()
}

#[cfg(test)]
thread_local! {
    /// Sets `allow_intrabc` in the inter frames (not `intra_only`) that this
    /// thread codes.
    pub(crate) static INTER_INTRABC: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}
