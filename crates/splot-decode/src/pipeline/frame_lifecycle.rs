// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame parsing, retained state, and lifecycle handoffs.

use super::{unsupported, unsupported_at, unsupported_feature_at};

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;

use splot_core::headers::film_grain::{FilmGrainModel, MAX_FILM_GRAIN, parse_film_grain};
use splot_core::headers::frame::{
    CoreSeqQuantView, FilmGrainConfig, FrameHeaderCore, FrameHeaderParseInput,
    FrameHeaderParseStatus, FrameReferenceStateView, FrameType, parse_frame_header_core,
};
use splot_core::headers::sequence::{
    ChromaFormatIdc, CroppingWindow, ProfileIdc, SequenceHeader, parse_sequence_header,
};
use splot_core::hls::MultiFrameHeaderRecord;
use splot_core::ivf::IvfHeader;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ObuType, TemporalLayerId};
#[cfg(test)]
use splot_recon::DecodedFrame;
use splot_recon::{PlaneRect, ReconError, SharedFrame};

use super::inflight::{PipelineFrameSlot, RefFrameSlot};
use crate::bitstream::stream_plan::DecodeSourceIssue;
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
    pub(crate) frame: PipelineFrameSlot,
    pub(crate) display_grain: Option<ActiveFilmGrain>,
    pub(crate) output_effects: super::output_effects::FrameOutputEffects,
    pub(crate) frame_cdfs: inter::FrameCdfHandle,
    pub(crate) motion_field: inter::MotionFieldHandle,
    /// The frame's header: CCSO reads it, and a picture takes its frame type
    /// from it. Every decoded frame has one.
    pub(crate) ccso_header: Option<std::sync::Arc<FrameHeaderCore>>,
    pub(crate) ccso_grid: inter::CcsoGridHandle,
    pub(crate) segment_ids: inter::SegmentIdMapHandle,
    pub(crate) frame_rate: PipelineFrameRate,
    /// Temporal layer id of the coded or show-existing frame OBU.
    pub(crate) temporal_layer_id: TemporalLayerId,
    /// The caller data of the temporal unit that coded or showed this frame.
    pub(crate) props: crate::DataProps,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PipelineFrameRate {
    pub(crate) numerator: u32,
    pub(crate) denominator: u32,
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
    pub(crate) fn slot_eight(&self) -> Result<RefFrameSlot<u8>> {
        self.frame
            .eight()
            .ok_or_else(|| mismatched_slot_storage(&self.frame))
    }
    pub(crate) fn slot_ten(&self) -> Result<RefFrameSlot<u16>> {
        self.frame
            .ten()
            .ok_or_else(|| mismatched_slot_storage(&self.frame))
    }
    pub(crate) fn byte_len(&self) -> Result<usize> {
        Ok(splot_recon::visible_byte_len(self.frame.info())?)
    }

    /// The grain output adds to this frame: none when the caller skips it.
    pub(crate) fn output_grain(&self, options: &crate::DecodeOptions) -> Option<&ActiveFilmGrain> {
        self.display_grain
            .as_ref()
            .filter(|_| !options.skip_film_grain())
    }

    pub(crate) fn validate_output_effects(&self) -> Result<()> {
        self.output_effects.validate_for_output()
    }

    /// Shares the decoded samples, failing closed when they have not landed.
    pub(crate) fn ready_frame(&self) -> Result<PipelineDecodedFrame> {
        self.frame.ready()
    }

    /// Blocks the driver thread until the samples land, then shares them.
    pub(crate) fn wait_ready_frame(&self) -> Result<PipelineDecodedFrame> {
        self.frame.wait_ready()
    }

    /// Blocks the driver thread until the frame's filter phase has settled.
    pub(crate) fn wait_settled(&self) -> Result<()> {
        self.frame.wait_settled()
    }
}

#[cfg(test)]
impl PipelineDecodedFrame {
    pub(crate) fn frame(&self) -> &DecodedFrame<u8> {
        match self {
            Self::Eight(frame) => frame,
            Self::Ten(_) => panic!("frame() called on a 10-bit frame"),
        }
    }
}

