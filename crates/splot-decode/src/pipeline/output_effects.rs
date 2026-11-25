// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! High-level output state consumed by one selected-layer decode.

use std::collections::BTreeMap;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::buffer_removal_timing::parse_buffer_removal_timing;
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::layer_config_record::{
    LayerConfigurationRecord, parse_layer_config_record,
};
use splot_core::headers::metadata::{
    MetadataType, MetadataUnit, parse_metadata_group, parse_metadata_short,
};
use splot_core::headers::operating_point_set::{OperatingPointSet, parse_operating_point_set};
use splot_core::headers::quantizer_matrix::{
    FundamentalQmTransform, NUM_CUSTOM_QMS, QuantizerMatrixLevel, parse_quantizer_matrix,
};
use splot_core::headers::sequence::SequenceHeader;
use splot_core::hls::{MAX_MFH_NUM, MfhId, MultiFrameHeaderRecord, parse_multi_frame_header};
use splot_core::obu::finish_obu_payload;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType};
use splot_core::write::metadata::type_matches_payload;
use splot_recon::QmUserPlane;

use super::{PipelineFrameRate, unsupported_feature_at, unsupported_with_spec};
use crate::bitstream::stream_plan::DecodeSourceIssue;
use crate::bitstream::tile_payload::{FrameUserQmLevel, FrameUserQmLevels};
use crate::error::{DecodeError, DecodeHeaderStateError, Result};

const LAYER_GLOBAL: u8 = 1;
const LAYER_CURRENT: u8 = 2;
const LAYER_VALUES: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameOutputEffects {
    pub(crate) content_interpretation: Option<ContentInterpretation>,
    pub(crate) metadata: Vec<MetadataUnit>,
}

impl FrameOutputEffects {
    #[cfg(test)]
    pub(crate) const fn empty() -> Self {
        Self {
            content_interpretation: None,
            metadata: Vec::new(),
        }
    }

    pub(crate) fn frame_rate(&self, fallback: PipelineFrameRate) -> PipelineFrameRate {
        let Some(timing) = self
            .content_interpretation
            .and_then(|content| content.timing_info)
        else {
            return fallback;
        };
        let ticks = timing
            .num_ticks_per_picture_minus_1
            .map_or(1, |minus_1| minus_1.saturating_add(1));
        let denominator = timing.num_units_in_display_tick.saturating_mul(ticks);
        if denominator == 0 {
            fallback
        } else {
            PipelineFrameRate {
                numerator: timing.time_scale,
                denominator,
            }
        }
    }

