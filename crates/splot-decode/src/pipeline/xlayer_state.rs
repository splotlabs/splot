// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-extended-layer decode state and global high-level syntax coordination.

use std::collections::BTreeMap;
use std::sync::Arc;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::layer_config_record::{LayerConfigurationRecord, LcrGlobalInfo};
use splot_core::headers::metadata::{MetadataType, MetadataUnit};
use splot_core::headers::operating_point_set::OperatingPointSet;
use splot_core::headers::sequence::{ProfileIdc, SequenceHeader, SequenceHeaderId, Tier};
use splot_core::hls::{MfhId, MultistreamDecoderOperation, parse_msdo};
use splot_core::obu::finish_obu_payload;
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType};

use super::atlas::{GlobalRecord, LayerAtlases, scope_mut};
use super::frame_lifecycle::{FilmGrainSlots, parse_sequence};
use super::output_effects::{
    MetadataAction, MetadataPersistence, OutputEffectState, parse_metadata_obu, parse_ops_obu,
    parse_validated_lcr,
};
use super::output_schedule::OutputScheduler;
use super::stream_schedule::{key_frame_activation, mfh_sequence_association};
use super::unsupported_feature_at;
use crate::bitstream::ops_plan::validate_ops_semantics;
use crate::bitstream::stream_plan::DecodeSourceIssue;
use crate::error::{DecodeError, Result};
use crate::reference::buffer::RuntimeReferenceBuffer;

#[derive(Default)]
pub(super) struct InBandLongTermPrelude {
    pub(super) frames: Vec<InBandLongTermFrame>,
}

pub(super) struct InBandLongTermFrame {
    pub(super) obu_type: ObuType,
    pub(super) long_term_id: u32,
    pub(super) hidden: bool,
    pub(super) frame_index: usize,
}

impl InBandLongTermPrelude {
    pub(super) fn begin_frame(&mut self, first_picture_in_tu: bool) {
        if first_picture_in_tu {
            self.frames.clear();
        }
    }

    pub(super) fn validate_required(
        &self,
        core: &FrameHeaderCore,
        reference: &RuntimeReferenceBuffer,
        offset: ByteOffset,
    ) -> Result<()> {
        self.validate_required_with(&core.ref_long_term_ids, offset, |id, frame_index| {
            reference.retains_hidden_long_term_reference(id, frame_index)
        })
    }