/// Reports the slot whose storage depth does not match the requested one.
fn mismatched_slot_storage(frame: &PipelineFrameSlot) -> DecodeError {
    match frame {
        PipelineFrameSlot::Eight(_) => unsupported(
            "unsupported_8bit_reference_for_10bit_decode",
            None,
            "inter decode pipeline requires reference frames to match the active 10-bit storage",
        ),
        PipelineFrameSlot::Ten(_) => unsupported(
            "unsupported_10bit_reference_retention",
            None,
            missing_capability_message!("reference.retention bit_depth=10"),
        ),
    }
}

#[derive(Clone)]
struct FilmGrainSlot {
    model: FilmGrainModel,
    embedded_layer_id: EmbeddedLayerId,
    temporal_layer_id: TemporalLayerId,
}

pub(super) struct FilmGrainSlots {
    models: [Option<FilmGrainSlot>; MAX_FILM_GRAIN],
}

impl FilmGrainSlots {
    pub(super) fn new() -> Self {
        Self {
            models: std::array::from_fn(|_| None),
        }
    }

    pub(super) fn update_from_obus(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        sequence: &SequenceHeader,
    ) -> Result<()> {
        for envelope in obus {
            if envelope.header.obu_type == ObuType::FilmGrain {
                self.update_from_obu(*envelope, sequence)?;
            }
        }
        Ok(())
    }

    fn update_from_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        sequence: &SequenceHeader,
    ) -> Result<()> {
        if envelope.header.temporal_layer_id > sequence.general.max_tlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "film grain OBU temporal_layer_id {} exceeds max_tlayer_id {}",
                        envelope.header.temporal_layer_id.get(),
                        sequence.general.max_tlayer_id.get()
                    ),
                ),
            });
        }
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
            *target = Some(FilmGrainSlot {
                model: update.model,
                embedded_layer_id: envelope.header.embedded_layer_id,
                temporal_layer_id: envelope.header.temporal_layer_id,
            });
        }
        Ok(())
    }

    pub(super) fn active_for_core(
        &self,
        envelope: ObuEnvelope<'_>,
        core: &FrameHeaderCore,
        sequence: &SequenceHeader,
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
        let slot = self
            .models
            .get(usize::from(fgm_id))
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                unsupported_feature_at(
                    "film_grain_model_unavailable",
                    frame_offset,
                    "apply_grain references an fgm_id whose model slot is unavailable",
                    "6.17.10.1",
                )
            })?;
        if !sequence
            .general
            .mlayer_dependency_map
            .depends_on(envelope.header.embedded_layer_id, slot.embedded_layer_id)
            || !sequence.general.tlayer_dependency_map.depends_on(
                envelope.header.embedded_layer_id,
                envelope.header.temporal_layer_id,
                slot.temporal_layer_id,
            )
        {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    frame_offset,
                    None,
                    "6.17.10.1",
                    format!(
                        "film grain model fgm_id {fgm_id} violates active layer dependency maps"
                    ),
                ),
            });
        }
        Ok(Some(ActiveFilmGrain {
            model: slot.model.clone(),
            grain_seed,
        }))
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
    parse_sequence_header(&mut reader).map_err(|error| DecodeError::MalformedSource {
        issue: DecodeSourceIssue::sequence_header_parse(
            envelope.offset,
            "5.4.1",
            error.to_string(),
        ),
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
    if let Err(error) = validate_profile_chroma(general.seq_profile_idc, general.chroma_format_idc)
    {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::sequence_header_conformance(offset, "A.2", error.to_string()),
        });
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
enum SequenceProfileError {
    #[error("seq_profile_idc {profile_idc} is reserved by Annex A.2 Table A.1")]
    Reserved { profile_idc: u8 },
    #[error(
        "chroma_format_idc {chroma_format_idc} is not permitted for seq_profile_idc {profile_idc} by Annex A.2 Table A.1"
    )]
    ChromaFormatMismatch {
        profile_idc: u8,
        chroma_format_idc: u8,
    },
}

