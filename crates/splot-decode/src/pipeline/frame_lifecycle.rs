// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame parsing, retained state, and lifecycle handoffs.

use super::{unsupported, unsupported_at, unsupported_feature_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use std::sync::Arc;

use splot_core::headers::film_grain::{FilmGrainModel, MAX_FILM_GRAIN, parse_film_grain};
use splot_core::headers::frame::{
    CoreSeqQuantView, FilmGrainConfig, FrameHeaderCore, FrameHeaderParseInput,
    FrameHeaderParseMode, FrameHeaderParseStatus, FrameReferenceStateView, FrameType,
    GlobalMotionRef, parse_frame_header_core,
};
use splot_core::headers::sequence::{
    BitDepthIdc, ChromaFormatIdc, CroppingWindow, SequenceHeader, parse_sequence_header,
};
use splot_core::hls::MultiFrameHeaderRecord;
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;
use splot_recon::{DecodedFrame, DecodedFrameHashInput, PlaneRect, ReconError, SharedFrame};

use crate::bitstream::tile_payload::FrameCdfSubset;
use crate::error::{DecodeError, Result};
use crate::filters::deblock;
use crate::prediction::inter;
use crate::reference::buffer as reference_buffer;
use crate::support::capability::missing_capability_message;

pub(crate) fn effective_allow_screen_content_tools(core: &FrameHeaderCore) -> bool {
    core.allow_screen_content_tools
        .or_else(|| {
            core.inter
                .as_ref()
                .and_then(|inter| inter.allow_screen_content_tools)
        })
        .unwrap_or(false)
}

pub(crate) enum PipelineDecodedFrame {
    Eight(SharedFrame<u8>),
    Ten(SharedFrame<u16>),
}

impl PipelineDecodedFrame {
    fn share(&self) -> Self {
        match self {
            Self::Eight(frame) => Self::Eight(frame.share()),
            Self::Ten(frame) => Self::Ten(frame.share()),
        }
    }

    pub(super) fn handle_count(&self) -> usize {
        match self {
            Self::Eight(frame) => frame.handle_count(),
            Self::Ten(frame) => frame.handle_count(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveFilmGrain {
    pub(crate) model: FilmGrainModel,
    pub(crate) grain_seed: u16,
}

pub(crate) fn deblock_quant_deltas(
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
) -> deblock::DeblockQuantDeltas {
    let (Some(tq), Some(quant)) = (
        sequence.transform_quant_entropy.as_ref(),
        core.quantization_params,
    ) else {
        return deblock::DeblockQuantDeltas::ZERO;
    };
    let seq_quant = CoreSeqQuantView::from_sequence_configs(&sequence.general, tq);
    deblock::DeblockQuantDeltas::from_frame_quant(quant, seq_quant.base_uv_ac_delta_q)
}

pub(crate) struct PipelineFrame {
    pub(crate) frame: PipelineDecodedFrame,
    pub(crate) display_grain: Option<ActiveFilmGrain>,
    pub(crate) output_effects: super::output_effects::FrameOutputEffects,
    pub(crate) frame_cdfs: Arc<FrameCdfSubset>,
    pub(crate) motion_field: inter::TemporalMotionField,
    pub(crate) ccso_params: Option<splot_core::headers::frame::CcsoParams>,
    pub(crate) ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    pub(crate) frame_rate_numerator: u32,
    pub(crate) frame_rate_denominator: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PipelineFrameRate {
    pub(super) numerator: u32,
    pub(super) denominator: u32,
}

impl PipelineFrameRate {
    pub(super) const ANNEX_B_DEFAULT: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    pub(crate) const fn from_ivf_header(header: IvfHeader) -> Self {
        Self {
            numerator: header.timebase_denominator,
            denominator: header.timebase_numerator,
        }
    }
}

impl PipelineFrame {
    pub(crate) fn frame_eight(&self) -> Result<&DecodedFrame<u8>> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => Ok(frame.get()),
            PipelineDecodedFrame::Ten(_) => Err(unsupported(
                "unsupported_10bit_reference_retention",
                None,
                missing_capability_message!("reference.retention bit_depth=10"),
            )),
        }
    }
    pub(crate) fn frame_ten(&self) -> Result<&DecodedFrame<u16>> {
        match &self.frame {
            PipelineDecodedFrame::Ten(frame) => Ok(frame.get()),
            PipelineDecodedFrame::Eight(_) => Err(unsupported(
                "unsupported_8bit_reference_for_10bit_decode",
                None,
                "inter decode pipeline requires reference frames to match the active 10-bit storage",
            )),
        }
    }
    pub(crate) fn byte_len(&self) -> Result<usize> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => {
                Ok(DecodedFrameHashInput::new(frame.get()).byte_len()?)
            }
            PipelineDecodedFrame::Ten(frame) => {
                Ok(DecodedFrameHashInput::new(frame.get()).byte_len()?)
            }
        }
    }

    pub(crate) fn validate_output_effects(&self) -> Result<()> {
        self.output_effects.validate_for_output()
    }

    pub(crate) fn share_decoded_frame(&self) -> PipelineDecodedFrame {
        self.frame.share()
    }
    #[cfg(test)]
    #[allow(clippy::panic)]
    pub(crate) fn frame(&self) -> &DecodedFrame<u8> {
        match &self.frame {
            PipelineDecodedFrame::Eight(frame) => frame.get(),
            PipelineDecodedFrame::Ten(_) => panic!("frame() called on a 10-bit PipelineFrame"),
        }
    }
}

pub(super) struct FilmGrainSlots {
    models: [Option<FilmGrainModel>; MAX_FILM_GRAIN],
}

impl FilmGrainSlots {
    pub(super) fn new() -> Self {
        Self {
            models: std::array::from_fn(|_| None),
        }
    }

    pub(super) fn update_from_obus(&mut self, obus: &[ObuEnvelope<'_>]) -> Result<()> {
        for envelope in obus {
            if envelope.header.obu_type == ObuType::FilmGrain {
                self.update_from_obu(*envelope)?;
            }
        }
        Ok(())
    }

    fn update_from_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let film_grain = parse_film_grain(&mut reader).map_err(|_| {
            unsupported_feature_at(
                "film_grain_obu_parse",
                envelope.offset,
                "runtime film-grain synthesis requires a fully parseable film_grain_obu",
                "5.14",
            )
        })?;
        for update in film_grain.models {
            let slot = usize::from(update.slot);
            let Some(target) = self.models.get_mut(slot) else {
                return Err(unsupported_feature_at(
                    "film_grain_slot_out_of_range",
                    envelope.offset,
                    "film_grain_obu updated a model slot outside MAX_FILM_GRAIN",
                    "5.14",
                ));
            };
            *target = Some(update.model);
        }
        Ok(())
    }

    pub(super) fn active_for_core(
        &self,
        core: &FrameHeaderCore,
        frame_offset: ByteOffset,
    ) -> Result<Option<ActiveFilmGrain>> {
        let Some(config) = film_grain_config_for_core(core) else {
            return Ok(None);
        };
        if !config.apply_grain {
            return Ok(None);
        }
        let fgm_id = config.fgm_id.ok_or_else(|| {
            unsupported_feature_at(
                "film_grain_config_missing_fgm_id",
                frame_offset,
                "apply_grain frames must carry fgm_id for load_grain_model",
                "5.18.10.1",
            )
        })?;
        let grain_seed = config.grain_seed.ok_or_else(|| {
            unsupported_feature_at(
                "film_grain_config_missing_seed",
                frame_offset,
                "apply_grain frames must carry grain_seed for film-grain synthesis",
                "5.18.10.1",
            )
        })?;
        let model = self
            .models
            .get(usize::from(fgm_id))
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| {
                unsupported_feature_at(
                    "film_grain_model_unavailable",
                    frame_offset,
                    "apply_grain references an fgm_id whose model slot is unavailable",
                    "6.17.10.1",
                )
            })?;
        Ok(Some(ActiveFilmGrain { model, grain_seed }))
    }
}

fn film_grain_config_for_core(core: &FrameHeaderCore) -> Option<FilmGrainConfig> {
    if let Some(tail) = core.intra_tail.as_ref() {
        return Some(tail.film_grain);
    }
    if let Some(config) = core.sef_film_grain {
        return Some(config);
    }
    if let Some(config) = core.inter.as_ref().and_then(|inter| inter.tip_film_grain) {
        return Some(config);
    }
    if let Some(config) = core.bridge_film_grain {
        return Some(config);
    }
    if let Some(tail) = core.inter_tail.as_ref() {
        return Some(tail.film_grain);
    }
    None
}

pub(crate) fn parse_sequence(envelope: ObuEnvelope<'_>) -> Result<SequenceHeader> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    parse_sequence_header(&mut reader).map_err(|_| {
        unsupported_at(
            "sequence_header_parse",
            envelope.offset,
            "decode runtime requires a fully parseable sequence header",
        )
    })
}

