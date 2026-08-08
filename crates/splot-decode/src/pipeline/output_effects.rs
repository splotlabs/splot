// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! High-level output state consumed by one selected-layer decode.

use std::collections::BTreeMap;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::buffer_removal_timing::{
    BufferRemovalTiming, parse_buffer_removal_timing,
};
use splot_core::headers::content_interpretation::{
    ContentInterpretation, parse_content_interpretation,
};
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::metadata::{
    MetadataGroupUnit, MetadataType, MetadataUnit, parse_metadata_group, parse_metadata_short,
};
use splot_core::headers::operating_point_set::parse_operating_point_set;
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

use super::{PipelineFrameRate, unsupported_feature_at};
use crate::bitstream::tile_payload::{FrameUserQmLevel, FrameUserQmLevels};
use crate::error::Result;

const LAYER_GLOBAL: u8 = 1;
const LAYER_CURRENT: u8 = 2;
const LAYER_VALUES: u8 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrameOutputEffects {
    pub(crate) content_interpretation: Option<ContentInterpretation>,
    pub(crate) buffer_removal_timing: Option<BufferRemovalTiming>,
    pub(crate) metadata: Vec<MetadataUnit>,
}

impl FrameOutputEffects {
    #[cfg(test)]
    pub(crate) const fn empty() -> Self {
        Self {
            content_interpretation: None,
            buffer_removal_timing: None,
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
            return Err(effect_error(
                "content_interpretation_reserved_bits",
                ByteOffset::new(0),
                "content interpretation reserved bits must be zero",
                "6.14",
            ));
        }
        if let Some(brt) = &self.buffer_removal_timing
            && let Some((_, count)) = brt.ops_reference()
            && brt.op_timings().len() != usize::from(count)
        {
            return Err(effect_error(
                "buffer_removal_timing_count",
                ByteOffset::new(0),
                "BRT operating-point entry count does not match br_ops_cnt",
                "6.11",
            ));
        }
        if self
            .metadata
            .iter()
            .any(|unit| !type_matches_payload(unit.metadata_type, &unit.payload))
        {
            return Err(effect_error(
                "metadata_payload_type",
                ByteOffset::new(0),
                "attached metadata payload does not match its metadata_type",
                "6.16.1",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetadataPersistence {
    Global,
    Basic,
    No,
    Enhanced,
    Reserved,
}

impl MetadataPersistence {
    const fn from_idc(idc: u8) -> Self {
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

    fn applies_to_base(&self) -> bool {
        match self.layer_idc {
            0 | LAYER_GLOBAL => true,
            LAYER_CURRENT => self.source_mlayer.get() == 0,
            LAYER_VALUES => self.mlayer_map.is_some_and(|map| map & 1 != 0),
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

pub(crate) struct OutputEffectState {
    mfh: [Option<MultiFrameHeaderRecord>; MAX_MFH_NUM as usize],
    qm: [Option<QmSlot>; NUM_CUSTOM_QMS],
    qm_protected: u16,
    qm_seen_since_frame: u16,
    qm_obu_seen_since_frame: bool,
    content_interpretation: Option<ContentInterpretation>,
    ci_in_current_tu: Option<ContentInterpretation>,
    brt: Option<BufferRemovalTiming>,
    ops_counts: BTreeMap<(ExtendedLayerId, u8), u8>,
    metadata: Vec<ActiveMetadata>,
    tu_index: u64,
}

impl OutputEffectState {
    pub(crate) fn new() -> Self {
        Self {
            mfh: std::array::from_fn(|_| None),
            qm: std::array::from_fn(|_| None),
            qm_protected: 0,
            qm_seen_since_frame: 0,
            qm_obu_seen_since_frame: false,
            content_interpretation: None,
            ci_in_current_tu: None,
            brt: None,
            ops_counts: BTreeMap::new(),
            metadata: Vec::new(),
            tu_index: 0,
        }
    }

    pub(crate) fn begin_temporal_unit(&mut self) {
        self.tu_index = self.tu_index.saturating_add(1);
        self.qm_protected = 0;
        self.ci_in_current_tu = None;
    }

    pub(crate) fn observe_prefix(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        sequence: &SequenceHeader,
    ) -> Result<()> {
        for envelope in obus {
            match envelope.header.obu_type {
                ObuType::TemporalDelimiter => self.begin_temporal_unit(),
                ObuType::OperatingPointSet => self.observe_ops(*envelope)?,
                ObuType::MultiFrameHeader => self.observe_mfh(*envelope, sequence)?,
                ObuType::BufferRemovalTiming => self.observe_brt(*envelope)?,
                ObuType::QuantizationMatrix => self.observe_qm(*envelope)?,
                ObuType::ContentInterpretation => self.observe_ci(*envelope)?,
                ObuType::MetadataShort | ObuType::MetadataGroup => {
                    self.observe_metadata(*envelope, false)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn observe_suffix(&mut self, obus: &[ObuEnvelope<'_>]) -> Result<()> {
        for envelope in obus {
            match envelope.header.obu_type {
                ObuType::Padding => {}
                ObuType::MetadataShort | ObuType::MetadataGroup => {
                    self.observe_metadata(*envelope, true)?;
                }
                _ => {
                    return Err(effect_error(
                        "output_effect_suffix_order",
                        envelope.offset,
                        "coded-frame suffixes may contain only suffix metadata and padding",
                        "7.3.3",
                    ));
                }
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
    ) -> Result<Option<FrameUserQmLevels>> {
        let starts_cvs = first_picture_in_tu
            && (sequence.general.single_picture_header_flag
                || matches!(
                    envelope.header.obu_type,
                    ObuType::ClosedLoopKey | ObuType::OpenLoopKey
                ));
        if starts_cvs {
            self.reset_unprotected_qm();
            self.content_interpretation = self.ci_in_current_tu;
            self.metadata.retain(|unit| unit.tu_index == self.tu_index);
        } else if let Some(content) = self.ci_in_current_tu {
            if self
                .content_interpretation
                .is_some_and(|active| active != content)
            {
                return Err(effect_error(
                    "content_interpretation_changed_in_cvs",
                    envelope.offset,
                    "content interpretation must remain identical within a coded video sequence",
                    "6.14",
                ));
            }
            self.content_interpretation = Some(content);
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

    pub(crate) fn finish_frame(&mut self) -> FrameOutputEffects {
        let effects = FrameOutputEffects {
            content_interpretation: self.content_interpretation,
            buffer_removal_timing: self.brt.take(),
            metadata: self
                .metadata
                .iter()
                .filter(|unit| unit.applies_to_base())
                .map(|unit| unit.unit.clone())
                .collect(),
        };
        self.metadata
            .retain(|unit| unit.persistence != MetadataPersistence::No);
        effects
    }

    fn observe_mfh(&mut self, envelope: ObuEnvelope<'_>, sequence: &SequenceHeader) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let mfh = parse_multi_frame_header(&mut reader).map_err(|_| {
            effect_error(
                "multi_frame_header_parse",
                envelope.offset,
                "decode requires a complete multi-frame header OBU",
                "5.7",
            )
        })?;
        finish(&mut reader, envelope, "multi_frame_header_tail", "5.7")?;
        if !mfh.mfh_id_in_range() || !mfh.seq_header_id_in_range() {
            return Err(effect_error(
                "multi_frame_header_id_range",
                envelope.offset,
                "multi-frame and sequence-header ids must be in range",
                "6.7",
            ));
        }
        if mfh.mfh_seq_header_id != u32::from(sequence.general.seq_header_id.get()) {
            return Err(effect_error(
                "multi_frame_header_sequence_unavailable",
                envelope.offset,
                "the referenced sequence header is not active in the selected decode layer",
                "7.3.8.6",
            ));
        }
        let id = usize::try_from(mfh.mfh_id()).map_err(|_| {
            effect_error(
                "multi_frame_header_id_range",
                envelope.offset,
                "multi-frame header id does not fit the runtime slot index",
                "6.7",
            )
        })?;
        let seq_id =
            splot_core::headers::sequence::SequenceHeaderId::try_new(mfh.mfh_seq_header_id)
                .ok_or_else(|| {
                    effect_error(
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

    fn observe_qm(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let qm = parse_quantizer_matrix(&mut reader).map_err(|_| {
            effect_error(
                "quantizer_matrix_parse",
                envelope.offset,
                "decode requires a complete quantizer matrix OBU",
                "5.13",
            )
        })?;
        finish(&mut reader, envelope, "quantizer_matrix_tail", "5.13")?;
        if qm.is_reset() {
            if self.qm_obu_seen_since_frame {
                return Err(effect_error(
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
            return Err(effect_error(
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

    fn observe_ci(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let content = parse_content_interpretation(&mut reader).map_err(|_| {
            effect_error(
                "content_interpretation_parse",
                envelope.offset,
                "decode requires a complete content interpretation OBU",
                "5.15",
            )
        })?;
        finish(&mut reader, envelope, "content_interpretation_tail", "5.15")?;
        if let Some(existing) = self.ci_in_current_tu
            && existing != content
        {
            return Err(effect_error(
                "content_interpretation_changed_in_temporal_unit",
                envelope.offset,
                "content interpretation must remain identical within an embedded layer and coded video sequence",
                "6.14",
            ));
        }
        self.ci_in_current_tu = Some(content);
        Ok(())
    }

    fn observe_ops(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let ops = parse_operating_point_set(&mut reader, envelope.header.extended_layer_id)
            .map_err(|_| {
                effect_error(
                    "operating_point_set_parse",
                    envelope.offset,
                    "BRT resolution requires a complete operating point set OBU",
                    "5.10",
                )
            })?;
        finish(&mut reader, envelope, "operating_point_set_tail", "5.10")?;
        if ops.reset_flag {
            self.ops_counts
                .retain(|(xlayer, _), _| *xlayer != ops.xlayer_id);
        }
        if ops.ops_cnt == 0 {
            self.ops_counts.remove(&(ops.xlayer_id, ops.ops_id));
        } else {
            self.ops_counts
                .insert((ops.xlayer_id, ops.ops_id), ops.ops_cnt);
        }
        Ok(())
    }

    fn observe_brt(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let brt = parse_buffer_removal_timing(&mut reader).map_err(|_| {
            effect_error(
                "buffer_removal_timing_parse",
                envelope.offset,
                "decode requires a complete buffer removal timing OBU",
                "5.12",
            )
        })?;
        finish(&mut reader, envelope, "buffer_removal_timing_tail", "5.12")?;
        if let Some((ops_id, ops_count)) = brt.ops_reference() {
            let available = self
                .ops_counts
                .get(&(envelope.header.extended_layer_id, ops_id))
                .or_else(|| self.ops_counts.get(&(GLOBAL_XLAYER_ID, ops_id)));
            if available.copied() != Some(ops_count) {
                return Err(effect_error(
                    "buffer_removal_timing_ops_unavailable",
                    envelope.offset,
                    "BRT operating point id/count does not match an available OPS",
                    "6.11",
                ));
            }
        }
        if self.brt.replace(brt).is_some() {
            return Err(effect_error(
                "buffer_removal_timing_duplicate",
                envelope.offset,
                "only one BRT OBU is allowed in a coded non-output frame unit",
                "7.3.4",
            ));
        }
        Ok(())
    }

    fn observe_metadata(&mut self, envelope: ObuEnvelope<'_>, expected_suffix: bool) -> Result<()> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        match envelope.header.obu_type {
            ObuType::MetadataShort => {
                let short =
                    parse_metadata_short(&mut reader, envelope.payload.len()).map_err(|_| {
                        effect_error(
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
                    self.cancel_metadata(envelope.header.extended_layer_id, short.metadata_type);
                } else if let Some(unit) = short.unit {
                    self.store_metadata(
                        envelope,
                        short.muh_layer_idc,
                        short.muh_persistence_idc,
                        None,
                        unit,
                    );
                }
            }
            ObuType::MetadataGroup => {
                let group = parse_metadata_group(&mut reader, envelope.header.extended_layer_id)
                    .map_err(|_| {
                        effect_error(
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
                    self.observe_metadata_group_unit(envelope, group_unit);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn observe_metadata_group_unit(&mut self, envelope: ObuEnvelope<'_>, group: MetadataGroupUnit) {
        if group.muh_cancel_flag {
            self.cancel_metadata(envelope.header.extended_layer_id, group.metadata_type);
            return;
        }
        let mlayer_map = metadata_group_base_mlayer_map(envelope, &group);
        let (Some(layer_idc), Some(persistence), Some(unit)) =
            (group.muh_layer_idc, group.muh_persistence_idc, group.unit)
        else {
            return;
        };
        self.store_metadata(envelope, layer_idc, persistence, mlayer_map, unit);
    }

    fn store_metadata(
        &mut self,
        envelope: ObuEnvelope<'_>,
        layer_idc: u8,
        persistence_idc: u8,
        mlayer_map: Option<u8>,
        unit: MetadataUnit,
    ) {
        let active = ActiveMetadata {
            unit,
            persistence: MetadataPersistence::from_idc(persistence_idc),
            layer_idc,
            source_mlayer: envelope.header.embedded_layer_id,
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

    fn cancel_metadata(&mut self, xlayer: ExtendedLayerId, metadata_type: MetadataType) {
        if xlayer.is_global() || xlayer.get() == 0 {
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
        for index in 0..NUM_CUSTOM_QMS {
            if self.qm_protected & (1 << index) != 0 {
                continue;
            }
            let reset = self.qm[index].as_ref().is_none_or(|slot| {
                slot.mlayer_id.is_none_or(|mlayer| {
                    sequence.general.mlayer_dependency_map.depends_on(
                        envelope.header.embedded_layer_id,
                        EmbeddedLayerId::from_bits(mlayer),
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
                    return Err(effect_error(
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
                    return Err(effect_error(
                        "quantizer_matrix_mlayer_dependency",
                        envelope.offset,
                        "referenced QM level violates MLayerDependencyMap",
                        "6.17.6.2",
                    ));
                }
                if let (Some(mlayer), Some(tlayer)) = (slot.mlayer_id, slot.tlayer_id)
                    && !sequence.general.tlayer_dependency_map.depends_on(
                        EmbeddedLayerId::from_bits(mlayer),
                        envelope.header.temporal_layer_id,
                        splot_core::types::TemporalLayerId::from_bits(tlayer),
                    )
                {
                    return Err(effect_error(
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

fn metadata_group_base_mlayer_map(
    envelope: ObuEnvelope<'_>,
    group: &MetadataGroupUnit,
) -> Option<u8> {
    if envelope.header.extended_layer_id.is_global() {
        let xlayer_map = group.muh_xlayer_map?;
        let map_index = (0..31)
            .filter(|bit| xlayer_map & (1 << bit) != 0)
            .position(|bit| bit == 0)?;
        group.muh_mlayer_maps.get(map_index).copied()
    } else {
        group.muh_mlayer_maps.first().copied()
    }
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
        effect_error(
            reason,
            envelope.offset,
            "output-effect OBU has a malformed payload tail",
            spec,
        )
    })
}

fn metadata_order_error(envelope: ObuEnvelope<'_>, expected_suffix: bool) -> crate::DecodeError {
    effect_error(
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

fn effect_error(
    reason: &'static str,
    offset: ByteOffset,
    message: &'static str,
    spec: &'static str,
) -> crate::DecodeError {
    unsupported_feature_at(reason, offset, message, spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_core::stream::{ParsedBitstream, parse_bitstream_partial};

    const STANDALONE_OLK_FIXTURE: &[u8] =
        include_bytes!("../../../../tests/conformance/vectors/valid/syn-standalone-olk-64x64.ivf");

    fn content(scan_type: u8) -> splot_core::Result<ContentInterpretation> {
        let payload = [scan_type << 6];
        let mut reader = BitReader::new(&payload, ByteOffset::new(0));
        parse_content_interpretation(&mut reader)
    }

    #[test]
    fn no_persistence_metadata_expires_after_frame_snapshot() {
        let mut state = OutputEffectState::new();
        state.metadata.push(ActiveMetadata {
            unit: MetadataUnit {
                metadata_type: MetadataType::IccProfile,
                payload_size: 4,
                payload: splot_core::headers::metadata::MetadataPayload::IccProfile(
                    splot_core::headers::metadata::MetadataIccProfile { payload_len: 4 },
                ),
            },
            persistence: MetadataPersistence::No,
            layer_idc: LAYER_CURRENT,
            source_mlayer: EmbeddedLayerId::from_bits(0),
            mlayer_map: None,
            tu_index: 0,
        });
        assert_eq!(state.finish_frame().metadata.len(), 1);
        assert!(state.finish_frame().metadata.is_empty());
    }

    #[test]
    fn open_loop_key_not_first_in_tu_preserves_cvs_consistency_checks() {
        let ParsedBitstream::Ivf(ivf) = parse_bitstream_partial(STANDALONE_OLK_FIXTURE) else {
            return;
        };
        let sequence_envelope = ivf
            .frames
            .iter()
            .flat_map(|frame| &frame.obus)
            .find(|obu| obu.header.obu_type == ObuType::SequenceHeader);
        let olk_envelope = ivf
            .frames
            .iter()
            .flat_map(|frame| &frame.obus)
            .find(|obu| obu.header.obu_type == ObuType::OpenLoopKey);
        let (Some(sequence_envelope), Some(olk_envelope)) = (sequence_envelope, olk_envelope)
        else {
            return;
        };
        let sequence = crate::pipeline::parse_sequence(*sequence_envelope);
        assert!(sequence.is_ok());
        let Ok(sequence) = sequence else {
            return;
        };
        let core = crate::pipeline::parse_frame_core(*olk_envelope, &sequence);
        assert!(core.is_ok());
        let Ok(core) = core else {
            return;
        };
        let active = content(0);
        let current = content(1);
        assert!(active.is_ok() && current.is_ok());
        let (Ok(active), Ok(current)) = (active, current) else {
            return;
        };
        let mut state = OutputEffectState::new();
        state.content_interpretation = Some(active);
        state.ci_in_current_tu = Some(current);

        let result = state.prepare_frame(*olk_envelope, &core, &sequence, false);

        assert!(matches!(
            result,
            Err(crate::DecodeError::UnsupportedFeature { unsupported })
                if unsupported.reason() == "content_interpretation_changed_in_cvs"
        ));
    }
}