fn validate_profile_chroma(
    profile: ProfileIdc,
    chroma: ChromaFormatIdc,
) -> core::result::Result<(), SequenceProfileError> {
    let profile_idc = profile.get();
    if profile.is_reserved() {
        return Err(SequenceProfileError::Reserved { profile_idc });
    }
    if profile.allows_chroma(chroma) {
        Ok(())
    } else {
        Err(SequenceProfileError::ChromaFormatMismatch {
            profile_idc,
            chroma_format_idc: chroma.get(),
        })
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
    if envelope.header.temporal_layer_id > sequence.general.max_tlayer_id {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_header_conformance(
                envelope.offset,
                None,
                "6.2.2",
                format!(
                    "frame OBU temporal_layer_id {} exceeds max_tlayer_id {}",
                    envelope.header.temporal_layer_id.get(),
                    sequence.general.max_tlayer_id.get()
                ),
            ),
        });
    }
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
        obu_tlayer_id: envelope.header.temporal_layer_id,
        first_picture_in_tu,
        active_sequence: Some(sequence),
        mfh_record,
        reference_state: *reference_state,
    };
    parse_frame_header_core(&mut reader, &input).map_err(|_| {
        unsupported_at(
            "frame_header_parse",
            envelope.offset,
            "decode runtime requires a fully parseable closed-loop-key frame header",
        )
    })
}

/// AV2 § 5.18.2 `keyFrame` (from the OBU type, not `FrameType`) or `FrameType == SWITCH_FRAME`;
/// consumed by the § 7.23 `RefValid` update (mirror `07-decoding-process.md` line 14100).
pub(crate) fn is_key_or_switch(core: &FrameHeaderCore) -> bool {
    core.is_key_frame || core.frame_type == Some(FrameType::Switch)
}

pub(crate) fn frame_ref_update_from_core(
    core: &FrameHeaderCore,
    offset: ByteOffset,
    embedded_layer_id: EmbeddedLayerId,
    temporal_layer_id: TemporalLayerId,
) -> Result<reference_buffer::FrameRefUpdate<'_>> {
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
    let is_inter = core.frame_type == Some(FrameType::Inter);
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
    let (num_total_refs, saved_order_hints, saved_gm_params) = core.saved_global_motion_state();
    let mut lr_frame_filter_class_counts = [0; 3];
    let mut lr_banks = [None; 3];
    if let Some(lr) = &core.lr_params {
        for (plane, params) in lr.planes.iter().enumerate() {
            if params.frame_filters_on {
                let bank = params.frame_filter_bank.as_ref();
                lr_banks[plane] = bank;
                lr_frame_filter_class_counts[plane] = bank.map_or_else(
                    || params.num_filter_classes.unwrap_or(1),
                    |bank| bank.classes.len() as u8,
                );
            }
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
        is_inter,
        is_intra_frame: matches!(core.frame_type, Some(FrameType::Key | FrameType::IntraOnly)),
        num_total_refs,
        saved_order_hints,
        saved_gm_params,
        long_term_id: core.long_term_id.and_then(|id| u32::try_from(id).ok()),
        embedded_layer_id,
        temporal_layer_id,
        lr_frame_filter_class_counts,
        lr_banks,
    })
}