pub(super) fn validate_sequence(sequence: &SequenceHeader, offset: ByteOffset) -> Result<()> {
    let general = &sequence.general;
    if !sequence.is_fully_parsed() {
        return Err(unsupported_at(
            "sequence_header_not_fully_parsed",
            offset,
            "decode runtime requires a fully parsed sequence header",
        ));
    }
    if !supported_profile_chroma(general.seq_profile_idc.get(), general.chroma_format_idc) {
        return Err(unsupported_at(
            "unsupported_profile",
            offset,
            "decode runtime requires a supported Annex A profile/chroma combination",
        ));
    }
    if sequence.intra.is_none() {
        return Err(unsupported_at(
            "missing_sequence_intra_config",
            offset,
            "decode runtime requires a fully parsed sequence intra config",
        ));
    }
    Ok(())
}

pub(crate) fn derive_visible_luma_rect(
    sequence: &SequenceHeader,
    frame_width: u32,
    frame_height: u32,
) -> Result<PlaneRect> {
    let general = &sequence.general;
    Ok(derive_visible_luma_rect_from_offsets(
        general.cropping_window,
        general.max_frame_width.get(),
        general.max_frame_height.get(),
        frame_width,
        frame_height,
    )?)
}

fn derive_visible_luma_rect_from_offsets(
    crop: CroppingWindow,
    max_frame_width: u32,
    max_frame_height: u32,
    frame_width: u32,
    frame_height: u32,
) -> splot_recon::Result<PlaneRect> {
    let frame_width = u64::from(frame_width);
    let frame_height = u64::from(frame_height);
    let max_frame_width = u64::from(max_frame_width);
    let max_frame_height = u64::from(max_frame_height);

    let left = u64::from(crop.left) * frame_width / max_frame_width;
    let right = frame_width - u64::from(crop.right) * frame_width / max_frame_width;
    let top = u64::from(crop.top) * frame_height / max_frame_height;
    let bottom = frame_height - u64::from(crop.bottom) * frame_height / max_frame_height;
    let width = positive_crop_extent(right, left, "visible width")?;
    let height = positive_crop_extent(bottom, top, "visible height")?;

    PlaneRect::new(crop_usize(left)?, crop_usize(top)?, width, height)
}