    pub(super) fn validate_required_with(
        &self,
        required_ids: &[u32],
        offset: ByteOffset,
        mut is_retained: impl FnMut(u32, usize) -> bool,
    ) -> Result<()> {
        if required_ids.is_empty() {
            return Ok(());
        }
        for &id in required_ids {
            if !self.frames.iter().any(|frame| frame.long_term_id == id) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_missing",
                    offset,
                    "each ref_long_term_id must name a preceding in-band key frame in the random-access temporal unit",
                    "7.3.9.1",
                ));
            }
            if self
                .frames
                .iter()
                .any(|frame| frame.long_term_id == id && !frame.hidden)
            {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_visible",
                    offset,
                    "in-band long-term reference key frames must disable immediate and implicit output",
                    "7.3.9.1",
                ));
            }
            if !self.frames.iter().any(|frame| {
                frame.long_term_id == id && frame.hidden && is_retained(id, frame.frame_index)
            }) {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_slot_unavailable",
                    offset,
                    "an in-band long-term reference must remain in the same valid reference slot used by sequential decoding",
                    "7.3.9.1",
                ));
            }
        }
        let mut saw_olk = false;
        for frame in self
            .frames
            .iter()
            .filter(|frame| required_ids.contains(&frame.long_term_id))
        {
            if frame.obu_type == ObuType::OpenLoopKey {
                saw_olk = true;
            } else if frame.obu_type == ObuType::ClosedLoopKey && saw_olk {
                return Err(unsupported_feature_at(
                    "random_access_long_term_reference_order",
                    offset,
                    "in-band long-term CLK references must precede all in-band long-term OLK references",
                    "7.3.9.1",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn note_frame(&mut self, core: &FrameHeaderCore, frame_index: usize) {
        if !matches!(core.obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey) {
            return;
        }
        let Some(long_term_id) = core.long_term_id.and_then(|id| u32::try_from(id).ok()) else {
            return;
        };
        self.frames.push(InBandLongTermFrame {
            obu_type: core.obu_type,
            long_term_id,
            hidden: core.immediate_output_frame == Some(false)
                && core.implicit_output_frame == Some(false),
            frame_index,
        });
    }
}

#[derive(Clone)]
struct GlobalMetadataItem {
    persistence: MetadataPersistence,
    layer_idc: u8,
    source_mlayer: EmbeddedLayerId,
    xlayer_map: Option<u32>,
    mlayer_maps: Vec<u8>,
    unit: MetadataUnit,
    tu_index: u64,
}

impl GlobalMetadataItem {
    fn same_scope(&self, other: &Self) -> bool {
        self.layer_idc == other.layer_idc
            && self.source_mlayer == other.source_mlayer
            && self.xlayer_map == other.xlayer_map
            && self.mlayer_maps == other.mlayer_maps
    }

    fn to_action(&self, persistence_idc: u8) -> MetadataAction {
        MetadataAction::Store {
            layer_idc: self.layer_idc,
            persistence_idc,
            source_mlayer: self.source_mlayer,
            xlayer_map: self.xlayer_map,
            mlayer_maps: self.mlayer_maps.clone(),
            unit: self.unit.clone(),
        }
    }
}

#[derive(Default)]
struct GlobalMetadataEntry {
    item: Option<(u8, GlobalMetadataItem)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SubStreamCeiling {
    profile: u8,
    level: u8,
    tier: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MsdoKeyFields {
    profile_idc: u8,
    level_idx: u8,
    tier: u8,
    num_streams_minus_2: u8,
    even_allocation_flag: bool,
    large_picture_idc: Option<u8>,
}

impl MsdoKeyFields {
    fn from_msdo(msdo: &MultistreamDecoderOperation) -> Self {
        Self {
            profile_idc: msdo.multistream_profile_idc.get(),
            level_idx: msdo.multistream_level_idx,
            tier: msdo.multistream_tier,
            num_streams_minus_2: msdo.num_streams_minus_2,
            even_allocation_flag: msdo.multistream_even_allocation_flag,
            large_picture_idc: msdo.multistream_large_picture_idc,
        }
    }
}

#[derive(Clone)]
struct ActiveMsdo {
    msdo: MultistreamDecoderOperation,
    payload: Vec<u8>,
    key_fields: MsdoKeyFields,
    ceilings: BTreeMap<u8, SubStreamCeiling>,
}

impl ActiveMsdo {
    fn from_validated(msdo: MultistreamDecoderOperation, payload: &[u8]) -> Self {
        let mut ceilings: BTreeMap<u8, SubStreamCeiling> = BTreeMap::new();
        for sub in msdo.sub_streams() {
            let entry = SubStreamCeiling {
                profile: sub.sub_stream_max_profile,
                level: sub.sub_stream_max_level,
                tier: sub.sub_stream_max_tier,
            };
            ceilings
                .entry(sub.sub_xlayer_id)
                .and_modify(|existing| {
                    existing.profile = existing.profile.min(entry.profile);
                    existing.level = existing.level.min(entry.level);
                    existing.tier = existing.tier.min(entry.tier);
                })
                .or_insert(entry);
        }
        Self {
            msdo,
            payload: payload.to_vec(),
            key_fields: MsdoKeyFields::from_msdo(&msdo),
            ceilings,
        }
    }
}

#[derive(Default)]
pub(super) struct GlobalHlsState {
    tu_index: u64,
    global_lcrs: [Option<GlobalRecord>; 8],
    atlases: super::atlas::ScopedAtlases,
    activated_global_lcrs: [Option<Arc<LcrGlobalInfo>>; 8],
    global_ops_counts: BTreeMap<u8, u8>,
    global_metadata: Vec<GlobalMetadataEntry>,
    active_msdo: Option<ActiveMsdo>,
    cmvs_start_tu_index: Option<u64>,
    tu_saw_msdo: bool,
    tu_saw_non_msdo_prefix: bool,
    last_celu_xlayer: Option<u8>,
    tu_order_hint_bits: Option<u8>,
    tu_output_order_hint: Option<(ExtendedLayerId, u32)>,
}

impl GlobalHlsState {
    fn begin_temporal_unit(&mut self) {
        self.tu_index = self.tu_index.saturating_add(1);
        self.tu_saw_msdo = false;
        self.tu_saw_non_msdo_prefix = false;
        self.last_celu_xlayer = None;
        self.tu_order_hint_bits = None;
        self.tu_output_order_hint = None;
        self.global_metadata.retain(|entry| {
            entry
                .item
                .as_ref()
                .is_some_and(|(_, item)| item.persistence != MetadataPersistence::No)
        });
    }

    fn record_ops(&mut self, ops: &OperatingPointSet) {
        if ops.reset_flag {
            self.global_ops_counts.clear();
        }
        if ops.ops_cnt == 0 {
            self.global_ops_counts.remove(&ops.ops_id);
        } else {
            self.global_ops_counts.insert(ops.ops_id, ops.ops_cnt);
        }
    }

    fn cancel_metadata(&mut self, metadata_type: MetadataType) {
        self.global_metadata.retain(|entry| {
            entry.item.as_ref().is_some_and(|(_, item)| {
                item.unit.metadata_type != metadata_type
                    || item.persistence == MetadataPersistence::Global
            })
        });
    }

    fn record_metadata_action(&mut self, action: &MetadataAction) {
        match action {
            MetadataAction::Cancel(metadata_type) => self.cancel_metadata(*metadata_type),
            MetadataAction::Store {
                layer_idc,
                persistence_idc,
                source_mlayer,
                xlayer_map,
                mlayer_maps,
                unit,
            } => {
                let candidate = GlobalMetadataItem {
                    persistence: MetadataPersistence::from_idc(*persistence_idc),
                    layer_idc: *layer_idc,
                    source_mlayer: *source_mlayer,
                    xlayer_map: *xlayer_map,
                    mlayer_maps: mlayer_maps.clone(),
                    unit: unit.clone(),
                    tu_index: self.tu_index,
                };
                match candidate.persistence {
                    MetadataPersistence::Global => {
                        self.global_metadata.retain(|entry| {
                            entry.item.as_ref().is_some_and(|(_, item)| {
                                item.unit.metadata_type != candidate.unit.metadata_type
                                    || item.persistence != MetadataPersistence::Global
                            })
                        });
                    }
                    MetadataPersistence::Basic | MetadataPersistence::Enhanced => {
                        self.global_metadata.retain(|entry| {
                            entry.item.as_ref().is_some_and(|(_, item)| {
                                item.unit.metadata_type != candidate.unit.metadata_type
                                    || item.persistence == MetadataPersistence::Global
                                    || !item.same_scope(&candidate)
                            })
                        });
                    }
                    MetadataPersistence::No | MetadataPersistence::Reserved => {}
                }
                self.global_metadata.push(GlobalMetadataEntry {
                    item: Some((*persistence_idc, candidate)),
                });
            }
        }
    }

    fn sync_effects(&self, effects: &mut OutputEffectState) {
        effects.set_tu_index(self.tu_index);
        for (id, slot) in self.global_lcrs.iter().enumerate() {
            if let Some((_, _, info)) = slot {
                effects.apply_global_lcr(id as u8, info.xlayer_map);
            }
        }
        for (&ops_id, &ops_cnt) in &self.global_ops_counts {
            effects.apply_ops_entry(GLOBAL_XLAYER_ID, ops_id, ops_cnt);
        }
        for entry in &self.global_metadata {
            if let Some((persistence_idc, item)) = &entry.item
                && (matches!(
                    item.persistence,
                    MetadataPersistence::Global
                        | MetadataPersistence::Basic
                        | MetadataPersistence::Enhanced
                ) || item.tu_index == self.tu_index)
            {
                effects.apply_metadata_action(GLOBAL_XLAYER_ID, item.to_action(*persistence_idc));
            }
        }
    }

    fn global_lcr(&self, lcr_id: u8) -> Option<(u64, &LcrGlobalInfo)> {
        self.global_lcrs
            .get(usize::from(lcr_id))
            .and_then(|slot| slot.as_ref().map(|(tu, _, info)| (*tu, info.as_ref())))
    }

    fn global_lcr_has_xlayer(&self, lcr_id: u8, xlayer: ExtendedLayerId) -> bool {
        self.global_lcr(lcr_id)
            .is_some_and(|(_, info)| (info.xlayer_map & (1u32 << u32::from(xlayer.get()))) != 0)
    }

    fn any_doh_constrained(&self) -> bool {
        if self
            .active_msdo
            .as_ref()
            .is_some_and(|active| active.msdo.multistream_doh_constraint_flag)
        {
            return true;
        }
        self.activated_global_lcrs
            .iter()
            .flatten()
            .any(|record| record.doh_constraint_flag)
    }

    fn has_multistream_mode(&self) -> bool {
        self.active_msdo.is_some() || self.activated_global_lcrs.iter().any(Option::is_some)
    }

    fn xlayer_declared_in_multistream(&self, xlayer: ExtendedLayerId) -> bool {
        if let Some(active) = &self.active_msdo {
            return active.ceilings.contains_key(&xlayer.get());
        }
        self.activated_global_lcrs
            .iter()
            .flatten()
            .any(|record| record.xlayer_map & (1 << xlayer.get()) != 0)
    }
}

fn malformed(offset: ByteOffset, spec_section: &'static str, message: String) -> DecodeError {
    DecodeError::MalformedSource {
        issue: DecodeSourceIssue::sequence_header_conformance(offset, spec_section, message),
    }
}

fn iop_of_profile(profile_idc: u8) -> Option<u8> {
    match profile_idc {
        0 => Some(0),
        1 | 3 | 4 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

fn config_idc_allows_profile(config_idc: u8, profile_idc: u8) -> bool {
    match config_idc {
        0 => matches!(profile_idc, 0..=2 | 31),
        1 => matches!(profile_idc, 0..=3 | 31),
        2 => matches!(profile_idc, 0..=2 | 4 | 31),
        _ => true,
    }
}

fn parse_validated_msdo(envelope: ObuEnvelope<'_>) -> Result<MultistreamDecoderOperation> {
    let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
    let msdo = parse_msdo(&mut reader)
        .map_err(|error| malformed(envelope.offset, "5.6", format!("invalid MSDO: {error}")))?;
    finish_obu_payload(
        &mut reader,
        envelope.payload,
        envelope.header.obu_type.is_extensible_obu(),
    )
    .map_err(|error| {
        malformed(
            envelope.offset,
            "5.6",
            format!("MSDO payload tail invalid: {error}"),
        )
    })?;
    if msdo.num_streams_minus_2 > 2 {
        return Err(malformed(
            envelope.offset,
            "6.6",
            format!(
                "OBU_MSDO num_streams_minus_2 {} exceeds maximum 2",
                msdo.num_streams_minus_2
            ),
        ));
    }
    if msdo.multistream_profile_idc.is_reserved() {
        return Err(malformed(
            envelope.offset,
            "A.2",
            format!(
                "OBU_MSDO uses reserved multistream_profile_idc {}",
                msdo.multistream_profile_idc.get()
            ),
        ));
    }
    if !msdo.multistream_even_allocation_flag
        && msdo
            .multistream_large_picture_idc
            .is_none_or(|idx| u32::from(idx) >= msdo.num_streams())
    {
        return Err(malformed(
            envelope.offset,
            "6.6",
            "OBU_MSDO multistream_large_picture_idc exceeds declared stream count".to_owned(),
        ));
    }
    for sub in msdo.sub_streams() {
        if sub.sub_xlayer_id >= GLOBAL_XLAYER_ID.get() {
            return Err(malformed(
                envelope.offset,
                "6.6",
                format!(
                    "OBU_MSDO sub_xlayer_id {} must be below GLOBAL_XLAYER_ID (31)",
                    sub.sub_xlayer_id
                ),
            ));
        }
        let sub_profile = ProfileIdc::from_bits(sub.sub_stream_max_profile);
        if sub_profile.is_reserved()
            || sub.sub_stream_max_profile > msdo.multistream_profile_idc.get()
        {
            return Err(malformed(
                envelope.offset,
                "6.6",
                format!(
                    "OBU_MSDO sub_stream_max_profile {} is invalid for multistream_profile_idc {}",
                    sub.sub_stream_max_profile,
                    msdo.multistream_profile_idc.get()
                ),
            ));
        }
    }
    Ok(msdo)
}

fn check_msdo_global_lcr_agreement(
    msdo: &MultistreamDecoderOperation,
    lcr: &LcrGlobalInfo,
    offset: ByteOffset,
) -> Result<()> {
    if msdo.num_streams() != lcr.xlayer_map.count_ones() {
        return Err(malformed(
            offset,
            "6.8.2",
            format!(
                "OBU_MSDO stream count {} disagrees with global LCR {} xlayer count {}",
                msdo.num_streams(),
                lcr.global_config_record_id,
                lcr.xlayer_map.count_ones()
            ),
        ));
    }
    for sub in msdo.sub_streams() {
        if (lcr.xlayer_map & (1u32 << u32::from(sub.sub_xlayer_id))) == 0 {
            return Err(malformed(
                offset,
                "6.8.2",
                format!(
                    "OBU_MSDO sub_xlayer_id {} is absent from global LCR {} lcr_xlayer_map",
                    sub.sub_xlayer_id, lcr.global_config_record_id
                ),
            ));
        }
    }
    if msdo.multistream_doh_constraint_flag != lcr.doh_constraint_flag {
        return Err(malformed(
            offset,
            "6.8.2",
            "OBU_MSDO multistream_doh_constraint_flag disagrees with global LCR lcr_doh_constraint_flag".to_owned(),
        ));
    }
    if let Some(agg) = lcr.aggregate_info {
        if !config_idc_allows_profile(agg.config_idc, msdo.multistream_profile_idc.get()) {
            return Err(malformed(
                offset,
                "6.8.2",
                format!(
                    "OBU_MSDO profile {} disagrees with global LCR lcr_config_idc {}",
                    msdo.multistream_profile_idc.get(),
                    agg.config_idc
                ),
            ));
        }
        if let Some(iop) = iop_of_profile(msdo.multistream_profile_idc.get())
            && iop != agg.max_interop
        {
            return Err(malformed(
                offset,
                "6.8.2",
                format!(
                    "OBU_MSDO profile IOP {iop} disagrees with global LCR lcr_max_interop {}",
                    agg.max_interop
                ),
            ));
        }
        if msdo.multistream_level_idx != agg.aggregate_level_idx
            || (msdo.multistream_tier != 0) != agg.max_tier_flag
        {
            return Err(malformed(
                offset,
                "6.8.2",
                "OBU_MSDO aggregate level or tier disagrees with global LCR".to_owned(),
            ));
        }
    }
    if lcr.seq_ptl_info_present {
        for sub in msdo.sub_streams() {
            let Some(ptl) = lcr
                .seq_ptl_infos
                .iter()
                .find(|entry| entry.xlayer_id == sub.sub_xlayer_id)
            else {
                return Err(malformed(
                    offset,
                    "6.8.2",
                    format!(
                        "global LCR missing PTL info for MSDO sub_xlayer_id {}",
                        sub.sub_xlayer_id
                    ),
                ));
            };
            if sub.sub_stream_max_profile != ptl.seq_profile_idc.get()
                || sub.sub_stream_max_level != ptl.max_level_idx
                || (sub.sub_stream_max_tier != 0) != ptl.tier_flag
            {
                return Err(malformed(
                    offset,
                    "6.8.2",
                    format!(
                        "OBU_MSDO sub-stream PTL for xlayer {} disagrees with global LCR",
                        sub.sub_xlayer_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(super) struct XLayerState {
    pub(super) xlayer_id: ExtendedLayerId,
    pub(super) sequence: SequenceHeader,
    pub(super) shared_sequence: Option<Arc<SequenceHeader>>,
    pub(super) reference: RuntimeReferenceBuffer,
    pub(super) scheduler: OutputScheduler,
    pub(super) output_effect_state: OutputEffectState,
    pub(super) film_grain_slots: FilmGrainSlots,
    pub(super) in_band_long_term_prelude: InBandLongTermPrelude,
    seen_frame_in_tu: bool,
    pub(super) decoding_initial_tu: bool,
    pub(super) discard_leading_frames: bool,
}

impl XLayerState {
    #[expect(clippy::too_many_arguments)]
    pub(super) fn new(
        xlayer_id: ExtendedLayerId,
        sequence: SequenceHeader,
        reference: RuntimeReferenceBuffer,
        scheduler: OutputScheduler,
        output_effect_state: OutputEffectState,
        film_grain_slots: FilmGrainSlots,
        in_band_long_term_prelude: InBandLongTermPrelude,
        discard_leading_frames: bool,
    ) -> Self {
        Self {
            xlayer_id,
            sequence,
            shared_sequence: None,
            reference,
            scheduler,
            output_effect_state,
            film_grain_slots,
            in_band_long_term_prelude,
            seen_frame_in_tu: true,
            decoding_initial_tu: true,
            discard_leading_frames,
        }
    }

    fn on_temporal_delimiter(&mut self, clear_discard: bool) {
        self.seen_frame_in_tu = false;
        self.decoding_initial_tu = false;
        if clear_discard {
            self.discard_leading_frames = false;
        }
        self.in_band_long_term_prelude.begin_frame(true);
        self.output_effect_state.begin_temporal_unit();
    }

    pub(super) fn begin_coded_picture(&mut self) -> bool {
        let first_picture_in_tu = !self.seen_frame_in_tu;
        self.seen_frame_in_tu = true;
        if first_picture_in_tu {
            self.decoding_initial_tu = false;
            self.discard_leading_frames = false;
        }
        self.in_band_long_term_prelude
            .begin_frame(first_picture_in_tu);
        first_picture_in_tu
    }

    pub(super) fn begin_discarded_leading_frame(&mut self) {
        self.seen_frame_in_tu = true;
    }
}

struct RetiredLayerHls {
    xlayer_id: ExtendedLayerId,
    sequence: SequenceHeader,
    output_effect_state: OutputEffectState,
    film_grain_slots: FilmGrainSlots,
}

impl From<XLayerState> for RetiredLayerHls {
    fn from(layer: XLayerState) -> Self {
        Self {
            xlayer_id: layer.xlayer_id,
            sequence: layer.sequence,
            output_effect_state: layer.output_effect_state,
            film_grain_slots: layer.film_grain_slots,
        }
    }
}

pub(super) struct XLayerTable {
    first: Option<XLayerState>,
    rest: Vec<XLayerState>,
    global: GlobalHlsState,
    pending_cmvs_flushed: Vec<usize>,
    layer_atlases: Vec<Option<LayerAtlases>>,
    prior_emitted_count: usize,
    retired: Vec<RetiredLayerHls>,
}

impl XLayerTable {
    pub(super) fn new(mut external_atlases: crate::ExternalAtlases) -> Self {
        let atlases = external_atlases
            .scopes
            .get_mut(usize::from(GLOBAL_XLAYER_ID.get()))
            .and_then(Option::take)
            .unwrap_or_default();
        Self {
            layer_atlases: external_atlases
                .scopes
                .into_iter()
                .map(|scope| scope.map(LayerAtlases::from))
                .collect(),
            first: None,
            rest: Vec::new(),
            global: GlobalHlsState {
                atlases,
                ..Default::default()
            },
            pending_cmvs_flushed: Vec::new(),
            prior_emitted_count: 0,
            retired: Vec::new(),
        }
    }

    pub(super) fn take_pending_cmvs_flushed(&mut self) -> Vec<usize> {
        core::mem::take(&mut self.pending_cmvs_flushed)
    }

    fn iter(&self) -> impl Iterator<Item = &XLayerState> {
        self.first.iter().chain(self.rest.iter())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut XLayerState> {
        self.first.iter_mut().chain(self.rest.iter_mut())
    }

    pub(super) fn len(&self) -> usize {
        usize::from(self.first.is_some()) + self.rest.len()
    }

    pub(super) fn contains(&self, xlayer: ExtendedLayerId) -> bool {
        self.get(xlayer).is_some()
    }

    pub(super) fn get(&self, xlayer: ExtendedLayerId) -> Option<&XLayerState> {
        if let Some(first) = &self.first
            && first.xlayer_id == xlayer
        {
            return Some(first);
        }
        self.rest.iter().find(|layer| layer.xlayer_id == xlayer)
    }

    pub(super) fn get_mut(&mut self, xlayer: ExtendedLayerId) -> Option<&mut XLayerState> {
        if let Some(first) = &mut self.first
            && first.xlayer_id == xlayer
        {
            return Some(first);
        }
        self.rest.iter_mut().find(|layer| layer.xlayer_id == xlayer)
    }

    pub(super) fn insert(&mut self, mut state: XLayerState) {
        let Some(first) = &mut self.first else {
            self.first = Some(state);
            return;
        };
        if state.xlayer_id == first.xlayer_id {
            *first = state;
            return;
        }
        if state.xlayer_id.get() < first.xlayer_id.get() {
            core::mem::swap(first, &mut state);
        }
        match self
            .rest
            .binary_search_by_key(&state.xlayer_id.get(), |layer| layer.xlayer_id.get())
        {
            Ok(idx) => self.rest[idx] = state,
            Err(idx) => self.rest.insert(idx, state),
        }
    }

    pub(super) fn newly_and_total(&self, xlayer: ExtendedLayerId) -> (&[usize], usize) {
        let total = self.total_emitted_count();
        let newly = self
            .get(xlayer)
            .map_or(&[][..], |layer| layer.scheduler.newly());
        (newly, total)
    }

    pub(super) fn available_sequence(&self, xlayer: ExtendedLayerId) -> Option<&SequenceHeader> {
        self.available_hls(xlayer).map(|(sequence, _)| sequence)
    }

    pub(super) fn available_mfh_sequence(
        &self,
        xlayer: ExtendedLayerId,
        mfh_id: MfhId,
    ) -> Option<SequenceHeaderId> {
        self.available_hls(xlayer)?
            .1
            .mfh_record(mfh_id)
            .map(|record| record.mfh_seq_header_id)
    }

    fn available_hls(
        &self,
        xlayer: ExtendedLayerId,
    ) -> Option<(&SequenceHeader, &OutputEffectState)> {
        self.get(xlayer)
            .map(|layer| (&layer.sequence, &layer.output_effect_state))
            .or_else(|| {
                self.retired
                    .iter()
                    .find(|layer| layer.xlayer_id == xlayer)
                    .map(|layer| (&layer.sequence, &layer.output_effect_state))
            })
    }

    fn effect_states_mut(&mut self) -> impl Iterator<Item = &mut OutputEffectState> {
        self.first
            .iter_mut()
            .chain(&mut self.rest)
            .map(|layer| &mut layer.output_effect_state)
            .chain(
                self.retired
                    .iter_mut()
                    .map(|layer| &mut layer.output_effect_state),
            )
    }

    pub(super) fn take_layer_hls(
        &mut self,
        xlayer: ExtendedLayerId,
    ) -> (OutputEffectState, FilmGrainSlots) {
        if let Some(index) = self
            .retired
            .iter()
            .position(|layer| layer.xlayer_id == xlayer)
        {
            let layer = self.retired.swap_remove(index);
            return (layer.output_effect_state, layer.film_grain_slots);
        }
        let mut effects = OutputEffectState::new_for_xlayer(xlayer);
        self.global.sync_effects(&mut effects);
        (effects, FilmGrainSlots::new())
    }

    pub(super) fn is_discarding_leading_frames(&self, xlayer: ExtendedLayerId) -> bool {
        self.get(xlayer)
            .is_some_and(|layer| layer.discard_leading_frames)
    }

    pub(super) fn has_active_cmvs(&self) -> bool {
        self.global.has_multistream_mode()
    }

    pub(super) fn global_lcr_activation_in_tu(
        &self,
        obus: &[ObuEnvelope<'_>],
    ) -> Result<Option<ByteOffset>> {
        if !self.has_active_cmvs() {
            return Ok(None);
        }
        let mut global_maps = self
            .global
            .global_lcrs
            .each_ref()
            .map(|slot| slot.as_ref().map_or(0, |(_, _, record)| record.xlayer_map));
        if global_maps.iter().all(|&map| map == 0)
            && !obus.iter().any(|env| {
                env.header.obu_type == ObuType::LayerConfigurationRecord
                    && env.header.extended_layer_id.is_global()
            })
        {
            return Ok(None);
        }
        let resolves_global = |global: &[u32; 8], local: u8, id: u8, xlayer: usize| {
            id != 0 && local & (1 << id) == 0 && global[usize::from(id)] & (1 << xlayer) != 0
        };
        let mut local_ids = [0u8; 31];
        for (xlayer, ids) in local_ids.iter_mut().enumerate() {
            let xlayer = ExtendedLayerId::from_bits(xlayer as u8);
            *ids = self
                .layer_atlases(xlayer)
                .map_or(0, |atlas| atlas.local_ids);
        }
        let mut global_associations = BTreeMap::new();
        let mut staged_mfhs = BTreeMap::new();
        for env in obus {
            let xlayer = usize::from(env.header.extended_layer_id.get());
            match env.header.obu_type {
                ObuType::LayerConfigurationRecord => match parse_validated_lcr(*env)? {
                    LayerConfigurationRecord::Global(info) => {
                        global_maps[usize::from(info.global_config_record_id)] = info.xlayer_map;
                    }
                    LayerConfigurationRecord::Local(info) => {
                        local_ids[xlayer] |= 1 << info.local_id;
                    }
                    _ => {}
                },
                ObuType::SequenceHeader if !env.header.extended_layer_id.is_global() => {
                    let sequence = parse_sequence(*env)?;
                    let id = sequence.general.seq_lcr_id.get();
                    let global = resolves_global(&global_maps, local_ids[xlayer], id, xlayer);
                    global_associations.insert(
                        (xlayer, sequence.general.seq_header_id),
                        global.then_some(env.offset),
                    );
                }
                ObuType::MultiFrameHeader if !env.header.extended_layer_id.is_global() => {
                    let (id, sequence) = mfh_sequence_association(*env)?;
                    staged_mfhs.insert((xlayer, id), sequence);
                }
                ObuType::ClosedLoopKey | ObuType::OpenLoopKey
                    if !env.header.extended_layer_id.is_global() =>
                {
                    if env.payload.first().is_some_and(|first| first & 0x80 == 0) {
                        continue;
                    }
                    let prefix = key_frame_activation(*env)?;

                    let sequence_id = prefix
                        .referenced_sequence_header_id
                        .or_else(|| staged_mfhs.get(&(xlayer, prefix.cur_mfh_id)).copied())
                        .or_else(|| {
                            self.available_mfh_sequence(
                                env.header.extended_layer_id,
                                prefix.cur_mfh_id,
                            )
                        });
                    if let Some(sequence_id) = sequence_id {
                        match global_associations.get(&(xlayer, sequence_id)) {
                            Some(Some(offset)) => return Ok(Some(*offset)),
                            Some(None) => {}
                            None => {
                                let layer = env.header.extended_layer_id;
                                if self
                                    .layer_atlases(layer)
                                    .and_then(|atlases| atlases.binding(sequence_id))
                                    .is_some_and(|binding| binding.direct_global_for(layer))
                                {
                                    return Ok(Some(env.offset));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    pub(super) fn observe_global_obus(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        skipped_tu: bool,
        tu_has_rap: bool,
        tu_has_clk: bool,
        global_lcr_activation: Option<ByteOffset>,
    ) -> Result<()> {
        let mut cmvs_reset = false;
        let mut starts_temporal_unit = false;
        let has_layers = self.first.is_some();
        for envelope in obus {
            if !envelope.header.extended_layer_id.is_global() {
                self.observe_layer_atlas(*envelope)?;
                continue;
            }
            match envelope.header.obu_type {
                ObuType::TemporalDelimiter => {
                    starts_temporal_unit = true;
                    self.global.begin_temporal_unit();
                    for layer in self.iter_mut() {
                        layer.on_temporal_delimiter(skipped_tu);
                    }
                    for layer in &mut self.retired {
                        layer.output_effect_state.begin_temporal_unit();
                    }
                }
                ObuType::Msdo => {
                    if self.global.tu_saw_msdo {
                        return Err(malformed(
                            envelope.offset,
                            "7.3.7",
                            "multiple OBU_MSDO in one temporal unit".to_owned(),
                        ));
                    }
                    if self.global.tu_saw_non_msdo_prefix || self.global.last_celu_xlayer.is_some()
                    {
                        return Err(malformed(
                            envelope.offset,
                            "7.3.7",
                            "OBU_MSDO must precede other global prefix OBUs and coded extended layer units"
                                .to_owned(),
                        ));
                    }
                    let msdo = parse_validated_msdo(*envelope)?;
                    let next_active = ActiveMsdo::from_validated(msdo, envelope.payload);
                    let mut triggered_reset = false;
                    if let Some(prev) = &self.global.active_msdo {
                        if !tu_has_rap && next_active.payload != prev.payload {
                            return Err(malformed(
                                envelope.offset,
                                "7.3.8.2",
                                "non-RAP OBU_MSDO differs from previous OBU_MSDO".to_owned(),
                            ));
                        }
                        if next_active.key_fields != prev.key_fields {
                            if !tu_has_clk && has_layers {
                                return Err(malformed(
                                    envelope.offset,
                                    "7.3.2",
                                    "OBU_MSDO parameters cannot change without an OBU_CLOSED_LOOP_KEY"
                                        .to_owned(),
                                ));
                            }
                            triggered_reset = has_layers;
                        }
                    }
                    if !triggered_reset {
                        for info in self.global.activated_global_lcrs.iter().flatten() {
                            check_msdo_global_lcr_agreement(&msdo, info, envelope.offset)?;
                        }
                    }
                    self.global.tu_saw_msdo = true;
                    if triggered_reset || self.global.cmvs_start_tu_index.is_none() {
                        self.global.cmvs_start_tu_index = Some(self.global.tu_index);
                    }
                    if triggered_reset {
                        self.global.activated_global_lcrs.fill(None);
                        cmvs_reset = true;
                    }
                    self.global.active_msdo = Some(next_active);
                }
                ObuType::AtlasSegment => {
                    self.global.tu_saw_non_msdo_prefix = true;
                    self.global.atlases.observe(*envelope)?;
                }
                ObuType::LayerConfigurationRecord => {
                    self.global.tu_saw_non_msdo_prefix = true;
                    if let LayerConfigurationRecord::Global(info) = parse_validated_lcr(*envelope)?
                    {
                        let idx = usize::from(info.global_config_record_id);
                        let xlayer_map = info.xlayer_map;
                        if let Some(slot) = self.global.global_lcrs.get_mut(idx) {
                            *slot = Some((self.global.tu_index, envelope.offset, Arc::new(info)));
                        }
                        for effects in self.effect_states_mut() {
                            effects.apply_global_lcr(idx as u8, xlayer_map);
                        }
                    }
                }
                ObuType::OperatingPointSet => {
                    self.global.tu_saw_non_msdo_prefix = true;
                    let ops = parse_ops_obu(*envelope)?;
                    validate_ops_semantics(&ops, envelope.offset)?;
                    self.global.record_ops(&ops);
                    for effects in self.effect_states_mut() {
                        effects.apply_ops_record(&ops);
                    }
                }
                ObuType::MetadataShort | ObuType::MetadataGroup => {
                    self.global.tu_saw_non_msdo_prefix = true;
                    parse_metadata_obu(*envelope, false, |action| {
                        self.global.record_metadata_action(&action);
                        for effects in self.effect_states_mut() {
                            effects.apply_metadata_action(GLOBAL_XLAYER_ID, action.clone());
                        }
                    })?;
                }
                _ => {}
            }
        }
        let starts_cvs_without_msdo =
            starts_temporal_unit && tu_has_clk && !self.global.tu_saw_msdo;
        if starts_cvs_without_msdo
            && self.global.active_msdo.is_some()
            && let Some(offset) = global_lcr_activation
        {
            return Err(malformed(
                offset,
                "7.3.2",
                "CMVS boundary differs with and without the activated global LCR".to_owned(),
            ));
        }
        let cmvs_ended =
            starts_cvs_without_msdo && self.has_active_cmvs() && global_lcr_activation.is_none();
        if cmvs_reset || cmvs_ended {
            let flushed = self.flush_all_outputs_and_invalidate();
            self.pending_cmvs_flushed.extend(flushed);
            self.prior_emitted_count = self.total_emitted_count();
            self.retired.extend(
                self.first
                    .take()
                    .into_iter()
                    .chain(self.rest.drain(..))
                    .map(RetiredLayerHls::from),
            );
        }
        if cmvs_ended {
            self.global.active_msdo = None;
            self.global.activated_global_lcrs.fill(None);
            self.global.cmvs_start_tu_index = None;
        }
        Ok(())
    }

    fn flush_all_outputs_and_invalidate(&mut self) -> Vec<usize> {
        let mut flushed = Vec::new();
        for layer in self.iter_mut() {
            flushed.extend(layer.scheduler.flush_all());
            layer.reference.invalidate_all();
        }
        flushed
    }

    pub(super) fn should_skip_pre_rap_frame(
        &self,
        xlayer: ExtendedLayerId,
        tu_has_rap: bool,
    ) -> bool {
        self.global.has_multistream_mode()
            && self.global.xlayer_declared_in_multistream(xlayer)
            && !self.contains(xlayer)
            && (self.first.is_some() || tu_has_rap)
    }

    pub(super) fn note_celu_order(
        &mut self,
        xlayer: ExtendedLayerId,
        offset: ByteOffset,
    ) -> Result<()> {
        if let Some(prev) = self.global.last_celu_xlayer
            && xlayer.get() < prev
        {
            return Err(malformed(
                offset,
                "7.3.7",
                format!(
                    "extended layer {} follows higher extended layer {} out of order",
                    xlayer.get(),
                    prev
                ),
            ));
        }
        self.global.last_celu_xlayer = Some(xlayer.get());
        Ok(())
    }

    pub(super) fn check_tu_doh(
        &mut self,
        xlayer: ExtendedLayerId,
        order_hint_bits: u8,
        order_hint: Option<u32>,
        is_output: bool,
        offset: ByteOffset,
    ) -> Result<()> {
        if !self.global.any_doh_constrained() {
            return Ok(());
        }
        match self.global.tu_order_hint_bits {
            Some(prev_bits) if prev_bits != order_hint_bits => {
                return Err(malformed(
                    offset,
                    "7.3.7",
                    format!(
                        "DOH constraint requires equal OrderHintBits in a temporal unit (got {order_hint_bits}, expected {prev_bits})"
                    ),
                ));
            }
            None => self.global.tu_order_hint_bits = Some(order_hint_bits),
            _ => {}
        }
        if is_output && let Some(hint) = order_hint {
            match self.global.tu_output_order_hint {
                Some((prev_xlayer, prev_hint)) if prev_xlayer != xlayer && prev_hint != hint => {
                    return Err(malformed(
                        offset,
                        "7.3.7",
                        format!(
                            "DOH constraint requires equal OrderHint across extended layers in a temporal unit (got {hint}, expected {prev_hint})"
                        ),
                    ));
                }
                None => self.global.tu_output_order_hint = Some((xlayer, hint)),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn layer_atlases(&self, id: ExtendedLayerId) -> Option<&LayerAtlases> {
        self.layer_atlases.get(usize::from(id.get()))?.as_ref()
    }

    fn observe_layer_atlas(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let id = envelope.header.extended_layer_id;
        match envelope.header.obu_type {
            ObuType::AtlasSegment => scope_mut(&mut self.layer_atlases, id)
                .layouts
                .observe(envelope)?,
            ObuType::LayerConfigurationRecord => {
                if let LayerConfigurationRecord::Local(record) = parse_validated_lcr(envelope)? {
                    scope_mut(&mut self.layer_atlases, id)
                        .observe_local_lcr(envelope.offset, record)?;
                }
            }
            ObuType::SequenceHeader
                if self.layer_atlases(id).is_some()
                    || self.global.global_lcrs.iter().any(Option::is_some) =>
            {
                let seq = parse_sequence(envelope)?.general;
                if self.layer_atlases(id).is_some()
                    || self.global.global_lcrs[usize::from(seq.seq_lcr_id.get())].is_some()
                {
                    scope_mut(&mut self.layer_atlases, id).bind(
                        seq.seq_header_id,
                        seq.seq_lcr_id.get(),
                        &self.global.global_lcrs,
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn note_pre_rap_local_lcr(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        xlayer: ExtendedLayerId,
    ) {
        if let Some(retired) = self
            .retired
            .iter_mut()
            .find(|layer| layer.xlayer_id == xlayer)
            && obus.iter().any(|env| {
                env.header.extended_layer_id == xlayer
                    && env.header.obu_type == ObuType::LayerConfigurationRecord
            })
        {
            retired.output_effect_state.note_local_lcr();
        }
    }

    pub(super) fn observe_other_non_global_prefix(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        skip_xlayer: ExtendedLayerId,
    ) -> Result<()> {
        for envelope in obus {
            let xlayer = envelope.header.extended_layer_id;
            if xlayer.is_global() || xlayer == skip_xlayer {
                continue;
            }
            self.observe_local_prefix(*envelope)?;
        }
        Ok(())
    }

    fn observe_local_prefix(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        let xlayer = envelope.header.extended_layer_id;
        let hls = if let Some(layer) = self.get_mut(xlayer) {
            Some((
                &layer.sequence,
                &mut layer.output_effect_state,
                &mut layer.film_grain_slots,
            ))
        } else {
            self.retired
                .iter_mut()
                .find(|layer| layer.xlayer_id == xlayer)
                .map(|layer| {
                    (
                        &layer.sequence,
                        &mut layer.output_effect_state,
                        &mut layer.film_grain_slots,
                    )
                })
        };
        if let Some((sequence, effects, film_grain)) = hls {
            effects.observe_prefix_obu(envelope, sequence)?;
            if envelope.header.obu_type == ObuType::FilmGrain {
                film_grain.update_from_obu(envelope, sequence)?;
            }
        }
        Ok(())
    }

    pub(super) fn apply_non_global_prefix(
        obus: &[ObuEnvelope<'_>],
        target_xlayer: ExtendedLayerId,
        sequence: &SequenceHeader,
        effects: &mut OutputEffectState,
        fg_slots: &mut FilmGrainSlots,
    ) -> Result<()> {
        for envelope in obus {
            if envelope.header.extended_layer_id != target_xlayer {
                continue;
            }
            effects.observe_prefix_obu(*envelope, sequence)?;
            if envelope.header.obu_type == ObuType::FilmGrain {
                fg_slots.update_from_obu(*envelope, sequence)?;
            }
        }
        Ok(())
    }

    pub(super) fn observe_skipped(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        tu_has_rap: bool,
        tu_has_clk: bool,
        global_lcr_activation: Option<ByteOffset>,
    ) -> Result<()> {
        self.observe_global_obus(obus, true, tu_has_rap, tu_has_clk, global_lcr_activation)?;
        self.observe_other_non_global_prefix(obus, GLOBAL_XLAYER_ID)
    }

    pub(super) fn finish_frame(
        &mut self,
        envelope: ObuEnvelope<'_>,
        activate: bool,
        atlas_mlayer: EmbeddedLayerId,
    ) -> Result<super::FrameOutputEffects> {
        let xlayer = envelope.header.extended_layer_id;
        let mlayer = envelope.header.embedded_layer_id;
        let layer = self
            .first
            .iter_mut()
            .chain(&mut self.rest)
            .find(|layer| layer.xlayer_id == xlayer)
            .ok_or_else(|| {
                unsupported_feature_at(
                    "missing_random_access_frame",
                    envelope.offset,
                    "frame requires an active extended layer",
                    "7.4",
                )
            })?;
        let effects = &mut layer.output_effect_state;
        let mut output = effects.finish_frame(mlayer);
        let msdo_stream = self
            .global
            .active_msdo
            .as_ref()
            .is_some_and(|msdo| msdo.ceilings.contains_key(&xlayer.get()));
        if let Some(atlas) = self
            .layer_atlases
            .get_mut(usize::from(xlayer.get()))
            .and_then(Option::as_mut)
        {
            if activate {
                atlas.activate(layer.sequence.general.seq_header_id);
            }
            atlas.snapshot(
                &self.global.atlases,
                msdo_stream,
                xlayer,
                atlas_mlayer,
                &mut output.atlas,
            )?;
        } else if msdo_stream {
            self.global
                .atlases
                .msdo_associations(xlayer, &mut output.atlas);
        }
        Ok(output)
    }

    pub(super) fn observe_suffix_for_xlayer(
        &mut self,
        obus: &[ObuEnvelope<'_>],
        target_xlayer: ExtendedLayerId,
    ) -> Result<()> {
        for envelope in obus {
            let xlayer = envelope.header.extended_layer_id;
            if xlayer.is_global() {
                match envelope.header.obu_type {
                    ObuType::Padding => {}
                    ObuType::MetadataShort | ObuType::MetadataGroup => {
                        parse_metadata_obu(*envelope, true, |action| {
                            self.global.record_metadata_action(&action);
                            for layer in self.iter_mut() {
                                layer
                                    .output_effect_state
                                    .apply_metadata_action(GLOBAL_XLAYER_ID, action.clone());
                            }
                        })?;
                    }
                    _ => {
                        if let Some(layer) = self.get_mut(target_xlayer) {
                            layer.output_effect_state.observe_suffix_obu(*envelope)?;
                        }
                    }
                }
            } else if xlayer == target_xlayer
                && let Some(layer) = self.get_mut(target_xlayer)
            {
                layer.output_effect_state.observe_suffix_obu(*envelope)?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_xlayer_lcr(
        &mut self,
        xlayer: ExtendedLayerId,
        sequence: &SequenceHeader,
        offset: ByteOffset,
        initializing_new_layer: bool,
    ) -> Result<()> {
        let will_have_multiple = if initializing_new_layer && !self.contains(xlayer) {
            self.first.is_some()
        } else {
            !self.rest.is_empty()
        };
        let has_msdo = self.global.active_msdo.is_some();
        if will_have_multiple && !has_msdo {
            let requires_msdo = |seq: &SequenceHeader| {
                matches!(
                    seq.general.seq_profile_idc,
                    ProfileIdc::Main420Ip0
                        | ProfileIdc::Main420Ip1
                        | ProfileIdc::Main422Ip1
                        | ProfileIdc::Main444Ip1
                )
            };
            if requires_msdo(sequence) || self.iter().any(|layer| requires_msdo(&layer.sequence)) {
                return Err(malformed(
                    offset,
                    "A.2",
                    "multi-extended-layer IOP 0/1 stream requires an active OBU_MSDO".to_owned(),
                ));
            }
        }

        if let Some(active) = &self.global.active_msdo {
            let Some(&ceiling) = active.ceilings.get(&xlayer.get()) else {
                return Err(malformed(
                    offset,
                    "6.6",
                    format!(
                        "extended layer {} is not listed in active OBU_MSDO sub_xlayer_id",
                        xlayer.get()
                    ),
                ));
            };
            let seq_tier = u8::from(sequence.general.seq_tier == Tier::High);
            if sequence.general.seq_profile_idc.get() > ceiling.profile
                || sequence.general.seq_level_idx.get() > ceiling.level
                || seq_tier > ceiling.tier
            {
                return Err(malformed(
                    offset,
                    "6.6",
                    format!(
                        "extended layer {} sequence PTL exceeds OBU_MSDO ceiling",
                        xlayer.get()
                    ),
                ));
            }
            if !sequence.general.monotonic_output_order_flag
                && !active.msdo.multistream_doh_constraint_flag
            {
                return Err(malformed(
                    offset,
                    "6.6",
                    format!(
                        "extended layer {} disables monotonic output order without multistream_doh_constraint_flag",
                        xlayer.get()
                    ),
                ));
            }
        }

        let binding = self
            .layer_atlases(xlayer)
            .and_then(|atlas| atlas.binding(sequence.general.seq_header_id));
        let has_local_lcr = binding.is_some_and(|record| record.local_parent.is_some());
        let lcr_id = sequence.general.seq_lcr_id.get();
        if lcr_id != 0 {
            let allow_local = !will_have_multiple || has_msdo;
            let in_local = allow_local && has_local_lcr;
            let global = if has_local_lcr && !allow_local {
                self.global.global_lcrs[usize::from(lcr_id)].as_ref()
            } else {
                binding.and_then(|record| record.global.as_ref())
            };
            let has_global_record = global.is_some();
            let in_global =
                global.is_some_and(|(_, _, info)| info.xlayer_map & (1 << xlayer.get()) != 0);
            let deferred_annex_a = !will_have_multiple
                && !has_msdo
                && !has_global_record
                && !in_local
                && sequence.general.max_mlayer_id.get() > 0;
            if !in_global && !in_local && !deferred_annex_a {
                return Err(malformed(
                    offset,
                    "6.4.1",
                    format!(
                        "extended layer {} references global LCR {} which is missing or omits bit {}",
                        xlayer.get(),
                        lcr_id,
                        xlayer.get()
                    ),
                ));
            }
            if !in_local
                && in_global
                && let Some((obs_tu, _, info)) = global
            {
                if !sequence.general.monotonic_output_order_flag
                    && !info.doh_constraint_flag
                    && will_have_multiple
                {
                    return Err(malformed(
                        offset,
                        "6.6",
                        format!(
                            "extended layer {} disables monotonic output order without lcr_doh_constraint_flag",
                            xlayer.get()
                        ),
                    ));
                }
                if let Some(active) = &self.global.active_msdo
                    && self
                        .global
                        .cmvs_start_tu_index
                        .is_some_and(|start| *obs_tu >= start)
                {
                    check_msdo_global_lcr_agreement(&active.msdo, info, offset)?;
                }
                self.global.activated_global_lcrs[usize::from(lcr_id)] = Some(Arc::clone(info));
            }
        } else if will_have_multiple && !has_msdo {
            return Err(malformed(
                offset,
                "6.4.1",
                format!(
                    "extended layer {} requires seq_lcr_id != 0 in a multi-xlayer stream without OBU_MSDO",
                    xlayer.get()
                ),
            ));
        } else if has_msdo
            && sequence.general.max_mlayer_id.get() > 0
            && !has_local_lcr
            && will_have_multiple
        {
            return Err(malformed(
                offset,
                "A.2",
                format!(
                    "extended layer {} with embedded layers requires an active local or global LCR",
                    xlayer.get()
                ),
            ));
        }

        if will_have_multiple && initializing_new_layer && !has_msdo {
            for existing in self.iter() {
                let existing_lcr_id = existing.sequence.general.seq_lcr_id.get();
                if existing_lcr_id == 0
                    || !self
                        .global
                        .global_lcr_has_xlayer(existing_lcr_id, existing.xlayer_id)
                {
                    return Err(malformed(
                        offset,
                        "6.4.1",
                        format!(
                            "active extended layer {} must be declared in an active global LCR before another extended layer activates",
                            existing.xlayer_id.get()
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn retains(&self, frame_index: usize) -> bool {
        self.iter().any(|layer| {
            layer.reference.retains(frame_index) || layer.scheduler.retains(frame_index)
        })
    }

    pub(super) fn forget(&mut self, frame_index: usize) {
        for layer in self.iter_mut() {
            layer.scheduler.forget(frame_index);
        }
    }

    pub(super) fn total_emitted_count(&self) -> usize {
        self.prior_emitted_count
            + self
                .iter()
                .map(|layer| layer.scheduler.emitted_count)
                .sum::<usize>()
    }

    pub(super) fn flush_layer(&mut self, layer_pos: usize) -> Vec<usize> {
        if layer_pos == 0 {
            self.first
                .as_mut()
                .map_or_else(Vec::new, |layer| layer.scheduler.flush_all())
        } else {
            self.rest[layer_pos - 1].scheduler.flush_all()
        }
    }
}