pub(super) fn ensure_intra_header_complete(
    core: &FrameHeaderCore,
    offset: ByteOffset,
) -> Result<()> {
    if core.status != FrameHeaderParseStatus::IntraHeaderComplete {
        return Err(unsupported_at(
            "incomplete_frame_header",
            offset,
            "decode runtime requires a complete intra frame header",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod sequence_profile_tests {

    use super::*;
    use crate::DecodeSourceIssueKind;
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

    const INTRA_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf");
    const PROFILE31_FIXTURE: &[u8] = include_bytes!(
        "../../../../tests/conformance/vectors/valid/syn-profile31-mono-intra-16x16.ivf"
    );

    const ALL_CHROMA: [ChromaFormatIdc; 4] = [
        ChromaFormatIdc::Monochrome,
        ChromaFormatIdc::Yuv420,
        ChromaFormatIdc::Yuv422,
        ChromaFormatIdc::Yuv444,
    ];

    fn sequence() -> SequenceHeader {
        crate::prediction::inter::test_support::fixture_sequence_and_key_core(INTRA_FIXTURE).0
    }

    #[test]
    fn oracle_fixture_carries_configurable_monochrome_profile() {
        let sequence = crate::prediction::inter::test_support::fixture_sequence_and_key_core(
            PROFILE31_FIXTURE,
        )
        .0;
        assert_eq!(sequence.general.seq_profile_idc, ProfileIdc::Configurable);
        assert_eq!(
            sequence.general.chroma_format_idc,
            ChromaFormatIdc::Monochrome
        );
        validate_sequence(&sequence, ByteOffset::new(47)).unwrap();
    }

    #[test]
    fn configurable_profile_accepts_every_table_a1_chroma_format() {
        for chroma in ALL_CHROMA {
            let mut sequence = sequence();
            sequence.general.seq_profile_idc = ProfileIdc::Configurable;
            sequence.general.chroma_format_idc = chroma;
            validate_sequence(&sequence, ByteOffset::new(47)).unwrap();
        }
    }

    #[test]
    fn fixed_profiles_match_table_a1_chroma_sets() {
        for profile_idc in 0_u8..=4 {
            for chroma in ALL_CHROMA {
                let expected = match profile_idc {
                    0..=2 => matches!(
                        chroma,
                        ChromaFormatIdc::Monochrome | ChromaFormatIdc::Yuv420
                    ),
                    3 => matches!(
                        chroma,
                        ChromaFormatIdc::Monochrome
                            | ChromaFormatIdc::Yuv420
                            | ChromaFormatIdc::Yuv422
                    ),
                    4 => matches!(
                        chroma,
                        ChromaFormatIdc::Monochrome
                            | ChromaFormatIdc::Yuv420
                            | ChromaFormatIdc::Yuv444
                    ),
                    _ => false,
                };
                assert_eq!(
                    validate_profile_chroma(ProfileIdc::from_bits(profile_idc), chroma).is_ok(),
                    expected,
                    "profile {profile_idc}, chroma {}",
                    chroma.get()
                );
            }
        }
    }

    #[test]
    fn reserved_profiles_are_malformed_for_every_chroma_format() {
        for profile_idc in 5_u8..=30 {
            for chroma in ALL_CHROMA {
                let error = validate_profile_chroma(ProfileIdc::from_bits(profile_idc), chroma)
                    .unwrap_err();
                assert_eq!(error, SequenceProfileError::Reserved { profile_idc });
            }
        }

        let mut sequence = sequence();
        sequence.general.seq_profile_idc = ProfileIdc::from_bits(5);
        let error = validate_sequence(&sequence, ByteOffset::new(47)).unwrap_err();
        assert!(matches!(
            error,
            DecodeError::MalformedSource { issue }
                if issue.kind() == DecodeSourceIssueKind::SequenceHeaderConformanceError
                    && issue.spec_section() == Some("A.2")
                    && issue.offset() == Some(ByteOffset::new(47))
                    && issue.message().contains("seq_profile_idc 5 is reserved")
        ));
    }

    #[test]
    fn fixed_profile_chroma_mismatch_is_malformed() {
        let mut sequence = sequence();
        sequence.general.seq_profile_idc = ProfileIdc::Main420Ip0;
        sequence.general.chroma_format_idc = ChromaFormatIdc::Yuv444;

        let error = validate_sequence(&sequence, ByteOffset::new(47)).unwrap_err();
        assert!(matches!(
            error,
            DecodeError::MalformedSource { issue }
                if issue.kind() == DecodeSourceIssueKind::SequenceHeaderConformanceError
                    && issue.spec_section() == Some("A.2")
                    && issue.offset() == Some(ByteOffset::new(47))
                    && issue.message().contains("chroma_format_idc 2")
        ));
    }

    #[test]
    fn truncated_sequence_header_is_a_typed_parse_error() {
        let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(INTRA_FIXTURE) else {
            panic!("fixture is IVF");
        };
        let envelope = parsed
            .frames
            .iter()
            .flat_map(|frame| frame.obus.iter())
            .find(|envelope| envelope.header.obu_type == ObuType::SequenceHeader)
            .copied()
            .expect("fixture carries a sequence header");
        let truncated = ObuEnvelope {
            payload: &envelope.payload[..1],
            ..envelope
        };

        let error = parse_sequence(truncated).unwrap_err();
        assert!(matches!(
            error,
            DecodeError::MalformedSource { issue }
                if issue.kind() == DecodeSourceIssueKind::SequenceHeaderParseError
                    && issue.spec_section() == Some("5.4.1")
                    && issue.offset() == Some(envelope.offset)
        ));
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

#[cfg(test)]
mod completeness_tests {
    use super::*;

    const INTRA_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-intra-64x64-10bit.ivf");

    #[test]
    fn incomplete_header_keeps_generic_runtime_frontier() {
        let (_, mut core) =
            crate::prediction::inter::test_support::fixture_sequence_and_key_core(INTRA_FIXTURE);
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: "AV2-5.18-FRAME-HEADER",
        };

        let result = ensure_intra_header_complete(&core, ByteOffset::new(74));
        assert!(matches!(
            &result,
            Err(DecodeError::UnsupportedFeature { .. })
        ));
        if let Err(DecodeError::UnsupportedFeature { unsupported }) = result {
            assert_eq!(unsupported.reason(), "incomplete_frame_header");
            assert_eq!(unsupported.spec_section(), "7.1");
            assert_eq!(unsupported.byte_offset(), Some(ByteOffset::new(74)));
        }
    }
}

#[cfg(test)]
mod film_grain_layer_tests {
    use super::*;
    use crate::DecodeSourceIssueKind;
    use crate::prediction::inter::test_support::fixture_sequence_and_key_core;
    use splot_core::annexb::parse_annex_b_obus;
    use splot_core::bitio::BitReader;
    use splot_core::headers::sequence::parse_sequence_header;
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

    #[test]
    fn active_film_grain_slot_rejects_disallowed_temporal_layer_dependency() {
        const FGM_FIXTURE: &[u8] = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-filmgrain-intra-64x64.ivf"
        );
        const SPARSE_OBU: &[u8] = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-tlayer4-sparse-dependencies.obu"
        );

        let (mut sequence, core) = fixture_sequence_and_key_core(FGM_FIXTURE);
        let sparse_obus = parse_annex_b_obus(SPARSE_OBU).unwrap();
        let sparse_seq_obu = sparse_obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
            .unwrap();
        let sparse_seq = parse_sequence_header(&mut BitReader::new(
            sparse_seq_obu.payload,
            sparse_seq_obu.payload_offset(),
        ))
        .unwrap();
        sequence.general.max_tlayer_id = sparse_seq.general.max_tlayer_id;
        sequence.general.tlayer_dependency_map = sparse_seq.general.tlayer_dependency_map;

        let ParsedBitstream::Ivf(parsed) = parse_bitstream_partial(FGM_FIXTURE) else {
            panic!("fixture is IVF");
        };
        let mut fgm_envelope = parsed
            .frames
            .iter()
            .flat_map(|frame| frame.obus.iter())
            .find(|obu| obu.header.obu_type == ObuType::FilmGrain)
            .copied()
            .expect("fixture carries film grain OBU");
        let mut frame_envelope = parsed
            .frames
            .iter()
            .flat_map(|frame| frame.obus.iter())
            .find(|obu| obu.header.obu_type == ObuType::ClosedLoopKey)
            .copied()
            .expect("fixture carries key frame OBU");

        fgm_envelope.header.temporal_layer_id = TemporalLayerId::from_bits(1);
        let mut slots = FilmGrainSlots::new();
        slots.update_from_obus(&[fgm_envelope], &sequence).unwrap();

        frame_envelope.header.temporal_layer_id = TemporalLayerId::from_bits(1);
        assert!(
            slots
                .active_for_core(frame_envelope, &core, &sequence, frame_envelope.offset)
                .unwrap()
                .is_some()
        );

        frame_envelope.header.temporal_layer_id = TemporalLayerId::from_bits(2);
        let result = slots.active_for_core(frame_envelope, &core, &sequence, frame_envelope.offset);
        assert!(matches!(
            result,
            Err(DecodeError::MalformedSource { issue })
                if issue.kind() == DecodeSourceIssueKind::FrameHeaderConformanceError
                    && issue.spec_section() == Some("6.17.10.1")
                    && issue.message().contains("violates active layer dependency maps")
        ));
    }
}