fn positive_crop_extent(end: u64, start: u64, field: &'static str) -> splot_recon::Result<usize> {
    let extent = end
        .checked_sub(start)
        .filter(|extent| *extent != 0)
        .ok_or(ReconError::ZeroDimension { field })?;
    crop_usize(extent)
}

fn crop_usize(value: u64) -> splot_recon::Result<usize> {
    usize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow {
        context: "sequence crop window",
    })
}

fn supported_profile_chroma(profile_idc: u8, chroma: ChromaFormatIdc) -> bool {
    match profile_idc {
        0..=2 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
        ),
        3 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv422
        ),
        4 => matches!(
            chroma,
            ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Yuv444
        ),
        _ => false,
    }
}

#[allow(clippy::unnecessary_wraps)]
pub(crate) fn ensure_runtime_storage_bit_depth(
    sequence: &SequenceHeader,
    _offset: ByteOffset,
) -> Result<()> {
    match sequence.general.bit_depth_idc {
        BitDepthIdc::Eight | BitDepthIdc::Ten => Ok(()),
    }
}

pub(crate) fn parse_frame_core(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
) -> Result<FrameHeaderCore> {
    parse_frame_core_with_mfh(envelope, sequence, None)
}

pub(crate) fn parse_frame_core_with_mfh(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
) -> Result<FrameHeaderCore> {
    parse_frame_core_with_reference(
        envelope,
        sequence,
        mfh_record,
        true,
        &FrameReferenceStateView::unknown(),
    )
}