    pub(crate) fn validate_for_output(&self) -> Result<()> {
        if self
            .content_interpretation
            .is_some_and(|content| content.reserved_2bit != 0)
        {
            return Err(unsupported_with_spec(
                "content_interpretation_reserved_bits",
                None,
                "content interpretation reserved bits must be zero",
                "6.14",
            ));
        }
        if self
            .metadata
            .iter()
            .any(|unit| !type_matches_payload(unit.metadata_type, &unit.payload))
        {
            return Err(unsupported_with_spec(
                "metadata_payload_type",
                None,
                "attached metadata payload does not match its metadata_type",
                "6.16.1",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MetadataPersistence {
    Global,
    Basic,
    No,
    Enhanced,
    Reserved,
}

impl MetadataPersistence {
    pub(crate) const fn from_idc(idc: u8) -> Self {
        match idc {
            0 => Self::Global,
            1 => Self::Basic,
            2 => Self::No,
            3 => Self::Enhanced,
            _ => Self::Reserved,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum MetadataAction {
    Cancel(MetadataType),
    Store {
        layer_idc: u8,
        persistence_idc: u8,
        source_mlayer: EmbeddedLayerId,
        xlayer_map: Option<u32>,
        mlayer_maps: Vec<u8>,
        unit: MetadataUnit,
    },
}

#[derive(Clone, Debug)]
struct ActiveMetadata {
    unit: MetadataUnit,
    persistence: MetadataPersistence,
    layer_idc: u8,
    source_mlayer: EmbeddedLayerId,
    mlayer_map: Option<u8>,
    tu_index: u64,
}

impl ActiveMetadata {
    fn same_scope(&self, other: &Self) -> bool {
        self.layer_idc == other.layer_idc
            && self.source_mlayer == other.source_mlayer
            && self.mlayer_map == other.mlayer_map
    }

    fn applies_to_layer(&self, mlayer: EmbeddedLayerId) -> bool {
        match self.layer_idc {
            0 | LAYER_GLOBAL => true,
            LAYER_CURRENT => self.source_mlayer == mlayer,
            LAYER_VALUES => self
                .mlayer_map
                .is_some_and(|map| map & (1u8 << mlayer.get()) != 0),
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
struct QmSlot {
    mlayer_id: Option<u8>,
    tlayer_id: Option<u8>,
    num_planes: u8,
    user: Option<FrameUserQmLevel>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrtCount {
    None,
    One,
    Multiple,
}

pub(crate) struct OutputEffectState {
    mfh: [Option<MultiFrameHeaderRecord>; MAX_MFH_NUM as usize],
    qm: [Option<QmSlot>; NUM_CUSTOM_QMS],
    qm_protected: u16,
    qm_seen_since_frame: u16,
    qm_obu_seen_since_frame: bool,
    content_interpretation: [Option<ContentInterpretation>; 8],
    ci_in_current_tu: [Option<ContentInterpretation>; 8],
    active_xlayer: ExtendedLayerId,
    local_lcr_tu: Option<u64>,
    local_lcr_in_cvs: bool,
    local_lcr_ids: u8,
    global_lcr_xlayers: [Option<u32>; 8],
    brt_count: BrtCount,
    second_brt_offset: Option<ByteOffset>,
    ops_counts: BTreeMap<(ExtendedLayerId, u8), u8>,
    metadata: Vec<ActiveMetadata>,
    tu_index: u64,
}

impl OutputEffectState {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::new_for_xlayer(ExtendedLayerId::from_bits(0))
    }

    pub(crate) fn new_for_xlayer(active_xlayer: ExtendedLayerId) -> Self {
        Self {
            mfh: std::array::from_fn(|_| None),
            qm: std::array::from_fn(|_| None),
            qm_protected: 0,
            qm_seen_since_frame: 0,
            qm_obu_seen_since_frame: false,
            content_interpretation: [None; 8],
            ci_in_current_tu: [None; 8],
            active_xlayer,
            local_lcr_tu: None,
            local_lcr_in_cvs: false,
            local_lcr_ids: 0,
            global_lcr_xlayers: [None; 8],
            brt_count: BrtCount::None,
            second_brt_offset: None,
            ops_counts: BTreeMap::new(),
            metadata: Vec::new(),
            tu_index: 0,
        }
    }

    pub(crate) fn set_tu_index(&mut self, tu_index: u64) {
        self.tu_index = tu_index;
    }

    pub(super) fn local_lcr_ids(&self) -> u8 {
        self.local_lcr_ids
    }

    pub(crate) fn has_local_lcr_id(&self, id: u8) -> bool {
        (1..=7).contains(&id) && (self.local_lcr_ids & (1u8 << id)) != 0
    }

    pub(crate) fn apply_global_lcr(&mut self, id: u8, xlayer_map: u32) {
        if let Some(slot) = self.global_lcr_xlayers.get_mut(usize::from(id)) {
            *slot = Some(xlayer_map);
        }
    }

    pub(crate) fn apply_ops_record(&mut self, ops: &OperatingPointSet) {
        if ops.reset_flag {
            self.ops_counts
                .retain(|(xlayer, _), _| *xlayer != ops.xlayer_id);
        }
        self.apply_ops_entry(ops.xlayer_id, ops.ops_id, ops.ops_cnt);
    }

    pub(crate) fn apply_ops_entry(&mut self, xlayer: ExtendedLayerId, ops_id: u8, ops_cnt: u8) {
        if ops_cnt == 0 {
            self.ops_counts.remove(&(xlayer, ops_id));
        } else {
            self.ops_counts.insert((xlayer, ops_id), ops_cnt);
        }
    }

    pub(crate) fn begin_temporal_unit(&mut self) {
        self.tu_index = self.tu_index.saturating_add(1);
        self.qm_protected = 0;
        self.ci_in_current_tu = [None; 8];
    }

    pub(crate) fn observe_prefix_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        sequence: &SequenceHeader,
    ) -> Result<()> {
        match envelope.header.obu_type {
            ObuType::TemporalDelimiter => self.begin_temporal_unit(),
            ObuType::LayerConfigurationRecord => self.observe_lcr(envelope)?,
            ObuType::OperatingPointSet => self.observe_ops(envelope)?,
            ObuType::MultiFrameHeader => self.observe_mfh(envelope, sequence)?,
            ObuType::BufferRemovalTiming => self.observe_brt(envelope)?,
            ObuType::QuantizationMatrix => self.observe_qm(envelope, sequence)?,
            ObuType::ContentInterpretation => self.observe_ci(envelope, sequence)?,
            ObuType::MetadataShort | ObuType::MetadataGroup => {
                self.observe_metadata(envelope, false)?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn observe_suffix_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        match envelope.header.obu_type {
            ObuType::Padding => {}
            ObuType::MetadataShort | ObuType::MetadataGroup => {
                self.observe_metadata(envelope, true)?;
            }
            _ => {
                return Err(unsupported_feature_at(
                    "output_effect_suffix_order",
                    envelope.offset,
                    "coded-frame suffixes may contain only suffix metadata and padding",
                    "7.3.3",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn mfh_record(&self, id: MfhId) -> Option<&MultiFrameHeaderRecord> {
        usize::try_from(id.get())
            .ok()
            .and_then(|index| self.mfh.get(index))
            .and_then(Option::as_ref)
    }

    pub(crate) fn resolve_mfh_record(
        &self,
        envelope: ObuEnvelope<'_>,
        sequence: &SequenceHeader,
        mfh_id: MfhId,
    ) -> Result<&MultiFrameHeaderRecord> {
        let record = self.mfh_record(mfh_id).ok_or_else(|| {
            unsupported_feature_at(
                "multi_frame_header_unavailable",
                envelope.offset,
                "frame references a multi-frame header that is not available in-band",
                "7.3.8.7",
            )
        })?;
        if record.mfh_seq_header_id != sequence.general.seq_header_id {
            return Err(unsupported_feature_at(
                "multi_frame_header_sequence_mismatch",
                envelope.offset,
                "referenced multi-frame header resolves to a different sequence header",
                "7.3.8.7",
            ));
        }
        if !sequence
            .general
            .mlayer_dependency_map
            .depends_on(envelope.header.embedded_layer_id, record.mfh_mlayer_id)
            || !sequence.general.tlayer_dependency_map.depends_on(
                envelope.header.embedded_layer_id,
                envelope.header.temporal_layer_id,
                record.mfh_tlayer_id,
            )
        {
            return Err(unsupported_feature_at(
                "multi_frame_header_layer_dependency",
                envelope.offset,
                "referenced multi-frame header violates the active layer dependency maps",
                "7.3.8.7",
            ));
        }
        Ok(record)
    }

    pub(crate) fn prepare_frame(
        &mut self,
        envelope: ObuEnvelope<'_>,
        core: &FrameHeaderCore,
        sequence: &SequenceHeader,
        first_picture_in_tu: bool,
        frame_index: Option<usize>,
    ) -> Result<Option<FrameUserQmLevels>> {
        let (Some(immediate_output), Some(implicit_output)) =
            (core.immediate_output_frame, core.implicit_output_frame)
        else {
            return Err(DecodeHeaderStateError::MissingFrameOutputClassification.into());
        };
        if !immediate_output && !implicit_output && self.brt_count == BrtCount::Multiple {
            let offset = self
                .second_brt_offset
                .ok_or(DecodeHeaderStateError::MissingBufferRemovalTimingOffset)?;
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_unit_conformance(
                    offset,
                    frame_index,
                    "7.3.4",
                    "a coded non-output frame unit may contain at most one buffer removal timing OBU"
                        .to_owned(),
                ),
            });
        }
        let mlayer_idx = usize::from(envelope.header.embedded_layer_id.get());
        let starts_cvs = first_picture_in_tu
            && (sequence.general.single_picture_header_flag
                || matches!(
                    envelope.header.obu_type,
                    ObuType::ClosedLoopKey | ObuType::OpenLoopKey
                ));
        if starts_cvs {
            if envelope.header.obu_type == ObuType::ClosedLoopKey {
                self.local_lcr_in_cvs = self.local_lcr_tu == Some(self.tu_index);
            }
            self.validate_annex_a_lcr(envelope, sequence)?;
            self.reset_unprotected_qm();
            self.content_interpretation = self.ci_in_current_tu;
            self.metadata.retain(|unit| unit.tu_index == self.tu_index);
        } else if let Some(content) = self.ci_in_current_tu.get(mlayer_idx).copied().flatten() {
            if self
                .content_interpretation
                .get(mlayer_idx)
                .copied()
                .flatten()
                .is_some_and(|active| active != content)
            {
                return Err(unsupported_feature_at(
                    "content_interpretation_changed_in_cvs",
                    envelope.offset,
                    "content interpretation must remain identical within a coded video sequence",
                    "6.14",
                ));
            }
            if let Some(slot) = self.content_interpretation.get_mut(mlayer_idx) {
                *slot = Some(content);
            }
        }
        if !starts_cvs
            && ((envelope.header.obu_type == ObuType::Switch
                && core.restricted_prediction_switch == Some(true))
                || (envelope.header.obu_type == ObuType::RasFrame && core.reached_qm_reset))
        {
            self.reset_switch_qm(envelope, sequence);
        }
        self.validate_frame_qm(envelope, core, sequence)?;
        self.qm_seen_since_frame = 0;
        self.qm_obu_seen_since_frame = false;
        Ok(self.active_user_qms())
    }

    fn validate_annex_a_lcr(
        &self,
        envelope: ObuEnvelope<'_>,
        sequence: &SequenceHeader,
    ) -> Result<()> {
        if sequence.general.max_mlayer_id.get() == 0 {
            return Ok(());
        }
        let profile = sequence.general.seq_profile_idc.get();
        let has_global_lcr = sequence.general.seq_lcr_id.get() != 0
            && self
                .global_lcr_xlayers
                .get(usize::from(sequence.general.seq_lcr_id.get()))
                .copied()
                .flatten()
                .is_some_and(|map| {
                    (map & (1u32 << u32::from(envelope.header.extended_layer_id.get()))) != 0
                });
        let satisfied = match profile {
            1 | 3 | 4 => self.local_lcr_in_cvs,
            2 => self.local_lcr_in_cvs || has_global_lcr,
            _ => true,
        };
        if !satisfied {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    envelope.offset,
                    "A.2",
                    format!(
                        "Annex A Table A.4: profile {profile} with max_mlayer_id {} requires an active layer configuration record",
                        sequence.general.max_mlayer_id.get()
                    ),
                ),
            });
        }
        Ok(())
    }

    pub(crate) fn finish_frame(&mut self, mlayer: EmbeddedLayerId) -> FrameOutputEffects {
        let idx = usize::from(mlayer.get());
        let effects = FrameOutputEffects {
            content_interpretation: self.content_interpretation.get(idx).copied().flatten(),
            metadata: self
                .metadata
                .iter()
                .filter(|unit| unit.applies_to_layer(mlayer))
                .map(|unit| unit.unit.clone())
                .collect(),
        };
        self.metadata.retain(|unit| {
            unit.persistence != MetadataPersistence::No || !unit.applies_to_layer(mlayer)
        });
        self.brt_count = BrtCount::None;
        self.second_brt_offset = None;
        effects
    }

    fn observe_mfh(&mut self, envelope: ObuEnvelope<'_>, sequence: &SequenceHeader) -> Result<()> {
        if envelope.header.embedded_layer_id > sequence.general.max_mlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "OBU mlayer_id {} exceeds max_mlayer_id {}",
                        envelope.header.embedded_layer_id.get(),
                        sequence.general.max_mlayer_id.get()
                    ),
                ),
            });
        }
        if envelope.header.temporal_layer_id > sequence.general.max_tlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "OBU temporal_layer_id {} exceeds max_tlayer_id {}",
                        envelope.header.temporal_layer_id.get(),
                        sequence.general.max_tlayer_id.get()
                    ),
                ),
            });
        }
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let mfh = parse_multi_frame_header(&mut reader).map_err(|_| {
            unsupported_feature_at(
                "multi_frame_header_parse",
                envelope.offset,
                "decode requires a complete multi-frame header OBU",
                "5.7",
            )
        })?;
        finish(&mut reader, envelope, "multi_frame_header_tail", "5.7")?;
        if !mfh.mfh_id_in_range() || !mfh.seq_header_id_in_range() {
            return Err(unsupported_feature_at(
                "multi_frame_header_id_range",
                envelope.offset,
                "multi-frame and sequence-header ids must be in range",
                "6.7",
            ));
        }
        if mfh.mfh_seq_header_id != u32::from(sequence.general.seq_header_id.get()) {
            return Err(unsupported_feature_at(
                "multi_frame_header_sequence_unavailable",
                envelope.offset,
                "the referenced sequence header is not active in the selected decode layer",
                "7.3.8.6",
            ));
        }
        let id = usize::try_from(mfh.mfh_id()).map_err(|_| {
            unsupported_feature_at(
                "multi_frame_header_id_range",
                envelope.offset,
                "multi-frame header id does not fit the runtime slot index",
                "6.7",
            )
        })?;
        let seq_id =
            splot_core::headers::sequence::SequenceHeaderId::try_new(mfh.mfh_seq_header_id)
                .ok_or_else(|| {
                    unsupported_feature_at(
                        "multi_frame_header_sequence_id_range",
                        envelope.offset,
                        "multi-frame header sequence id is out of range",
                        "6.7",
                    )
                })?;
        self.mfh[id] = Some(MultiFrameHeaderRecord {
            mfh_id: MfhId::from_raw(id as u32),
            mfh_seq_header_id: seq_id,
            mfh_tlayer_id: envelope.header.temporal_layer_id,
            mfh_mlayer_id: envelope.header.embedded_layer_id,
            mfh_frame_size: mfh.mfh_frame_size,
            mfh_seg_info_present_flag: mfh.mfh_seg_info_present_flag,
            mfh_ext_seg_flag: mfh.mfh_ext_seg_flag,
            mfh_allow_seg_info_change: mfh.mfh_allow_seg_info_change,
            mfh_segment_info: mfh.segment_info,
            mfh_deblocking_filter_update: mfh.mfh_deblocking_filter_update,
            mfh_apply_deblocking_filter: mfh.mfh_apply_deblocking_filter,
            offset: envelope.offset,
        });
        Ok(())
    }

    fn observe_qm(&mut self, envelope: ObuEnvelope<'_>, sequence: &SequenceHeader) -> Result<()> {
        if envelope.header.embedded_layer_id > sequence.general.max_mlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "quantizer matrix mlayer_id {} exceeds max_mlayer_id {}",
                        envelope.header.embedded_layer_id.get(),
                        sequence.general.max_mlayer_id.get()
                    ),
                ),
            });
        }
        if envelope.header.temporal_layer_id > sequence.general.max_tlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "quantizer matrix temporal_layer_id {} exceeds max_tlayer_id {}",
                        envelope.header.temporal_layer_id.get(),
                        sequence.general.max_tlayer_id.get()
                    ),
                ),
            });
        }
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let qm = parse_quantizer_matrix(&mut reader).map_err(|_| {
            unsupported_feature_at(
                "quantizer_matrix_parse",
                envelope.offset,
                "decode requires a complete quantizer matrix OBU",
                "5.13",
            )
        })?;
        finish(&mut reader, envelope, "quantizer_matrix_tail", "5.13")?;
        if qm.is_reset() {
            if self.qm_obu_seen_since_frame {
                return Err(unsupported_feature_at(
                    "quantizer_matrix_duplicate_reset",
                    envelope.offset,
                    "a reset QM OBU must be first between coded frames",
                    "6.12",
                ));
            }
            self.qm = std::array::from_fn(|_| {
                Some(QmSlot {
                    mlayer_id: None,
                    tlayer_id: None,
                    num_planes: qm.num_planes,
                    user: None,
                })
            });
            self.qm_protected = (1 << NUM_CUSTOM_QMS) - 1;
            self.qm_obu_seen_since_frame = true;
            return Ok(());
        }
        let duplicate = self.qm_seen_since_frame & qm.qm_bit_map;
        if duplicate != 0 {
            return Err(unsupported_feature_at(
                "quantizer_matrix_duplicate_level",
                envelope.offset,
                "a quantizer matrix level was specified twice between coded frames",
                "6.12",
            ));
        }
        self.qm_obu_seen_since_frame = true;
        self.qm_seen_since_frame |= qm.qm_bit_map;
        for level in &qm.levels {
            let index = usize::from(level.level);
            self.qm[index] = Some(QmSlot {
                mlayer_id: Some(envelope.header.embedded_layer_id.get()),
                tlayer_id: Some(envelope.header.temporal_layer_id.get()),
                num_planes: qm.num_planes,
                user: build_user_qm_level(level),
            });
            self.qm_protected |= 1 << index;
        }
        Ok(())
    }

    fn observe_ci(&mut self, envelope: ObuEnvelope<'_>, sequence: &SequenceHeader) -> Result<()> {
        if envelope.header.embedded_layer_id > sequence.general.max_mlayer_id {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::frame_header_conformance(
                    envelope.offset,
                    None,
                    "6.2.2",
                    format!(
                        "content interpretation mlayer_id {} exceeds max_mlayer_id {}",
                        envelope.header.embedded_layer_id.get(),
                        sequence.general.max_mlayer_id.get()
                    ),
                ),
            });
        }
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let content = parse_content_interpretation(&mut reader).map_err(|_| {
            unsupported_feature_at(
                "content_interpretation_parse",
                envelope.offset,
                "decode requires a complete content interpretation OBU",
                "5.15",
            )
        })?;
        finish(&mut reader, envelope, "content_interpretation_tail", "5.15")?;
        let idx = usize::from(envelope.header.embedded_layer_id.get());
        if let Some(existing) = self.ci_in_current_tu.get(idx).copied().flatten()
            && existing != content
        {
            return Err(unsupported_feature_at(
                "content_interpretation_changed_in_temporal_unit",
                envelope.offset,
                "content interpretation must remain identical within an embedded layer and coded video sequence",
                "6.14",
            ));
        }
        if let Some(slot) = self.ci_in_current_tu.get_mut(idx) {
            *slot = Some(content);
        }
        Ok(())
    }

    fn observe_lcr(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        match parse_validated_lcr(envelope)? {
            LayerConfigurationRecord::Global(info) => {
                self.apply_global_lcr(info.global_config_record_id, info.xlayer_map);
            }
            LayerConfigurationRecord::Local(info) => {
                self.local_lcr_tu = Some(self.tu_index);
                self.local_lcr_in_cvs = true;
                if (1..=7).contains(&info.local_id) {
                    self.local_lcr_ids |= 1u8 << info.local_id;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn observe_ops(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let ops = parse_ops_obu(envelope)?;
        self.apply_ops_record(&ops);
        Ok(())
    }

    fn observe_brt(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let brt = parse_buffer_removal_timing(&mut reader)
            .map_err(|error| malformed_obu_payload(envelope, "5.12", error))?;
        finish_obu_payload(
            &mut reader,
            envelope.payload,
            envelope.header.obu_type.is_extensible_obu(),
        )
        .map_err(|error| malformed_obu_payload(envelope, "5.12", error))?;
        if let Some((ops_id, ops_count)) = brt.ops_reference() {
            let available = self
                .ops_counts
                .get(&(envelope.header.extended_layer_id, ops_id))
                .or_else(|| self.ops_counts.get(&(GLOBAL_XLAYER_ID, ops_id)));
            if available.copied() != Some(ops_count) {
                return Err(unsupported_feature_at(
                    "buffer_removal_timing_ops_unavailable",
                    envelope.offset,
                    "BRT operating point id/count does not match an available OPS",
                    "6.11",
                ));
            }
        }
        match self.brt_count {
            BrtCount::None => self.brt_count = BrtCount::One,
            BrtCount::One => {
                self.brt_count = BrtCount::Multiple;
                self.second_brt_offset = Some(envelope.offset);
            }
            BrtCount::Multiple => {}
        }
        Ok(())
    }

    fn observe_metadata(&mut self, envelope: ObuEnvelope<'_>, expected_suffix: bool) -> Result<()> {
        let xlayer = envelope.header.extended_layer_id;
        parse_metadata_obu(envelope, expected_suffix, |action| {
            self.apply_metadata_action(xlayer, action);
        })
    }

    pub(crate) fn apply_metadata_action(
        &mut self,
        source_xlayer: ExtendedLayerId,
        action: MetadataAction,
    ) {
        match action {
            MetadataAction::Cancel(metadata_type) => {
                self.cancel_metadata(source_xlayer, metadata_type);
            }
            MetadataAction::Store {
                layer_idc,
                persistence_idc,
                source_mlayer,
                xlayer_map,
                mlayer_maps,
                unit,
            } => {
                let mlayer_map = if source_xlayer.is_global() {
                    let Some(map) = xlayer_map else {
                        self.store_metadata_entry(
                            layer_idc,
                            persistence_idc,
                            source_mlayer,
                            mlayer_maps.first().copied(),
                            unit,
                        );
                        return;
                    };
                    let mask = 1u32 << u32::from(self.active_xlayer.get());
                    if map & mask == 0 {
                        return;
                    }
                    let slot = (map & (mask - 1)).count_ones() as usize;
                    mlayer_maps.get(slot).copied()
                } else {
                    mlayer_maps.first().copied()
                };
                self.store_metadata_entry(
                    layer_idc,
                    persistence_idc,
                    source_mlayer,
                    mlayer_map,
                    unit,
                );
            }
        }
    }

    fn store_metadata_entry(
        &mut self,
        layer_idc: u8,
        persistence_idc: u8,
        source_mlayer: EmbeddedLayerId,
        mlayer_map: Option<u8>,
        unit: MetadataUnit,
    ) {
        let active = ActiveMetadata {
            unit,
            persistence: MetadataPersistence::from_idc(persistence_idc),
            layer_idc,
            source_mlayer,
            mlayer_map,
            tu_index: self.tu_index,
        };
        match active.persistence {
            MetadataPersistence::Global => self.metadata.retain(|existing| {
                existing.unit.metadata_type != active.unit.metadata_type
                    || existing.persistence != MetadataPersistence::Global
            }),
            MetadataPersistence::Basic | MetadataPersistence::Enhanced => {
                self.metadata.retain(|existing| {
                    existing.unit.metadata_type != active.unit.metadata_type
                        || existing.persistence == MetadataPersistence::Global
                        || !existing.same_scope(&active)
                });
            }
            MetadataPersistence::No | MetadataPersistence::Reserved => {}
        }
        self.metadata.push(active);
    }

    #[cfg(test)]
    fn store_metadata(
        &mut self,
        envelope: ObuEnvelope<'_>,
        layer_idc: u8,
        persistence_idc: u8,
        mlayer_map: Option<u8>,
        unit: MetadataUnit,
    ) {
        self.store_metadata_entry(
            layer_idc,
            persistence_idc,
            envelope.header.embedded_layer_id,
            mlayer_map,
            unit,
        );
    }

    pub(crate) fn cancel_metadata(&mut self, xlayer: ExtendedLayerId, metadata_type: MetadataType) {
        if xlayer.is_global() || xlayer == self.active_xlayer {
            self.metadata.retain(|existing| {
                existing.unit.metadata_type != metadata_type
                    || existing.persistence == MetadataPersistence::Global
            });
        }
    }

    fn reset_unprotected_qm(&mut self) {
        for index in 0..NUM_CUSTOM_QMS {
            if self.qm_protected & (1 << index) == 0 {
                self.qm[index] = None;
            }
        }
    }

    fn reset_switch_qm(&mut self, envelope: ObuEnvelope<'_>, sequence: &SequenceHeader) {
        let presence = sequence.general.mlayer_dependency_map.presence_map();
        for index in 0..NUM_CUSTOM_QMS {
            if self.qm_protected & (1 << index) != 0 {
                continue;
            }
            let reset = self.qm[index].as_ref().is_none_or(|slot| {
                slot.mlayer_id.is_none_or(|mlayer| {
                    presence.is_present(
                        EmbeddedLayerId::from_bits(mlayer),
                        envelope.header.embedded_layer_id,
                    )
                })
            });
            if reset {
                self.qm[index] = None;
            }
        }
    }

    fn validate_frame_qm(
        &self,
        envelope: ObuEnvelope<'_>,
        core: &FrameHeaderCore,
        sequence: &SequenceHeader,
    ) -> Result<()> {
        let Some(setup) = core.setup_qm_params.filter(|setup| setup.using_qmatrix) else {
            return Ok(());
        };
        let num_planes = if sequence.general.chroma_format_idc.is_monochrome() {
            1
        } else {
            3
        };
        let count = usize::from(setup.pic_qm_num_minus_1) + 1;
        for levels in setup.levels.iter().take(count) {
            let referenced = [levels.qm_y, levels.qm_u, levels.qm_v];
            for level in referenced.into_iter().take(num_planes) {
                if usize::from(level) >= NUM_CUSTOM_QMS {
                    continue;
                }
                let Some(slot) = self.qm[usize::from(level)].as_ref() else {
                    continue;
                };
                if usize::from(slot.num_planes) != num_planes {
                    return Err(unsupported_feature_at(
                        "quantizer_matrix_plane_count",
                        envelope.offset,
                        "referenced QM plane count differs from the active sequence",
                        "6.17.6.2",
                    ));
                }
                if let Some(mlayer) = slot.mlayer_id
                    && !sequence.general.mlayer_dependency_map.depends_on(
                        envelope.header.embedded_layer_id,
                        EmbeddedLayerId::from_bits(mlayer),
                    )
                {
                    return Err(unsupported_feature_at(
                        "quantizer_matrix_mlayer_dependency",
                        envelope.offset,
                        "referenced QM level violates MLayerDependencyMap",
                        "6.17.6.2",
                    ));
                }
                if let (Some(_mlayer), Some(tlayer)) = (slot.mlayer_id, slot.tlayer_id)
                    && !sequence.general.tlayer_dependency_map.depends_on(
                        envelope.header.embedded_layer_id,
                        envelope.header.temporal_layer_id,
                        splot_core::types::TemporalLayerId::from_bits(tlayer),
                    )
                {
                    return Err(unsupported_feature_at(
                        "quantizer_matrix_tlayer_dependency",
                        envelope.offset,
                        "referenced QM level violates TLayerDependencyMap",
                        "6.17.6.2",
                    ));
                }
            }
        }
        Ok(())
    }

    fn active_user_qms(&self) -> Option<FrameUserQmLevels> {
        let levels =
            std::array::from_fn(|index| self.qm[index].as_ref().and_then(|slot| slot.user.clone()));
        levels.iter().any(Option::is_some).then(|| Arc::new(levels))
    }
}

fn build_user_qm_level(level: &QuantizerMatrixLevel) -> Option<FrameUserQmLevel> {
    let matrices = level.matrices.as_ref()?;
    let mut transforms: [[Option<QmUserPlane>; 3]; 3] =
        std::array::from_fn(|_| std::array::from_fn(|_| None));
    for matrix in matrices {
        let transform = match matrix.transform {
            FundamentalQmTransform::Tx8x8 => 0,
            FundamentalQmTransform::Tx8x4 => 1,
            FundamentalQmTransform::Tx4x8 => 2,
        };
        for (plane, values) in matrix.planes.iter().enumerate() {
            if let Some(target) = transforms[transform].get_mut(plane) {
                *target = Some(QmUserPlane {
                    width: usize::from(values.width),
                    height: usize::from(values.height),
                    values: Arc::from(values.values.clone()),
                });
            }
        }
    }
    Some(FrameUserQmLevel { transforms })
}

pub(crate) fn parse_validated_lcr(envelope: ObuEnvelope<'_>) -> Result<LayerConfigurationRecord> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let record = parse_layer_config_record(&mut reader, envelope.header.extended_layer_id)
        .map_err(|_| {
            unsupported_feature_at(
                "layer_config_record_parse",
                envelope.offset,
                "decode requires a complete layer configuration record OBU",
                "5.8",
            )
        })?;
    finish(&mut reader, envelope, "layer_config_record_tail", "5.8")?;
    if let LayerConfigurationRecord::Global(ref info) = record {
        if info.global_config_record_id == 0 || info.global_config_record_id > 7 {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    envelope.offset,
                    "6.8.2",
                    format!(
                        "global LCR lcr_global_config_record_id {} must be in 1..=7",
                        info.global_config_record_id
                    ),
                ),
            });
        }
        if info.xlayer_map == 0 || (info.xlayer_map & (1u32 << 31)) != 0 {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    envelope.offset,
                    "6.8.2",
                    "global LCR lcr_xlayer_map must be in 1..=(1 << 31) - 1".to_owned(),
                ),
            });
        }
    }
    Ok(record)
}