pub(crate) fn parse_frame_core_with_reference(
    envelope: ObuEnvelope<'_>,
    sequence: &SequenceHeader,
    mfh_record: Option<&MultiFrameHeaderRecord>,
    first_picture_in_tu: bool,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<FrameHeaderCore> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let is_first_tile_group = reader.read_bit().map_err(|_| {
        unsupported_at(
            "tile_group_prefix_parse",
            envelope.offset,
            "decode runtime requires a parseable first tile-group prefix",
        )
    })? != 0;
    if !is_first_tile_group {
        return Err(unsupported_at(
            "non_first_tile_group",
            envelope.offset,
            "decode runtime requires the frame header in the first tile group",
        ));
    }
    let input = FrameHeaderParseInput {
        obu_type: envelope.header.obu_type,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: *reference_state,
        mode: FrameHeaderParseMode::Core,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "frame_header_parse",
            envelope.offset,
            "decode runtime requires a fully parseable closed-loop-key frame header",
        )
    })
}
pub(crate) fn frame_ref_update_from_core(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    frame_cdfs: FrameCdfSubset,
    ccso_params: Option<splot_core::headers::frame::CcsoParams>,
    ccso_grid: Option<crate::filters::ccso::CcsoUnitGrid>,
    motion_field: inter::TemporalMotionField,
    embedded_layer_id: splot_core::types::EmbeddedLayerId,
) -> Result<reference_buffer::FrameRefUpdate> {
    let refresh_frame_flags = core.refresh_frame_flags.ok_or_else(|| {
        unsupported_at(
            "missing_refresh_frame_flags",
            offset,
            "multi-frame decode requires a parsed refresh_frame_flags for the §7.23 update",
        )
    })?;
    let frame_size = core.frame_size.ok_or_else(|| {
        unsupported_at(
            "missing_frame_size_for_ref_update",
            offset,
            "multi-frame decode requires a parsed frame size for the §7.23 update",
        )
    })?;
    let quantization = core.quantization_params.ok_or_else(|| {
        unsupported_at(
            "missing_base_q_for_ref_update",
            offset,
            "multi-frame decode requires a parsed base_q_idx for the §7.23 update",
        )
    })?;
    let mut frame_cdfs = frame_cdfs;
    frame_cdfs
        .replicate_coeff_q_context_for_base_q(quantization.base_q_idx)
        .map_err(|_| {
            unsupported_at(
                "reference_coefficient_cdf_context",
                offset,
                "reference refresh requires a valid coefficient CDF context",
            )
        })?;
    let is_inter = core.frame_type == Some(FrameType::Inter);
    let adapted = !core.obu_type.is_tip_frame()
        && core.obu_type != ObuType::BridgeFrame
        && core.disable_cdf_update != Some(true);
    let order_hint = core.display_order_hint().ok_or_else(|| {
        unsupported_at(
            "missing_order_hint_for_ref_update",
            offset,
            "the § 7.23 reference update requires a derived OrderHint",
        )
    })?;
    let order_hint_lsb = core.order_hint_lsb.ok_or_else(|| {
        unsupported_at(
            "missing_order_hint_lsb_for_ref_update",
            offset,
            "the § 7.23 reference update requires OrderHintLsbs",
        )
    })?;
    let mut saved_order_hints = [0; 7];
    let mut saved_gm_params = [GlobalMotionRef::identity().gm_params; 7];
    let num_total_refs = core
        .inter
        .as_ref()
        .and_then(|inter| inter.num_total_refs)
        .unwrap_or(0);
    if let Some(inter) = core.inter.as_ref() {
        for (target, &hint) in saved_order_hints.iter_mut().zip(&inter.order_hints) {
            *target = hint;
        }
    }
    if let Some(tail) = core.inter_tail.as_ref() {
        for (target, model) in saved_gm_params
            .iter_mut()
            .zip(tail.global_motion.references)
        {
            *target = model.gm_params;
        }
    }
    Ok(reference_buffer::FrameRefUpdate {
        refresh_frame_flags,
        order_hint,
        order_hint_lsb,
        implicit_output_frame: core.implicit_output_frame == Some(true),
        immediate_output_frame: core.immediate_output_frame == Some(true),
        width: frame_size.width,
        height: frame_size.height,
        base_q_idx: quantization.base_q_idx,
        delta_q_u_ac: quantization.delta_q_u_ac,
        delta_q_v_ac: quantization.delta_q_v_ac,
        is_key_or_switch: (core.is_key_frame && !core.is_bridge)
            || core.frame_type == Some(FrameType::Switch),
        is_inter,
        adapted,
        num_total_refs,
        saved_order_hints,
        saved_gm_params,
        frame_cdfs: Arc::new(frame_cdfs),
        ccso_params,
        ccso_grid,
        motion_field,
        long_term_id: core.long_term_id.and_then(|id| u32::try_from(id).ok()),
        embedded_layer_id,
        lr_frame_filter_class_counts: lr_frame_filter_class_counts(core),
        lr_frame_filter_taps: lr_frame_filter_taps(core),
    })
}

fn lr_frame_filter_taps(core: &FrameHeaderCore) -> [Vec<Vec<i16>>; 3] {
    let mut taps: [Vec<Vec<i16>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let Some(lr) = core.lr_params.as_ref() else {
        return taps;
    };
    for (plane, params) in lr.planes.iter().enumerate().take(3) {
        if !params.frame_filters_on {
            continue;
        }
        let Some(bank) = params.frame_filter_bank.as_ref() else {
            continue;
        };
        taps[plane] = bank
            .classes
            .iter()
            .map(|class| class.coeffs.clone())
            .collect();
    }
    taps
}

fn lr_frame_filter_class_counts(core: &FrameHeaderCore) -> [u8; 3] {
    let mut counts = [0u8; 3];
    let Some(lr) = core.lr_params.as_ref() else {
        return counts;
    };
    for (plane, params) in lr.planes.iter().enumerate().take(3) {
        if !params.frame_filters_on {
            continue;
        }
        let classes = params
            .frame_filter_bank
            .as_ref()
            .map(|bank| bank.classes.len())
            .or_else(|| params.num_filter_classes.map(usize::from))
            .unwrap_or(1);
        counts[plane] = u8::try_from(classes).unwrap_or(u8::MAX);
    }
    counts
}

pub(super) fn ensure_intra_header_complete(
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(incomplete_intra_header_error(core.status, offset));
    }
    Ok(())
}

pub(crate) fn incomplete_intra_header_error(
    status: FrameHeaderParseStatus,
    offset: ByteOffset,
) -> DecodeError {
    match status {
        FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { .. } => unsupported_feature_at(
            "unsupported_wienerns_filter",
            offset,
            missing_capability_message!("filters.wiener_ns read_wienerns_filter §5.18.7.11"),
            "5.18.7.11",
        ),
        _ => unsupported_at(
            "incomplete_frame_header",
            offset,
            "decode runtime requires a complete intra frame header",
        ),
    }
}

#[cfg(test)]
mod crop_tests {
    use super::*;

    #[test]
    fn crop_window_derives_visible_rect_at_sequence_maximum() -> splot_recon::Result<()> {
        let rect = derive_visible_luma_rect_from_offsets(
            CroppingWindow {
                left: 2,
                right: 4,
                top: 6,
                bottom: 8,
            },
            64,
            64,
            64,
            64,
        )?;

        assert_eq!(rect, PlaneRect::new(2, 6, 58, 50)?);
        Ok(())
    }

    #[test]
    fn crop_window_scales_offsets_for_smaller_frames() -> splot_recon::Result<()> {
        let rect = derive_visible_luma_rect_from_offsets(
            CroppingWindow {
                left: 3,
                right: 5,
                top: 7,
                bottom: 9,
            },
            128,
            128,
            64,
            64,
        )?;

        assert_eq!(rect, PlaneRect::new(1, 3, 61, 57)?);
        Ok(())
    }

    #[test]
    fn crop_window_rejects_nonpositive_visible_extent() {
        let error = derive_visible_luma_rect_from_offsets(
            CroppingWindow {
                left: 3,
                right: 1,
                top: 0,
                bottom: 0,
            },
            4,
            4,
            4,
            4,
        );

        assert!(matches!(
            error,
            Err(ReconError::ZeroDimension {
                field: "visible width"
            })
        ));
    }
}