pub(crate) fn parse_ops_obu(envelope: ObuEnvelope<'_>) -> Result<OperatingPointSet> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let ops = parse_operating_point_set(&mut reader, envelope.header.extended_layer_id).map_err(
        |_| {
            unsupported_feature_at(
                "operating_point_set_parse",
                envelope.offset,
                "BRT resolution requires a complete operating point set OBU",
                "5.10",
            )
        },
    )?;
    finish(&mut reader, envelope, "operating_point_set_tail", "5.10")?;
    Ok(ops)
}

pub(crate) fn parse_metadata_obu(
    envelope: ObuEnvelope<'_>,
    expected_suffix: bool,
    mut on_action: impl FnMut(MetadataAction),
) -> Result<()> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let source_mlayer = envelope.header.embedded_layer_id;
    match envelope.header.obu_type {
        ObuType::MetadataShort => {
            let short =
                parse_metadata_short(&mut reader, envelope.payload.len()).map_err(|_| {
                    unsupported_feature_at(
                        "metadata_short_parse",
                        envelope.offset,
                        "decode requires a complete metadata short OBU",
                        "5.17.2",
                    )
                })?;
            if short.metadata_is_suffix != expected_suffix {
                return Err(metadata_order_error(envelope, expected_suffix));
            }
            finish(&mut reader, envelope, "metadata_short_tail", "5.17.2")?;
            if short.muh_cancel_flag {
                on_action(MetadataAction::Cancel(short.metadata_type));
            } else if let Some(unit) = short.unit {
                on_action(MetadataAction::Store {
                    layer_idc: short.muh_layer_idc,
                    persistence_idc: short.muh_persistence_idc,
                    source_mlayer,
                    xlayer_map: None,
                    mlayer_maps: Vec::new(),
                    unit,
                });
            }
        }
        ObuType::MetadataGroup => {
            let group = parse_metadata_group(&mut reader, envelope.header.extended_layer_id)
                .map_err(|_| {
                    unsupported_feature_at(
                        "metadata_group_parse",
                        envelope.offset,
                        "decode requires a complete metadata group OBU",
                        "5.17.3",
                    )
                })?;
            if group.metadata_is_suffix != expected_suffix {
                return Err(metadata_order_error(envelope, expected_suffix));
            }
            finish(&mut reader, envelope, "metadata_group_tail", "5.17.3")?;
            for group_unit in group.units {
                if group_unit.muh_cancel_flag {
                    on_action(MetadataAction::Cancel(group_unit.metadata_type));
                } else if let (Some(layer_idc), Some(persistence_idc), Some(unit)) = (
                    group_unit.muh_layer_idc,
                    group_unit.muh_persistence_idc,
                    group_unit.unit,
                ) {
                    on_action(MetadataAction::Store {
                        layer_idc,
                        persistence_idc,
                        source_mlayer,
                        xlayer_map: group_unit.muh_xlayer_map,
                        mlayer_maps: group_unit.muh_mlayer_maps,
                        unit,
                    });
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish(
    reader: &mut BitReader<'_>,
    envelope: ObuEnvelope<'_>,
    reason: &'static str,
    spec: &'static str,
) -> Result<()> {
    finish_obu_payload(
        reader,
        envelope.payload,
        envelope.header.obu_type.is_extensible_obu(),
    )
    .map_err(|_| {
        unsupported_feature_at(
            reason,
            envelope.offset,
            "output-effect OBU has a malformed payload tail",
            spec,
        )
    })
}

fn malformed_obu_payload(
    envelope: ObuEnvelope<'_>,
    spec: &'static str,
    error: impl core::fmt::Display,
) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::obu_payload_parse(envelope.offset, spec, error.to_string()),
    }
}

fn metadata_order_error(envelope: ObuEnvelope<'_>, expected_suffix: bool) -> crate::DecodeError {
    unsupported_feature_at(
        "metadata_prefix_suffix_order",
        envelope.offset,
        if expected_suffix {
            "metadata following frame data must signal metadata_is_suffix equal to 1"
        } else {
            "metadata preceding frame data must signal metadata_is_suffix equal to 0"
        },
        "7.3.3",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::headers::buffer_removal_timing::BufferRemovalTiming;
    use splot_core::headers::metadata::{MetadataHdrCll, MetadataPayload};
    use splot_core::obu::ObuHeader;
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};
    use splot_core::types::TemporalLayerId;
    use splot_core::write::{BitWriter, write_buffer_removal_timing};

    const STANDALONE_OLK_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-standalone-olk-64x64.ivf");

    fn content(scan_type: u8) -> splot_core::Result<ContentInterpretation> {
        let payload = [scan_type << 6];
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        parse_content_interpretation(&mut reader)
    }

    fn metadata_envelope(xlayer: u8, mlayer: u8) -> ObuEnvelope<'static> {
        ObuEnvelope {
            offset: ByteOffset::new(0),
            size: 2,
            header: ObuHeader {
                has_header_extension: true,
                obu_type: ObuType::MetadataShort,
                temporal_layer_id: TemporalLayerId::from_bits(0),
                embedded_layer_id: EmbeddedLayerId::from_bits(mlayer),
                extended_layer_id: ExtendedLayerId::from_bits(xlayer),
                header_size_bytes: 2,
            },
            payload: &[],
        }
    }

    fn brt_payload(br_time: u32) -> Option<Vec<u8>> {
        let mut writer = BitWriter::new();
        write_buffer_removal_timing(&mut writer, &BufferRemovalTiming::ExtendedLayer { br_time })
            .ok()?;
        let remainder = writer.bit_len() % 8;
        let tail_bits = if remainder == 0 { 8 } else { 8 - remainder };
        writer.write_trailing_bits(tail_bits).ok()?;
        Some(writer.into_bytes())
    }

    fn brt_envelope(payload: &[u8], offset: u64) -> ObuEnvelope<'_> {
        ObuEnvelope {
            offset: ByteOffset::new(offset),
            size: u32::try_from(payload.len().saturating_add(1)).unwrap_or(u32::MAX),
            header: ObuHeader {
                has_header_extension: false,
                obu_type: ObuType::BufferRemovalTiming,
                temporal_layer_id: TemporalLayerId::from_bits(0),
                embedded_layer_id: EmbeddedLayerId::from_bits(0),
                extended_layer_id: ExtendedLayerId::from_bits(0),
                header_size_bytes: 1,
            },
            payload,
        }
    }

    fn olk_frame() -> Option<(ObuEnvelope<'static>, SequenceHeader, FrameHeaderCore)> {
        let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(STANDALONE_OLK_FIXTURE) else {
            return None;
        };
        let sequence_envelope = ivf
            .frames
            .iter()
            .flat_map(|frame| &frame.obus)
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)?;
        let frame_envelope = ivf
            .frames
            .iter()
            .flat_map(|frame| &frame.obus)
            .find(|obu| obu.header.obu_type == ObuType::OpenLoopKey)?;
        let sequence = crate::pipeline::parse_sequence(*sequence_envelope).ok()?;
        let core = crate::pipeline::parse_frame_core(*frame_envelope, &sequence).ok()?;
        Some((*frame_envelope, sequence, core))
    }

    fn observe_brt_time(state: &mut OutputEffectState, payload: &[u8], offset: u64) -> Result<()> {
        state.observe_brt(brt_envelope(payload, offset))
    }

    fn hdr_cll(max_cll: u16) -> MetadataUnit {
        MetadataUnit {
            metadata_type: MetadataType::HdrCll,
            payload_size: 4,
            payload: MetadataPayload::HdrCll(MetadataHdrCll {
                max_cll,
                max_fall: 0,
            }),
        }
    }

    fn max_cll(unit: &ActiveMetadata) -> Option<u16> {
        match &unit.unit.payload {
            MetadataPayload::HdrCll(cll) => Some(cll.max_cll),
            _ => None,
        }
    }

    #[test]
    fn multiple_brts_follow_output_classification_and_reset_after_frame() {
        let Some((frame_envelope, sequence, base_core)) = olk_frame() else {
            return;
        };
        let (Some(first), Some(second)) = (brt_payload(0), brt_payload(1)) else {
            return;
        };

        let mut immediate = OutputEffectState::new();
        assert!(observe_brt_time(&mut immediate, &first, 10).is_ok());
        assert!(observe_brt_time(&mut immediate, &second, 20).is_ok());
        let mut immediate_core = base_core.clone();
        immediate_core.immediate_output_frame = Some(true);
        immediate_core.implicit_output_frame = Some(false);
        assert!(
            immediate
                .prepare_frame(frame_envelope, &immediate_core, &sequence, false, Some(4),)
                .is_ok()
        );
        assert_eq!(immediate.brt_count, BrtCount::Multiple);
        let _ = immediate.finish_frame(EmbeddedLayerId::BASE);
        assert_eq!(immediate.brt_count, BrtCount::None);
        assert_eq!(immediate.second_brt_offset, None);

        let mut implicit = OutputEffectState::new();
        assert!(observe_brt_time(&mut implicit, &first, 10).is_ok());
        assert!(observe_brt_time(&mut implicit, &second, 20).is_ok());
        let mut implicit_core = base_core.clone();
        implicit_core.immediate_output_frame = Some(false);
        implicit_core.implicit_output_frame = Some(true);
        assert!(
            implicit
                .prepare_frame(frame_envelope, &implicit_core, &sequence, false, Some(5),)
                .is_ok()
        );
        assert_eq!(implicit.brt_count, BrtCount::Multiple);
        let _ = implicit.finish_frame(EmbeddedLayerId::BASE);
        assert_eq!(implicit.brt_count, BrtCount::None);

        let mut non_output = OutputEffectState::new();
        assert!(observe_brt_time(&mut non_output, &first, 10).is_ok());
        let mut non_output_core = base_core;
        non_output_core.immediate_output_frame = Some(false);
        non_output_core.implicit_output_frame = Some(false);
        assert!(
            non_output
                .prepare_frame(frame_envelope, &non_output_core, &sequence, false, Some(6),)
                .is_ok()
        );
        assert_eq!(non_output.brt_count, BrtCount::One);
        let _ = non_output.finish_frame(EmbeddedLayerId::BASE);
        assert_eq!(non_output.brt_count, BrtCount::None);
    }

    #[test]
    fn non_output_duplicate_brt_is_typed_at_second_offset_before_state_effects() {
        let Some((frame_envelope, sequence, mut core)) = olk_frame() else {
            return;
        };
        let (Some(first), Some(second)) = (brt_payload(0), brt_payload(1)) else {
            return;
        };
        let mut state = OutputEffectState::new();
        assert!(observe_brt_time(&mut state, &first, 10).is_ok());
        assert!(observe_brt_time(&mut state, &second, 20).is_ok());
        state.store_metadata(metadata_envelope(0, 0), LAYER_CURRENT, 1, None, hdr_cll(10));
        core.immediate_output_frame = Some(false);
        core.implicit_output_frame = Some(false);

        let result = state.prepare_frame(frame_envelope, &core, &sequence, true, Some(7));

        assert!(matches!(
            &result,
            Err(DecodeError::MalformedSource { issue })
                if issue.kind() == crate::DecodeSourceIssueKind::FrameUnitConformanceError
                    && issue.spec_section() == Some("7.3.4")
                    && issue.offset() == Some(ByteOffset::new(20))
                    && issue.frame_index() == Some(7)
        ));
        let report = result
            .as_ref()
            .err()
            .and_then(crate::DecodeDiagnosticReport::from_decode_error);
        assert!(matches!(
            report.map(|report| report.details),
            Some(crate::DecodeDiagnosticDetails::MalformedSource(_))
        ));
        assert_eq!(state.metadata.len(), 1);
        assert_eq!(state.brt_count, BrtCount::Multiple);
        assert_eq!(state.second_brt_offset, Some(ByteOffset::new(20)));
    }

    #[test]
    fn missing_output_classification_is_typed_header_state() {
        let Some((frame_envelope, sequence, mut core)) = olk_frame() else {
            return;
        };
        core.implicit_output_frame = None;
        let result = OutputEffectState::new().prepare_frame(
            frame_envelope,
            &core,
            &sequence,
            false,
            Some(8),
        );
        assert!(matches!(
            result,
            Err(DecodeError::HeaderState {
                source: DecodeHeaderStateError::MissingFrameOutputClassification,
            })
        ));
    }

    #[test]
    fn malformed_brt_payloads_are_typed_and_fail_closed() {
        let Some(valid) = brt_payload(0) else {
            return;
        };
        let mut eof_state = OutputEffectState::new();
        let eof = observe_brt_time(&mut eof_state, &valid[..0], 30);
        assert!(matches!(
            eof,
            Err(DecodeError::MalformedSource { issue })
                if issue.kind() == crate::DecodeSourceIssueKind::ObuPayloadParseError
                    && issue.spec_section() == Some("5.12")
                    && issue.offset() == Some(ByteOffset::new(30))
        ));
        assert_eq!(eof_state.brt_count, BrtCount::None);

        let mut invalid_tail = valid;
        invalid_tail.push(0x80);
        let mut tail_state = OutputEffectState::new();
        let tail = observe_brt_time(&mut tail_state, &invalid_tail, 40);
        assert!(matches!(
            tail,
            Err(DecodeError::MalformedSource { issue })
                if issue.kind() == crate::DecodeSourceIssueKind::ObuPayloadParseError
                    && issue.spec_section() == Some("5.12")
                    && issue.offset() == Some(ByteOffset::new(40))
        ));
        assert_eq!(tail_state.brt_count, BrtCount::None);
    }

    #[test]
    fn global_metadata_overwrites_global_and_survives_cancel() {
        let mut state = OutputEffectState::new();
        let envelope = metadata_envelope(0, 0);
        state.store_metadata(envelope, LAYER_GLOBAL, 0, None, hdr_cll(10));
        state.store_metadata(envelope, LAYER_CURRENT, 1, None, hdr_cll(20));
        state.store_metadata(envelope, LAYER_GLOBAL, 0, None, hdr_cll(30));

        assert_eq!(state.metadata.len(), 2);
        assert_eq!(
            state
                .metadata
                .iter()
                .filter(|unit| unit.persistence == MetadataPersistence::Global)
                .filter_map(max_cll)
                .collect::<Vec<_>>(),
            vec![30]
        );

        state.cancel_metadata(ExtendedLayerId::from_bits(0), MetadataType::HdrCll);

        assert_eq!(state.metadata.len(), 1);
        assert_eq!(state.metadata[0].persistence, MetadataPersistence::Global);
        assert_eq!(max_cll(&state.metadata[0]), Some(30));
    }

    #[test]
    fn basic_and_enhanced_metadata_replace_same_scope_and_cancel() {
        let mut state = OutputEffectState::new();
        let base = metadata_envelope(0, 0);
        let upper = metadata_envelope(0, 1);
        state.store_metadata(base, LAYER_CURRENT, 1, None, hdr_cll(10));
        state.store_metadata(base, LAYER_CURRENT, 1, None, hdr_cll(20));
        assert_eq!(state.metadata.len(), 1);
        assert_eq!(max_cll(&state.metadata[0]), Some(20));

        state.store_metadata(base, LAYER_CURRENT, 3, None, hdr_cll(30));
        assert_eq!(state.metadata.len(), 1);
        assert_eq!(state.metadata[0].persistence, MetadataPersistence::Enhanced);
        assert_eq!(max_cll(&state.metadata[0]), Some(30));

        state.store_metadata(base, LAYER_CURRENT, 3, None, hdr_cll(40));
        assert_eq!(state.metadata.len(), 1);
        assert_eq!(max_cll(&state.metadata[0]), Some(40));

        state.store_metadata(upper, LAYER_CURRENT, 3, None, hdr_cll(50));
        assert_eq!(state.metadata.len(), 2);

        state.cancel_metadata(ExtendedLayerId::from_bits(0), MetadataType::HdrCll);
        assert!(state.metadata.is_empty());
    }

    #[test]
    fn metadata_snapshot_applies_only_to_selected_base_layer() {
        let mut state = OutputEffectState::new();
        state.store_metadata(metadata_envelope(0, 1), LAYER_GLOBAL, 2, None, hdr_cll(10));
        state.store_metadata(metadata_envelope(0, 0), LAYER_CURRENT, 2, None, hdr_cll(20));
        state.store_metadata(metadata_envelope(0, 1), LAYER_CURRENT, 2, None, hdr_cll(30));
        state.store_metadata(
            metadata_envelope(0, 1),
            LAYER_VALUES,
            2,
            Some(0b0000_0001),
            hdr_cll(40),
        );
        state.store_metadata(
            metadata_envelope(0, 1),
            LAYER_VALUES,
            2,
            Some(0b0000_0010),
            hdr_cll(50),
        );

        let effects = state.finish_frame(EmbeddedLayerId::BASE);
        assert_eq!(
            effects
                .metadata
                .iter()
                .filter_map(|unit| match &unit.payload {
                    MetadataPayload::HdrCll(cll) => Some(cll.max_cll),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![10, 20, 40]
        );
    }

    #[test]
    fn no_persistence_metadata_expires_after_frame_snapshot() {
        let mut state = OutputEffectState::new();
        state.store_metadata(metadata_envelope(0, 0), LAYER_CURRENT, 2, None, hdr_cll(10));
        assert_eq!(state.finish_frame(EmbeddedLayerId::BASE).metadata.len(), 1);
        assert!(
            state
                .finish_frame(EmbeddedLayerId::BASE)
                .metadata
                .is_empty()
        );
    }

    #[test]
    fn coded_video_sequence_start_prunes_metadata_from_earlier_temporal_units() {
        let fixture = olk_frame();
        assert!(
            fixture.is_some(),
            "the OLK fixture must resolve sequence and frame headers"
        );
        let Some((olk_envelope, sequence, core)) = fixture else {
            return;
        };
        let mut state = OutputEffectState::new();
        state.store_metadata(metadata_envelope(0, 0), LAYER_GLOBAL, 1, None, hdr_cll(10));
        state.begin_temporal_unit();
        state.store_metadata(metadata_envelope(0, 0), LAYER_CURRENT, 1, None, hdr_cll(20));

        assert!(
            state
                .prepare_frame(olk_envelope, &core, &sequence, true, None)
                .is_ok()
        );
        assert_eq!(state.metadata.len(), 1);
        assert_eq!(max_cll(&state.metadata[0]), Some(20));
    }

    #[test]
    fn open_loop_key_not_first_in_tu_preserves_cvs_consistency_checks() {
        let fixture = olk_frame();
        assert!(
            fixture.is_some(),
            "the OLK fixture must resolve sequence and frame headers"
        );
        let Some((olk_envelope, sequence, core)) = fixture else {
            return;
        };
        let active = content(0);
        let current = content(1);
        assert!(active.is_ok() && current.is_ok());
        let (Ok(active), Ok(current)) = (active, current) else {
            return;
        };
        let mut state = OutputEffectState::new();
        state.content_interpretation[0] = Some(active);
        state.ci_in_current_tu[0] = Some(current);

        let result = state.prepare_frame(olk_envelope, &core, &sequence, false, None);

        assert!(matches!(
            result,
            Err(crate::DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == "content_interpretation_changed_in_cvs"
        ));
    }

    fn sparse_tlayer_sequence() -> SequenceHeader {
        const SPARSE_OBU: &[u8] = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-tlayer4-sparse-dependencies.obu"
        );
        let obus = splot_core::annexb::parse_annex_b_obus(SPARSE_OBU).unwrap();
        let seq_obu = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
            .unwrap();
        let mut reader = BitReader::new(seq_obu.payload, seq_obu.payload_offset());
        splot_core::headers::sequence::parse_sequence_header(&mut reader).unwrap()
    }

    #[test]
    fn resolve_mfh_record_rejects_disallowed_temporal_layer_dependency() {
        let Some((mut frame_envelope, _base_seq, _core)) = olk_frame() else {
            return;
        };
        let sequence = sparse_tlayer_sequence();
        let mfh_id = MfhId::from_raw(1);
        let mut state = OutputEffectState::new();
        state.mfh[1] = Some(MultiFrameHeaderRecord {
            mfh_id,
            mfh_seq_header_id: sequence.general.seq_header_id,
            mfh_tlayer_id: splot_core::types::TemporalLayerId::from_bits(1),
            mfh_mlayer_id: EmbeddedLayerId::from_bits(0),
            mfh_frame_size: None,
            mfh_seg_info_present_flag: false,
            mfh_ext_seg_flag: None,
            mfh_allow_seg_info_change: None,
            mfh_segment_info: None,
            mfh_deblocking_filter_update: false,
            mfh_apply_deblocking_filter: [false; 4],
            offset: ByteOffset::new(0),
        });

        frame_envelope.header.temporal_layer_id = splot_core::types::TemporalLayerId::from_bits(1);
        assert!(
            state
                .resolve_mfh_record(frame_envelope, &sequence, mfh_id)
                .is_ok()
        );

        frame_envelope.header.temporal_layer_id = splot_core::types::TemporalLayerId::from_bits(2);
        let result = state.resolve_mfh_record(frame_envelope, &sequence, mfh_id);
        assert!(matches!(
            result,
            Err(crate::DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == "multi_frame_header_layer_dependency"
        ));
    }

    #[test]
    fn validate_frame_qm_rejects_disallowed_temporal_layer_dependency() {
        let Some((mut frame_envelope, _base_seq, mut core)) = olk_frame() else {
            return;
        };
        let sequence = sparse_tlayer_sequence();
        let mut state = OutputEffectState::new();
        state.qm[2] = Some(QmSlot {
            mlayer_id: Some(0),
            tlayer_id: Some(1),
            num_planes: 3,
            user: None,
        });
        let setup = core
            .setup_qm_params
            .as_mut()
            .expect("key frame core carries setup_qm_params");
        setup.using_qmatrix = true;
        setup.pic_qm_num_minus_1 = 0;
        setup.levels[0].qm_y = 2;
        setup.levels[0].qm_u = 2;
        setup.levels[0].qm_v = 2;

        frame_envelope.header.temporal_layer_id = splot_core::types::TemporalLayerId::from_bits(1);
        assert!(
            state
                .validate_frame_qm(frame_envelope, &core, &sequence)
                .is_ok()
        );

        frame_envelope.header.temporal_layer_id = splot_core::types::TemporalLayerId::from_bits(2);
        let result = state.validate_frame_qm(frame_envelope, &core, &sequence);
        assert!(matches!(
            result,
            Err(crate::DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == "quantizer_matrix_tlayer_dependency"
        ));
    }

    #[test]
    fn metadata_snapshot_scopes_to_mlayer_and_retains_other_layer_nonpersistent_units() {
        let mut state = OutputEffectState::new();
        let ml0 = metadata_envelope(0, 0);
        let ml1 = metadata_envelope(0, 1);
        state.store_metadata(ml0, LAYER_GLOBAL, 0, None, hdr_cll(10));
        state.store_metadata(ml0, LAYER_CURRENT, 1, None, hdr_cll(20));
        state.store_metadata(ml1, LAYER_CURRENT, 2, None, hdr_cll(30));
        state.store_metadata(ml0, LAYER_VALUES, 1, Some(0b0000_0001), hdr_cll(40));
        state.store_metadata(ml1, LAYER_VALUES, 2, Some(0b0000_0010), hdr_cll(50));

        let collect_clls = |effects: FrameOutputEffects| -> Vec<u16> {
            effects
                .metadata
                .iter()
                .filter_map(|unit| match &unit.payload {
                    MetadataPayload::HdrCll(cll) => Some(cll.max_cll),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(
            collect_clls(state.finish_frame(EmbeddedLayerId::BASE)),
            vec![10, 20, 40]
        );
        assert_eq!(
            collect_clls(state.finish_frame(EmbeddedLayerId::from_bits(1))),
            vec![10, 30, 50]
        );
        assert_eq!(
            collect_clls(state.finish_frame(EmbeddedLayerId::from_bits(1))),
            vec![10]
        );
    }

    #[test]
    fn content_interpretation_is_scoped_per_embedded_layer() {
        let Some((olk_envelope, sequence, core)) = olk_frame() else {
            return;
        };
        let (Ok(ci0), Ok(ci1)) = (content(0), content(1)) else {
            panic!("test CI payloads must parse");
        };

        let mut state = OutputEffectState::new();
        state.content_interpretation[0] = Some(ci0);
        state.content_interpretation[1] = Some(ci1);
        state.ci_in_current_tu[1] = Some(ci1);

        let mut ml1_env = olk_envelope;
        ml1_env.header.embedded_layer_id = EmbeddedLayerId::from_bits(1);
        assert!(
            state
                .prepare_frame(ml1_env, &core, &sequence, false, None)
                .is_ok()
        );
        assert_eq!(
            state
                .finish_frame(EmbeddedLayerId::BASE)
                .content_interpretation,
            Some(ci0)
        );
        assert_eq!(
            state
                .finish_frame(EmbeddedLayerId::from_bits(1))
                .content_interpretation,
            Some(ci1)
        );

        state.begin_temporal_unit();
        state.ci_in_current_tu[1] = Some(ci0);
        let err = state.prepare_frame(ml1_env, &core, &sequence, false, None);
        assert!(matches!(
            err,
            Err(crate::DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == "content_interpretation_changed_in_cvs"
        ));
    }

    #[test]
    fn reset_switch_qm_respects_mlayer_presence_map_polarity() {
        let Some((mut frame_envelope, mut sequence, _core)) = olk_frame() else {
            return;
        };
        sequence.general.max_mlayer_id = EmbeddedLayerId::from_bits(1);
        sequence.general.mlayer_dependency_map =
            splot_core::headers::sequence::MLayerDependencyMap::default_for(
                EmbeddedLayerId::from_bits(1),
            );
        let mk_slot = |mlayer| {
            Some(QmSlot {
                mlayer_id: Some(mlayer),
                tlayer_id: Some(0),
                num_planes: 3,
                user: None,
            })
        };

        let mut state = OutputEffectState::new();
        state.qm[0] = mk_slot(0);
        state.qm[1] = mk_slot(1);
        state.qm_protected = 0;

        frame_envelope.header.embedded_layer_id = EmbeddedLayerId::from_bits(1);
        state.reset_switch_qm(frame_envelope, &sequence);
        assert!(
            state.qm[0].is_some(),
            "ML1 switch must preserve ML0 QM slot"
        );
        assert!(state.qm[1].is_none(), "ML1 switch must clear ML1 QM slot");

        state.qm[0] = mk_slot(0);
        state.qm[1] = mk_slot(1);
        frame_envelope.header.embedded_layer_id = EmbeddedLayerId::BASE;
        state.reset_switch_qm(frame_envelope, &sequence);
        assert!(state.qm[0].is_none(), "ML0 switch must clear ML0 QM slot");
        assert!(
            state.qm[1].is_none(),
            "ML0 switch must invalidate dependent ML1 QM slot"
        );
    }

    #[test]
    fn validate_annex_a_lcr_enforces_table_a4_by_profile() {
        const MLAYER_OBU: &[u8] = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-mlayer2-crossref-8bit.obu"
        );
        let obus = splot_core::annexb::parse_annex_b_obus(MLAYER_OBU).unwrap();
        let lcr_obu = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::LayerConfigurationRecord)
            .copied()
            .unwrap();
        let seq_obu = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
            .copied()
            .unwrap();
        let frame_obu = obus
            .iter()
            .find(|obu| obu.header.obu_type == ObuType::ClosedLoopKey)
            .copied()
            .unwrap();
        let mut reader = BitReader::new(seq_obu.payload, seq_obu.payload_offset());
        let mut sequence =
            splot_core::headers::sequence::parse_sequence_header(&mut reader).unwrap();

        let mut state = OutputEffectState::new();
        sequence.general.seq_profile_idc = splot_core::headers::sequence::ProfileIdc::from_bits(1);
        assert!(matches!(
            state.validate_annex_a_lcr(frame_obu, &sequence),
            Err(crate::DecodeError::MalformedSource { issue })
                if issue.spec_section() == Some("A.2")
                    && issue.message().contains("Annex A Table A.4")
        ));

        sequence.general.seq_profile_idc = splot_core::headers::sequence::ProfileIdc::from_bits(2);
        sequence.general.seq_lcr_id = splot_core::headers::sequence::LcrId::from_bits(1);
        state.global_lcr_xlayers[1] = Some(0b10);
        assert!(matches!(
            state.validate_annex_a_lcr(frame_obu, &sequence),
            Err(crate::DecodeError::MalformedSource { issue }) if issue.spec_section() == Some("A.2")
        ));
        state.global_lcr_xlayers[1] = Some(0b01);
        assert!(state.validate_annex_a_lcr(frame_obu, &sequence).is_ok());

        state.global_lcr_xlayers[1] = None;
        state.observe_lcr(lcr_obu).unwrap();
        assert!(state.local_lcr_in_cvs);
        sequence.general.seq_profile_idc = splot_core::headers::sequence::ProfileIdc::from_bits(1);
        assert!(state.validate_annex_a_lcr(frame_obu, &sequence).is_ok());
    }
}
