// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Plan-only AV2 stream traversal over already parsed `splot-core` structures.
//!
//! Feature tracking: `DECODE-STREAM-STATE-PLANNER`.

use core::fmt;

use splot_core::annexb::{AnnexBObuCursor, ObuEnvelope, PartialParse};
use splot_core::bitio::BitReader;
use splot_core::headers::sequence::{
    MLayerDependencyMap, TLayerDependencyMap, parse_sequence_header_general,
};
use splot_core::ivf::{IvfError, IvfWarning};
use splot_core::obu::ObuHeader;
use splot_core::span::ByteOffset;
use splot_core::stream::{BitstreamFormat, ParsedBitstream};
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};

use crate::bitstream::byte_stream::SourceBytes;
pub use crate::bitstream::ops_plan::OperatingPointSelection;
use crate::bitstream::ops_plan::OpsPlanTracker;
use crate::error::{DecodeError, Result};
use crate::{DecodeLimitName, DecodeOptions, UNSUPPORTED_FEATURE_RULE_ID};

/// Feature ID for parsed stream planning.
pub const DECODE_STREAM_STATE_FEATURE_ID: &str = "DECODE-STREAM-STATE-PLANNER";

/// Parsed stream input for [`crate::DecodeContext::plan_stream`].
///
/// This type deliberately holds parser output, not raw bytes. Raw-byte planning
/// performs bounded pre-parse traversal before reusing this parsed input shape.
#[derive(Clone, Copy, Debug)]
pub struct DecodeStreamInput<'a> {
    parsed: &'a ParsedBitstream<'a>,
    input_len_bytes: u64,
}

impl<'a> DecodeStreamInput<'a> {
    /// Builds a stream-planner input from already parsed `splot-core` output.
    #[must_use]
    pub const fn new(parsed: &'a ParsedBitstream<'a>, input_len_bytes: u64) -> Self {
        Self {
            parsed,
            input_len_bytes,
        }
    }

    /// Already parsed stream/container output.
    #[must_use]
    pub const fn parsed(self) -> &'a ParsedBitstream<'a> {
        self.parsed
    }

    /// Original input length in bytes, supplied by the caller.
    #[must_use]
    pub const fn input_len_bytes(self) -> u64 {
        self.input_len_bytes
    }
}

/// Caller-selected temporal enhancement layers to decode and emit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TemporalLayerSelection {
    /// Decode and emit all temporal layers present in the stream (up to `max_tlayer_id`).
    #[default]
    All,
    /// Decode and emit temporal layers `0..=target` along with their transitive
    /// dependencies in `TLayerDependencyMap`.
    Target(TemporalLayerId),
    /// Decode all transitive temporal dependencies required to emit only the layers
    /// whose bit (`1 << tlayer_id`, within `1..=0b1111`) is set in this mask.
    OutputMask(u8),
}

impl TemporalLayerSelection {
    /// Selects only the temporal base layer (`obu_tlayer_id == 0`).
    pub const BASE_ONLY: Self = Self::Target(TemporalLayerId::BASE);

    pub(crate) const fn validate(self) -> Result<()> {
        match self {
            Self::All => Ok(()),
            Self::Target(target) => {
                if target.get() > 3 {
                    Err(DecodeError::InvalidTemporalSelection {
                        reason: "temporal layer target must be within 0..=3",
                    })
                } else {
                    Ok(())
                }
            }
            Self::OutputMask(mask) => {
                if mask == 0 {
                    Err(DecodeError::InvalidTemporalSelection {
                        reason: "temporal output mask must select at least one temporal layer",
                    })
                } else if mask > 0x0F {
                    Err(DecodeError::InvalidTemporalSelection {
                        reason: "temporal output mask must only set bits within 0..=3",
                    })
                } else {
                    Ok(())
                }
            }
        }
    }

    pub(crate) const fn requested_output_mask(self) -> u8 {
        match self {
            Self::All => 0x0F,
            Self::Target(target) => {
                let id = target.get();
                if id >= 3 { 0x0F } else { (1u8 << (id + 1)) - 1 }
            }
            Self::OutputMask(mask) => mask & 0x0F,
        }
    }
}

/// Caller-selected embedded enhancement layers to decode and emit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EmbeddedLayerSelection {
    /// Decode and emit all embedded layers present in the stream (up to `max_mlayer_id`).
    #[default]
    All,
    /// Decode transitive dependencies required to emit only embedded layer `target`.
    Target(EmbeddedLayerId),
    /// Decode all transitive embedded dependencies required to emit only the layers
    /// whose bit (`1 << mlayer_id`, within `1..=0xFF`) is set in this mask.
    OutputMask(u8),
}

impl EmbeddedLayerSelection {
    /// Selects only the embedded base layer (`obu_mlayer_id == 0`).
    pub const BASE_ONLY: Self = Self::Target(EmbeddedLayerId::BASE);

    pub(crate) const fn validate(self) -> Result<()> {
        match self {
            Self::Target(target) if target.get() > 7 => {
                Err(DecodeError::InvalidEmbeddedSelection {
                    reason: "embedded layer target must be within 0..=7",
                })
            }
            Self::OutputMask(0) => Err(DecodeError::InvalidEmbeddedSelection {
                reason: "embedded output mask must select at least one embedded layer",
            }),
            _ => Ok(()),
        }
    }

    pub(crate) const fn requested_output_mask(self) -> u8 {
        match self {
            Self::All => u8::MAX,
            Self::Target(target) => {
                let id = target.get();
                if id > 7 { 0 } else { 1u8 << id }
            }
            Self::OutputMask(mask) => mask,
        }
    }
}

/// Caller-selected extended enhancement layers to decode and emit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExtendedLayerSelection {
    /// Decode and emit all active extended layers (`0..=30`) in the stream.
    #[default]
    All,
    /// Decode and emit only extended layer `target` (`0..=30`) alongside global state.
    Target(ExtendedLayerId),
    /// Decode and emit only the extended layers whose bit (`1u32 << xlayer_id`,
    /// within bits `0..=30`) is set in this mask.
    OutputMask(u32),
}

impl ExtendedLayerSelection {
    pub(crate) const fn validate(self) -> Result<()> {
        match self {
            Self::Target(target) if target.get() > 30 => {
                Err(DecodeError::InvalidExtendedSelection {
                    reason: "extended layer target must be within 0..=30",
                })
            }
            Self::OutputMask(0) => Err(DecodeError::InvalidExtendedSelection {
                reason: "extended output mask must select at least one extended layer",
            }),
            Self::OutputMask(mask) if (mask & (1u32 << 31)) != 0 => {
                Err(DecodeError::InvalidExtendedSelection {
                    reason: "extended output mask must not select global xlayer 31",
                })
            }
            _ => Ok(()),
        }
    }

    /// Returns `true` when `xlayer` (`0..=30`) is selected for decode and emission.
    #[must_use]
    pub const fn is_xlayer_selected(self, xlayer: ExtendedLayerId) -> bool {
        let id = xlayer.get();
        if id > 30 {
            return false;
        }
        match self {
            Self::All => true,
            Self::Target(target) => target.get() == id,
            Self::OutputMask(mask) => (mask & (1u32 << id)) != 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerDependencyFilter {
    temporal: TemporalLayerSelection,
    embedded: EmbeddedLayerSelection,
    max_mlayer_id: EmbeddedLayerId,
    mlayer_dependency_map: MLayerDependencyMap,
    max_tlayer_id: TemporalLayerId,
    tlayer_dependency_map: TLayerDependencyMap,
    decode_tlayer_masks: [u8; 8],
    output_tlayer_masks: [u8; 8],
}

impl LayerDependencyFilter {
    fn default_output_tlayer_masks(
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
    ) -> [u8; 8] {
        let output_m_mask = embedded.requested_output_mask();
        let output_t_mask = temporal.requested_output_mask();
        let mut masks = [0u8; 8];
        for (m, slot) in masks.iter_mut().enumerate() {
            if (output_m_mask & (1u8 << m)) != 0 {
                *slot = output_t_mask;
            }
        }
        masks
    }

    fn new(temporal: TemporalLayerSelection, embedded: EmbeddedLayerSelection) -> Self {
        let max_mlayer_id = EmbeddedLayerId::from_bits(7);
        let max_tlayer_id = TemporalLayerId::from_bits(3);
        let mlayer_dependency_map = MLayerDependencyMap::default_for(max_mlayer_id);
        let tlayer_dependency_map = TLayerDependencyMap::default_for(max_tlayer_id, max_mlayer_id);
        let output_tlayer_masks = Self::default_output_tlayer_masks(temporal, embedded);
        let mut filter = Self {
            temporal,
            embedded,
            max_mlayer_id,
            mlayer_dependency_map,
            max_tlayer_id,
            tlayer_dependency_map,
            decode_tlayer_masks: [0; 8],
            output_tlayer_masks,
        };
        filter.recompute_masks();
        filter
    }

    pub(crate) fn set_output_tlayer_masks(&mut self, masks: [u8; 8]) {
        self.output_tlayer_masks = masks;
        self.recompute_masks();
    }

    fn reset_default_output_masks(&mut self) {
        self.output_tlayer_masks = Self::default_output_tlayer_masks(self.temporal, self.embedded);
        self.recompute_masks();
    }

    fn apply_sequence_general(
        &mut self,
        general: &splot_core::headers::sequence::SequenceHeaderGeneral,
    ) {
        self.max_mlayer_id = general.max_mlayer_id;
        self.mlayer_dependency_map = general.mlayer_dependency_map;
        self.max_tlayer_id = general.max_tlayer_id;
        self.tlayer_dependency_map = general.tlayer_dependency_map;
        self.recompute_masks();
    }

    fn recompute_masks(&mut self) {
        let mut decode_masks = self.output_tlayer_masks;
        let max_m = self.max_mlayer_id.get().min(7);
        let max_t = self.max_tlayer_id.get().min(3);
        loop {
            let mut changed = false;
            for curr_m in 0..=max_m {
                let m_mask = decode_masks[usize::from(curr_m)];
                if m_mask == 0 {
                    continue;
                }
                let curr_m_id = EmbeddedLayerId::from_bits(curr_m);
                for curr_t in 0..=max_t {
                    if (m_mask & (1u8 << curr_t)) == 0 {
                        continue;
                    }
                    let curr_t_id = TemporalLayerId::from_bits(curr_t);
                    for ref_m in 0..=curr_m {
                        if !self
                            .mlayer_dependency_map
                            .depends_on(curr_m_id, EmbeddedLayerId::from_bits(ref_m))
                        {
                            continue;
                        }
                        for ref_t in 0..=max_t {
                            let ref_bit = 1u8 << ref_t;
                            if (decode_masks[usize::from(ref_m)] & ref_bit) == 0
                                && self.tlayer_dependency_map.depends_on(
                                    curr_m_id,
                                    curr_t_id,
                                    TemporalLayerId::from_bits(ref_t),
                                )
                            {
                                decode_masks[usize::from(ref_m)] |= ref_bit;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        self.decode_tlayer_masks = decode_masks;
    }

    const fn is_layer_selected(&self, mlayer: EmbeddedLayerId, tlayer: TemporalLayerId) -> bool {
        let m = mlayer.get();
        let t = tlayer.get();
        m <= 7 && t <= 3 && (self.decode_tlayer_masks[m as usize] & (1u8 << t)) != 0
    }

    const fn is_layer_output(&self, mlayer: EmbeddedLayerId, tlayer: TemporalLayerId) -> bool {
        let m = mlayer.get();
        let t = tlayer.get();
        m <= 7 && t <= 3 && (self.output_tlayer_masks[m as usize] & (1u8 << t)) != 0
    }
}

/// Active temporal, embedded, and extended layer decode/output bitmasks derived
/// from caller selections and each extended layer's active sequence header.
#[derive(Clone, Debug)]
pub(crate) struct ActiveLayerFilter {
    extended: ExtendedLayerSelection,
    default_filter: LayerDependencyFilter,
    first: Option<(ExtendedLayerId, LayerDependencyFilter)>,
    rest: Vec<(ExtendedLayerId, LayerDependencyFilter)>,
    ops: Option<Box<OpsPlanTracker>>,
}

impl ActiveLayerFilter {
    #[cfg(test)]
    pub(crate) fn new(temporal: TemporalLayerSelection, embedded: EmbeddedLayerSelection) -> Self {
        Self::new_with_extended(temporal, embedded, ExtendedLayerSelection::All)
    }

    pub(crate) fn new_with_extended(
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
        extended: ExtendedLayerSelection,
    ) -> Self {
        Self::new_with_selections(temporal, embedded, extended, OperatingPointSelection::None)
    }

    pub(crate) fn new_with_selections(
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
        extended: ExtendedLayerSelection,
        operating_point: OperatingPointSelection,
    ) -> Self {
        Self {
            extended,
            default_filter: LayerDependencyFilter::new(temporal, embedded),
            first: None,
            rest: Vec::new(),
            ops: (!operating_point.is_none())
                .then(|| Box::new(OpsPlanTracker::new(operating_point))),
        }
    }

    pub(crate) fn from_options(options: &DecodeOptions) -> Self {
        Self::new_with_selections(
            options.temporal_layer_selection(),
            options.embedded_layer_selection(),
            options.extended_layer_selection(),
            options.operating_point_selection(),
        )
    }

    pub(crate) const fn selection(&self) -> TemporalLayerSelection {
        self.default_filter.temporal
    }

    pub(crate) const fn embedded_selection(&self) -> EmbeddedLayerSelection {
        self.default_filter.embedded
    }

    pub(crate) const fn extended_selection(&self) -> ExtendedLayerSelection {
        self.extended
    }

    pub(crate) const fn extended_filtering_disabled(&self) -> bool {
        matches!(self.extended, ExtendedLayerSelection::All) && self.ops.is_none()
    }

    pub(crate) fn is_xlayer_selected(&self, xlayer: ExtendedLayerId) -> bool {
        if let Some(ops) = &self.ops {
            ops.is_xlayer_selected(xlayer)
        } else {
            self.extended.is_xlayer_selected(xlayer)
        }
    }

    fn filter_for(&self, xlayer: ExtendedLayerId) -> &LayerDependencyFilter {
        if let Some((first_id, filter)) = &self.first {
            if *first_id == xlayer {
                return filter;
            }
            if let Some((_, filter)) = self.rest.iter().find(|(id, _)| *id == xlayer) {
                return filter;
            }
        }
        &self.default_filter
    }

    fn filter_for_mut(&mut self, xlayer: ExtendedLayerId) -> &mut LayerDependencyFilter {
        let default = self.default_filter;
        let first = self.first.get_or_insert((xlayer, default));
        if first.0 == xlayer {
            return &mut first.1;
        }
        if let Some(index) = self.rest.iter().position(|(id, _)| *id == xlayer) {
            return &mut self.rest[index].1;
        }
        self.rest.push((xlayer, default));
        let len = self.rest.len();
        &mut self.rest[len - 1].1
    }

    pub(crate) fn observe_envelope(&mut self, envelope: ObuEnvelope<'_>) -> Result<()> {
        if envelope.header.obu_type == ObuType::OperatingPointSet
            && envelope.header.temporal_layer_id == TemporalLayerId::BASE
            && envelope.header.embedded_layer_id == EmbeddedLayerId::BASE
            && self.ops.is_some()
        {
            if let Some(ops) = &mut self.ops {
                ops.apply_obu(envelope)?;
            }
            self.sync_ops_masks();
            return Ok(());
        }
        if envelope.header.obu_type != ObuType::SequenceHeader
            || envelope.header.extended_layer_id == GLOBAL_XLAYER_ID
            || envelope.header.temporal_layer_id != TemporalLayerId::BASE
            || envelope.header.embedded_layer_id != EmbeddedLayerId::BASE
        {
            return Ok(());
        }
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        if let Ok(general) = parse_sequence_header_general(&mut reader) {
            let xlayer = envelope.header.extended_layer_id;
            let masks = self.ops.as_ref().and_then(|ops| {
                ops.resolved_entries()
                    .iter()
                    .find(|(id, _)| *id == xlayer)
                    .map(|(_, masks)| *masks)
            });
            let filter = self.filter_for_mut(xlayer);
            if let Some(masks) = masks {
                filter.set_output_tlayer_masks(masks);
            }
            filter.apply_sequence_general(&general);
        }
        Ok(())
    }

    fn sync_ops_masks(&mut self) {
        let Some(ops) = self.ops.as_deref() else {
            return;
        };
        if !ops.is_resolved() {
            if let Some((_, filter)) = &mut self.first {
                filter.reset_default_output_masks();
            }
            for (_, filter) in &mut self.rest {
                filter.reset_default_output_masks();
            }
            return;
        }
        for (id, filter) in self.first.iter_mut().chain(self.rest.iter_mut()) {
            ops.apply_masks_to_filter(*id, filter);
        }
        let entries: Vec<(ExtendedLayerId, [u8; 8])> = ops.resolved_entries().to_vec();
        for (xlayer, masks) in entries {
            self.filter_for_mut(xlayer).set_output_tlayer_masks(masks);
        }
    }

    pub(crate) fn check_operating_point_for_frame(
        &mut self,
        xlayer: ExtendedLayerId,
        offset: ByteOffset,
    ) -> Result<()> {
        let Some(ops) = &mut self.ops else {
            return Ok(());
        };
        if ops.should_fail_unavailable_frame(xlayer) {
            return Err(ops.unavailable_error(offset));
        }
        Ok(())
    }

    pub(crate) fn ensure_operating_point_at_eof(
        &self,
        saw_frame: bool,
        offset: ByteOffset,
    ) -> Result<()> {
        let Some(ops) = &self.ops else {
            return Ok(());
        };
        if saw_frame && !ops.ever_resolved() {
            return Err(ops.unavailable_error(offset));
        }
        Ok(())
    }

    #[cfg(test)]
    const fn default_or_first(&self) -> &LayerDependencyFilter {
        match &self.first {
            Some((_, filter)) => filter,
            None => &self.default_filter,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_layer_selected(
        &self,
        mlayer: EmbeddedLayerId,
        tlayer: TemporalLayerId,
    ) -> bool {
        self.default_or_first().is_layer_selected(mlayer, tlayer)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) const fn is_layer_output(
        &self,
        mlayer: EmbeddedLayerId,
        tlayer: TemporalLayerId,
    ) -> bool {
        self.default_or_first().is_layer_output(mlayer, tlayer)
    }

    #[must_use]
    pub(crate) fn is_layer_selected_for(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        tlayer: TemporalLayerId,
    ) -> bool {
        self.filter_for(xlayer).is_layer_selected(mlayer, tlayer)
    }

    #[must_use]
    pub(crate) fn is_layer_output_for(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        tlayer: TemporalLayerId,
    ) -> bool {
        self.is_xlayer_selected(xlayer) && self.filter_for(xlayer).is_layer_output(mlayer, tlayer)
    }

    #[must_use]
    pub(crate) fn is_obu_selected(&self, header: ObuHeader) -> bool {
        if header.obu_type.is_reserved() || matches!(header.obu_type, ObuType::TemporalDelimiter) {
            return true;
        }
        if header.extended_layer_id == GLOBAL_XLAYER_ID {
            if !header.obu_type.permits_global_xlayer() {
                return false;
            }
        } else if !self.is_xlayer_selected(header.extended_layer_id) {
            return false;
        }
        if header.obu_type.requires_base_temporal_and_embedded_layer() {
            true
        } else {
            self.is_layer_selected_for(
                header.extended_layer_id,
                header.embedded_layer_id,
                header.temporal_layer_id,
            )
        }
    }
}

/// The decoded layer selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLayerSelection {
    temporal_layer_id: TemporalLayerId,
    embedded_layer_id: EmbeddedLayerId,
    extended_layer_id: ExtendedLayerId,
    temporal_selection: TemporalLayerSelection,
    embedded_selection: EmbeddedLayerSelection,
    extended_selection: ExtendedLayerSelection,
}

impl Default for DecodeLayerSelection {
    fn default() -> Self {
        Self {
            temporal_selection: TemporalLayerSelection::All,
            embedded_selection: EmbeddedLayerSelection::All,
            extended_selection: ExtendedLayerSelection::All,
            ..Self::BASE
        }
    }
}

impl DecodeLayerSelection {
    /// The base layer: temporal 0, embedded 0, extended 0.
    pub const BASE: Self = Self {
        temporal_layer_id: TemporalLayerId::from_bits(0),
        embedded_layer_id: EmbeddedLayerId::from_bits(0),
        extended_layer_id: ExtendedLayerId::from_bits(0),
        temporal_selection: TemporalLayerSelection::BASE_ONLY,
        embedded_selection: EmbeddedLayerSelection::BASE_ONLY,
        extended_selection: ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(0)),
    };

    /// Returns the planner's base-layer selection.
    #[must_use]
    pub const fn base() -> Self {
        Self::BASE
    }

    pub(crate) fn adopt_with_filter(
        selected: &mut Option<Self>,
        layer: ExtendedLayerId,
        filter: &ActiveLayerFilter,
    ) -> Self {
        if selected.is_none() && layer != GLOBAL_XLAYER_ID && filter.is_xlayer_selected(layer) {
            *selected = Some(
                Self {
                    extended_layer_id: layer,
                    ..Self::BASE
                }
                .with_temporal_selection(filter.selection())
                .with_embedded_selection(filter.embedded_selection())
                .with_extended_selection(filter.extended_selection()),
            );
        }
        selected.unwrap_or_else(|| {
            Self::default()
                .with_temporal_selection(filter.selection())
                .with_embedded_selection(filter.embedded_selection())
                .with_extended_selection(filter.extended_selection())
        })
    }

    /// The selection of a bitstream: the first selected non-global extended layer
    /// in `layers`, or the target / layer 0 when there is none.
    pub(crate) fn of_stream(
        mut layers: impl Iterator<Item = ExtendedLayerId>,
        temporal_selection: TemporalLayerSelection,
        embedded_selection: EmbeddedLayerSelection,
        extended_selection: ExtendedLayerSelection,
    ) -> Self {
        let extended_layer_id = layers
            .find(|&layer| {
                layer != GLOBAL_XLAYER_ID && extended_selection.is_xlayer_selected(layer)
            })
            .unwrap_or(match extended_selection {
                ExtendedLayerSelection::Target(target) => target,
                _ => Self::BASE.extended_layer_id,
            });
        Self {
            extended_layer_id,
            ..Self::BASE
        }
        .with_temporal_selection(temporal_selection)
        .with_embedded_selection(embedded_selection)
        .with_extended_selection(extended_selection)
    }

    /// Selected temporal layer id.
    #[must_use]
    pub const fn temporal_layer_id(self) -> TemporalLayerId {
        self.temporal_layer_id
    }

    /// Configured temporal-layer selection policy.
    #[must_use]
    pub const fn temporal_selection(self) -> TemporalLayerSelection {
        self.temporal_selection
    }

    /// Returns a copy with the configured temporal-layer selection policy.
    #[must_use]
    pub const fn with_temporal_selection(mut self, selection: TemporalLayerSelection) -> Self {
        self.temporal_selection = selection;
        self.temporal_layer_id = if let TemporalLayerSelection::Target(target) = selection {
            target
        } else {
            TemporalLayerId::BASE
        };
        self
    }

    /// Selected embedded layer id.
    #[must_use]
    pub const fn embedded_layer_id(self) -> EmbeddedLayerId {
        self.embedded_layer_id
    }

    /// Configured embedded-layer selection policy.
    #[must_use]
    pub const fn embedded_selection(self) -> EmbeddedLayerSelection {
        self.embedded_selection
    }

    /// Returns a copy with the configured embedded-layer selection policy.
    #[must_use]
    pub const fn with_embedded_selection(mut self, selection: EmbeddedLayerSelection) -> Self {
        self.embedded_selection = selection;
        self.embedded_layer_id = if let EmbeddedLayerSelection::Target(target) = selection {
            target
        } else {
            EmbeddedLayerId::BASE
        };
        self
    }

    /// Selected extended layer id.
    #[must_use]
    pub const fn extended_layer_id(self) -> ExtendedLayerId {
        self.extended_layer_id
    }

    /// Configured extended-layer selection policy.
    #[must_use]
    pub const fn extended_selection(self) -> ExtendedLayerSelection {
        self.extended_selection
    }

    /// Returns a copy with the configured extended-layer selection policy.
    #[must_use]
    pub const fn with_extended_selection(mut self, selection: ExtendedLayerSelection) -> Self {
        self.extended_selection = selection;
        self
    }
}

/// Deterministic plan over accepted parsed-stream OBU metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeStreamPlan {
    format: BitstreamFormat,
    selected_layer: DecodeLayerSelection,
    input_len_bytes: u64,
    obus: Vec<DecodePlannedObu>,
    obu_count: u64,
    frame_candidate_count: u64,
    source_warnings: Vec<DecodeSourceIssue>,
}

impl DecodeStreamPlan {
    /// Source container format.
    #[must_use]
    pub const fn format(&self) -> BitstreamFormat {
        self.format
    }

    /// Selected layer.
    #[must_use]
    pub const fn selected_layer(&self) -> DecodeLayerSelection {
        self.selected_layer
    }

    /// Caller-supplied input length in bytes.
    #[must_use]
    pub const fn input_len_bytes(&self) -> u64 {
        self.input_len_bytes
    }

    /// Count of planned OBUs.
    #[must_use]
    pub const fn obu_count(&self) -> u64 {
        self.obu_count
    }

    /// Count of accepted frame candidates.
    #[must_use]
    pub const fn frame_candidate_count(&self) -> u64 {
        self.frame_candidate_count
    }

    /// Planned OBUs in deterministic source order.
    pub fn obus(&self) -> core::slice::Iter<'_, DecodePlannedObu> {
        self.obus.iter()
    }

    /// Accepted frame candidates of any kind (key or inter), in deterministic
    /// source order. This is the per-frame decode order the multi-frame runtime
    /// walks (AV2 § 5.2.1, § 6.18).
    #[cfg(test)]
    pub fn frame_candidates_all(&self) -> impl Iterator<Item = &DecodePlannedObu> {
        self.obus.iter().filter(|obu| obu.role.is_frame_candidate())
    }

    /// Non-fatal source/container warnings carried into the plan.
    #[must_use]
    pub fn source_warnings(&self) -> &[DecodeSourceIssue] {
        &self.source_warnings
    }
}

/// Source kind for a planned OBU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeObuSourceKind {
    /// Raw Annex B input.
    AnnexB,
    /// IVF frame payload containing Annex B.
    Ivf,
}

/// IVF frame context for an OBU planned from an IVF payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeIvfFrameContext {
    frame_index: usize,
    frame_payload_offset: ByteOffset,
    frame_payload_size: u32,
    pts: u64,
}

impl DecodeIvfFrameContext {
    pub(crate) const fn new(
        frame_index: usize,
        frame_payload_offset: ByteOffset,
        frame_payload_size: u32,
        pts: u64,
    ) -> Self {
        Self {
            frame_index,
            frame_payload_offset,
            frame_payload_size,
            pts,
        }
    }

    /// Zero-based IVF frame index.
    #[must_use]
    pub const fn frame_index(self) -> usize {
        self.frame_index
    }

    /// Absolute offset of the IVF frame payload.
    #[must_use]
    pub const fn frame_payload_offset(self) -> ByteOffset {
        self.frame_payload_offset
    }

    /// Declared IVF frame payload size.
    #[must_use]
    pub const fn frame_payload_size(self) -> u32 {
        self.frame_payload_size
    }

    /// IVF presentation timestamp metadata.
    #[must_use]
    pub const fn pts(self) -> u64 {
        self.pts
    }
}

/// Role assigned to an accepted planned OBU.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodePlannedObuRole {
    /// Ordering/global marker accepted by the planner.
    Global,
    /// OBU retained for source traversal but not selected for base temporal-layer
    /// extraction (AV2 Annex F.3.2).
    UnselectedLayer,
    /// State OBU for the selected base layer.
    SelectedLayerState,
    /// Key-frame candidate for the decode stage (AV2 § 5.2.1).
    FrameCandidate,
    /// A non-first tile-group OBU that continues the preceding coded frame.
    /// Continuations are retained in source order but do not consume a frame
    /// decode slot of their own (AV2 § 7.3.3 / § 7.3.4).
    FrameContinuation,
    /// Non-key frame candidate admitted so the multi-frame runtime can process it.
    InterFrameCandidate,
}

impl DecodePlannedObuRole {
    /// Whether this role is a frame candidate (key or inter) the runtime decodes
    /// into an output frame, as opposed to a global marker or layer-state OBU.
    #[must_use]
    pub const fn is_frame_candidate(self) -> bool {
        matches!(self, Self::FrameCandidate | Self::InterFrameCandidate)
    }

    /// Whether this role carries tile data for an already-open coded frame.
    #[must_use]
    pub const fn is_frame_continuation(self) -> bool {
        matches!(self, Self::FrameContinuation)
    }
}

/// One accepted OBU in a parsed stream plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodePlannedObu {
    index: u64,
    source_kind: DecodeObuSourceKind,
    ivf_frame: Option<DecodeIvfFrameContext>,
    offset: ByteOffset,
    size: u32,
    payload_len: u64,
    header: ObuHeader,
    role: DecodePlannedObuRole,
}

impl DecodePlannedObu {
    const fn new(
        index: u64,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        envelope: ObuEnvelope<'_>,
        role: DecodePlannedObuRole,
    ) -> Self {
        Self {
            index,
            source_kind,
            ivf_frame,
            offset: envelope.offset,
            size: envelope.size,
            payload_len: envelope.payload.len() as u64,
            header: envelope.header,
            role,
        }
    }

    fn end(&self) -> u64 {
        self.offset.get().saturating_add(u64::from(self.size))
    }

    /// Zero-based OBU index in the plan.
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Source kind for this OBU.
    #[must_use]
    pub const fn source_kind(&self) -> DecodeObuSourceKind {
        self.source_kind
    }

    /// IVF context when the OBU came from an IVF frame payload.
    #[must_use]
    pub const fn ivf_frame(&self) -> Option<DecodeIvfFrameContext> {
        self.ivf_frame
    }

    /// Absolute byte offset of the OBU header.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// Declared OBU size in bytes, including the OBU header.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// OBU payload length in bytes.
    #[must_use]
    pub const fn payload_len(&self) -> u64 {
        self.payload_len
    }

    /// Parsed OBU header.
    #[must_use]
    pub const fn header(&self) -> ObuHeader {
        self.header
    }

    /// Parsed OBU type.
    #[must_use]
    pub const fn obu_type(&self) -> ObuType {
        self.header.obu_type
    }

    /// Planner role for this OBU.
    #[must_use]
    pub const fn role(&self) -> DecodePlannedObuRole {
        self.role
    }
}

/// Source issue category recorded by the parsed stream planner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeSourceIssueKind {
    /// Fatal raw Annex B parser error.
    AnnexBParseError,
    /// Fatal IVF container parser error.
    IvfContainerError,
    /// Fatal Annex B parser error inside an IVF frame payload.
    IvfFramePayloadError,
    /// Non-fatal IVF container warning.
    IvfWarning,
    /// IVF codec metadata selects a codec outside the AV2 decoder input domain.
    IvfUnsupportedCodec,
    /// Fatal AV2 sequence-header syntax parse error.
    SequenceHeaderParseError,
    /// Fatal AV2 sequence-header conformance error.
    SequenceHeaderConformanceError,
    /// Fatal AV2 frame-header conformance error.
    FrameHeaderConformanceError,
    /// Fatal AV2 OBU payload syntax parse error encountered by the decode runtime.
    ObuPayloadParseError,
    /// Fatal coded-frame-unit conformance error.
    FrameUnitConformanceError,
    /// Fatal AV2 tile-payload syntax decode error.
    TilePayloadParseError,
}

impl fmt::Display for DecodeSourceIssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Source, container, or runtime bitstream issue observed while decoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeSourceIssue {
    kind: DecodeSourceIssueKind,
    rule_id: Option<&'static str>,
    spec_section: Option<&'static str>,
    offset: Option<ByteOffset>,
    frame_index: Option<usize>,
    message: String,
}

impl DecodeSourceIssue {
    pub(crate) fn sequence_header_parse(
        offset: ByteOffset,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::SequenceHeaderParseError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index: None,
            message,
        }
    }

    pub(crate) fn sequence_header_conformance(
        offset: ByteOffset,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::SequenceHeaderConformanceError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index: None,
            message,
        }
    }

    pub(crate) fn frame_header_conformance(
        offset: ByteOffset,
        frame_index: Option<usize>,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::FrameHeaderConformanceError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index,
            message,
        }
    }

    pub(crate) fn obu_payload_parse(
        offset: ByteOffset,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::ObuPayloadParseError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index: None,
            message,
        }
    }

    pub(crate) fn frame_unit_conformance(
        offset: ByteOffset,
        frame_index: Option<usize>,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::FrameUnitConformanceError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index,
            message,
        }
    }

    pub(crate) fn tile_payload(
        offset: ByteOffset,
        spec_section: &'static str,
        message: String,
    ) -> Self {
        Self {
            kind: DecodeSourceIssueKind::TilePayloadParseError,
            rule_id: None,
            spec_section: Some(spec_section),
            offset: Some(offset),
            frame_index: None,
            message,
        }
    }

    /// Source issue category.
    #[must_use]
    pub const fn kind(&self) -> DecodeSourceIssueKind {
        self.kind
    }

    /// Source parser rule id, when the parser exposes one.
    #[must_use]
    pub const fn rule_id(&self) -> Option<&'static str> {
        self.rule_id
    }

    /// AV2 section associated with the issue, when known.
    #[must_use]
    pub const fn spec_section(&self) -> Option<&'static str> {
        self.spec_section
    }

    /// Source byte offset, when known.
    #[must_use]
    pub const fn offset(&self) -> Option<ByteOffset> {
        self.offset
    }

    /// IVF frame index, when the issue is frame-local.
    #[must_use]
    pub const fn frame_index(&self) -> Option<usize> {
        self.frame_index
    }

    /// Human-readable source parser message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DecodeSourceIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.rule_id, self.offset, self.frame_index) {
            (Some(rule_id), Some(offset), Some(frame_index)) => write!(
                f,
                "{:?} ({rule_id}) in IVF frame {frame_index} at byte {offset}: {}",
                self.kind, self.message
            ),
            (Some(rule_id), Some(offset), None) => write!(
                f,
                "{:?} ({rule_id}) at byte {offset}: {}",
                self.kind, self.message
            ),
            (_, _, Some(frame_index)) => write!(
                f,
                "{:?} in IVF frame {frame_index}: {}",
                self.kind, self.message
            ),
            _ => write!(f, "{:?}: {}", self.kind, self.message),
        }
    }
}

/// Stable reason a parsed stream is outside the planner tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecodeUnsupportedReason {
    /// OBU layer scope violates AV2 § 6.2.2 global/local xlayer rules.
    InvalidLayerScope,
    /// OBU uses an embedded layer other than the selected base layer.
    NonBaseEmbeddedLayer,
    /// OBU uses an extended layer other than the selected base layer or global layer.
    NonBaseExtendedLayer,
    /// OBU participates in multistream or external-HLS selection.
    MultistreamSelection,
}

/// Generates an exhaustive `const fn as_str(self) -> &'static str` label mapping.
#[macro_export]
macro_rules! impl_reason_labels {
    ($vis:vis $name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        impl $name {
            /// Stable snake-case label.
            #[must_use]
            $vis const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }
    };
}

impl_reason_labels!(pub DecodeUnsupportedReason {
    InvalidLayerScope => "invalid_layer_scope",
    NonBaseEmbeddedLayer => "non_base_embedded_layer",
    NonBaseExtendedLayer => "non_base_extended_layer",
    MultistreamSelection => "multistream_selection",
});

impl_reason_labels!(pub DecodeSourceIssueKind {
    AnnexBParseError => "annex_b_parse_error",
    IvfContainerError => "ivf_container_error",
    IvfFramePayloadError => "ivf_frame_payload_error",
    IvfWarning => "ivf_warning",
    IvfUnsupportedCodec => "ivf_unsupported_codec",
    SequenceHeaderParseError => "sequence_header_parse_error",
    SequenceHeaderConformanceError => "sequence_header_conformance_error",
    FrameHeaderConformanceError => "frame_header_conformance_error",
    ObuPayloadParseError => "obu_payload_parse_error",
    FrameUnitConformanceError => "frame_unit_conformance_error",
    TilePayloadParseError => "tile_payload_parse_error",
});

impl fmt::Display for DecodeUnsupportedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unsupported structure metadata for plan-only stream traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodeUnsupportedStructure {
    reason: DecodeUnsupportedReason,
    obu_type: ObuType,
    offset: ByteOffset,
    spec_section: &'static str,
    message: &'static str,
}

impl DecodeUnsupportedStructure {
    /// Stable decoder diagnostic rule id for diagnostic adaptation.
    #[must_use]
    pub const fn rule_id(&self) -> &'static str {
        UNSUPPORTED_FEATURE_RULE_ID
    }

    /// Feature ID that owns this unsupported planner result.
    #[must_use]
    pub const fn feature_id(&self) -> &'static str {
        DECODE_STREAM_STATE_FEATURE_ID
    }

    /// Stable unsupported reason.
    #[must_use]
    pub const fn reason(&self) -> DecodeUnsupportedReason {
        self.reason
    }

    /// OBU type that triggered the unsupported result.
    #[must_use]
    pub const fn obu_type(&self) -> ObuType {
        self.obu_type
    }

    /// OBU byte offset that triggered the unsupported result.
    #[must_use]
    pub const fn offset(&self) -> ByteOffset {
        self.offset
    }

    /// AV2 spec section associated with the unsupported result.
    #[must_use]
    pub const fn spec_section(&self) -> &'static str {
        self.spec_section
    }

    /// Human-readable unsupported result.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

impl fmt::Display for DecodeUnsupportedStructure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {} for {}: {}",
            self.reason,
            self.offset,
            self.obu_type.spec_name(),
            self.message
        )
    }
}

pub(crate) fn plan_stream(
    input: DecodeStreamInput<'_>,
    options: &DecodeOptions,
) -> Result<DecodeStreamPlan> {
    options.validate_selections()?;
    let limits = options.limits();
    limits.ensure(DecodeLimitName::MaxInputBytes, input.input_len_bytes)?;

    match input.parsed {
        ParsedBitstream::AnnexB(partial) => {
            plan_annex_b(partial, input.input_len_bytes, options, true)
        }
        ParsedBitstream::Ivf(ivf) => {
            let mut builder =
                PlanBuilder::new(BitstreamFormat::Ivf, input.input_len_bytes, options, true);
            let mut planner = IvfPlanner::default();
            for frame in &ivf.frames {
                planner.push_frame(&mut builder, frame.frame, &frame.obus, frame.error.as_ref());
            }
            planner.finish(builder, ivf.header, &ivf.warnings, ivf.error.as_ref())
        }
    }
}

pub(crate) fn plan_annex_b(
    partial: &PartialParse<'_>,
    input_len_bytes: u64,
    options: &DecodeOptions,
    keep_obus: bool,
) -> Result<DecodeStreamPlan> {
    options.validate_selections()?;
    if let Some(error) = &partial.error {
        return Err(DecodeError::MalformedSource {
            issue: issue_from_core_error(DecodeSourceIssueKind::AnnexBParseError, None, error),
        });
    }
    let mut builder =
        PlanBuilder::new(BitstreamFormat::AnnexB, input_len_bytes, options, keep_obus);
    for &obu in &partial.obus {
        builder.push_obu(obu, DecodeObuSourceKind::AnnexB, None)?;
    }
    builder.finish()
}

/// Plans IVF frame records one at a time. A record's planning error waits for
/// [`Self::finish`], so container errors found later in the stream still win,
/// as they do when the whole stream is parsed before it is planned.
#[derive(Default)]
pub(crate) struct IvfPlanner {
    first_unsupported: Option<DecodeUnsupportedStructure>,
    error: Option<DecodeError>,
}

impl IvfPlanner {
    pub(crate) fn push_frame(
        &mut self,
        builder: &mut PlanBuilder,
        frame: splot_core::ivf::IvfFrame<'_>,
        obus: &[ObuEnvelope<'_>],
        error: Option<&splot_core::Error>,
    ) {
        if self.error.is_none()
            && let Err(error) = self.try_push_frame(builder, frame, obus, error)
        {
            self.error = Some(error);
        }
    }

    fn try_push_frame(
        &mut self,
        builder: &mut PlanBuilder,
        frame: splot_core::ivf::IvfFrame<'_>,
        obus: &[ObuEnvelope<'_>],
        error: Option<&splot_core::Error>,
    ) -> Result<()> {
        builder
            .limits
            .ensure(DecodeLimitName::MaxIvfFrameRecords, frame.index as u64 + 1)?;
        if let Some(error) = error {
            return Err(DecodeError::MalformedSource {
                issue: issue_from_core_error(
                    DecodeSourceIssueKind::IvfFramePayloadError,
                    Some(frame.index),
                    error,
                ),
            });
        }
        let context = Some(ivf_frame_context(frame));
        for &obu in obus {
            builder.push_obu_or_first_unsupported(
                obu,
                DecodeObuSourceKind::Ivf,
                context,
                &mut self.first_unsupported,
            )?;
        }
        Ok(())
    }

    pub(crate) fn finish(
        self,
        mut builder: PlanBuilder,
        header: Option<splot_core::ivf::IvfHeader>,
        warnings: &[IvfWarning],
        error: Option<&IvfError>,
    ) -> Result<DecodeStreamPlan> {
        for warning in warnings {
            builder
                .source_warnings
                .push(issue_from_ivf_warning(warning));
        }
        if let Some(error) = error {
            return Err(DecodeError::MalformedSource {
                issue: issue_from_ivf_error(error),
            });
        }
        if let Some(header) = header
            && header.fourcc != *b"AV02"
        {
            return Err(DecodeError::MalformedSource {
                issue: issue_from_unsupported_ivf_codec(header.fourcc),
            });
        }
        if let Some(error) = self.error {
            return Err(error);
        }
        if let Some(unsupported) = self.first_unsupported {
            return Err(DecodeError::UnsupportedStructure { unsupported });
        }
        builder.finish()
    }
}

pub(crate) struct PlanBuilder {
    format: BitstreamFormat,
    selected_layer: Option<DecodeLayerSelection>,
    layer_filter: ActiveLayerFilter,
    pub(crate) input_len_bytes: u64,
    limits: crate::DecodeLimits,
    keep_obus: bool,
    obus: Vec<DecodePlannedObu>,
    obu_count: u64,
    traversed_obu_count: u64,
    frame_candidate_count: u64,
    saw_any_frame: bool,
    source_warnings: Vec<DecodeSourceIssue>,
}

impl PlanBuilder {
    pub(crate) fn new(
        format: BitstreamFormat,
        input_len_bytes: u64,
        options: &DecodeOptions,
        keep_obus: bool,
    ) -> Self {
        Self {
            format,
            selected_layer: None,
            layer_filter: ActiveLayerFilter::from_options(options),
            input_len_bytes,
            limits: options.limits(),
            keep_obus,
            obus: Vec::new(),
            obu_count: 0,
            traversed_obu_count: 0,
            frame_candidate_count: 0,
            saw_any_frame: false,
            source_warnings: Vec::new(),
        }
    }

    fn push_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
    ) -> Result<()> {
        let role = self.classify_limited_obu(envelope)?;
        self.push_classified_obu(envelope, source_kind, ivf_frame, role);
        Ok(())
    }

    fn push_obu_or_first_unsupported(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        first_unsupported: &mut Option<DecodeUnsupportedStructure>,
    ) -> Result<()> {
        match self.push_obu(envelope, source_kind, ivf_frame) {
            Ok(()) => Ok(()),
            Err(DecodeError::UnsupportedStructure { unsupported }) => {
                if first_unsupported.is_none() {
                    *first_unsupported = Some(unsupported);
                }
                Ok(())
            }
            Err(DecodeError::Limit { source }) => {
                if let Some(unsupported) = first_unsupported.as_ref() {
                    Err(DecodeError::UnsupportedStructure {
                        unsupported: unsupported.clone(),
                    })
                } else {
                    Err(DecodeError::Limit { source })
                }
            }
            Err(error) => Err(error),
        }
    }

    fn classify_limited_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<DecodePlannedObuRole> {
        let next_obu_count = self.traversed_obu_count.saturating_add(1);
        self.limits
            .ensure(DecodeLimitName::MaxObus, next_obu_count)?;
        self.traversed_obu_count = next_obu_count;

        if is_coded_frame_obu(envelope.header.obu_type) {
            self.saw_any_frame = true;
        }
        let selected_layer = self.selected_layer.unwrap_or_default();
        let role = classify_obu(envelope, selected_layer, &mut self.layer_filter)?;
        DecodeLayerSelection::adopt_with_filter(
            &mut self.selected_layer,
            envelope.header.extended_layer_id,
            &self.layer_filter,
        );
        if role.is_frame_candidate() {
            let next_frame_count = self.frame_candidate_count.saturating_add(1);
            self.limits
                .ensure(DecodeLimitName::MaxFramesToDecode, next_frame_count)?;
            self.frame_candidate_count = next_frame_count;
        }

        Ok(role)
    }

    fn push_classified_obu(
        &mut self,
        envelope: ObuEnvelope<'_>,
        source_kind: DecodeObuSourceKind,
        ivf_frame: Option<DecodeIvfFrameContext>,
        role: DecodePlannedObuRole,
    ) {
        if self.keep_obus {
            self.obus.push(DecodePlannedObu::new(
                self.obu_count,
                source_kind,
                ivf_frame,
                envelope,
                role,
            ));
        }
        self.obu_count += 1;
    }

    fn finish(self) -> Result<DecodeStreamPlan> {
        self.layer_filter.ensure_operating_point_at_eof(
            self.saw_any_frame,
            ByteOffset::new(self.input_len_bytes),
        )?;
        Ok(DecodeStreamPlan {
            format: self.format,
            selected_layer: self.selected_layer.unwrap_or_else(|| {
                DecodeLayerSelection::default()
                    .with_temporal_selection(self.layer_filter.selection())
                    .with_embedded_selection(self.layer_filter.embedded_selection())
                    .with_extended_selection(self.layer_filter.extended_selection())
            }),
            input_len_bytes: self.input_len_bytes,
            obu_count: self.obu_count,
            obus: self.obus,
            frame_candidate_count: self.frame_candidate_count,
            source_warnings: self.source_warnings,
        })
    }
}

pub(crate) fn is_coded_frame_obu(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::ClosedLoopKey
            | ObuType::OpenLoopKey
            | ObuType::LeadingTileGroup
            | ObuType::RegularTileGroup
            | ObuType::Switch
            | ObuType::LeadingSef
            | ObuType::RegularSef
            | ObuType::LeadingTip
            | ObuType::RegularTip
            | ObuType::BridgeFrame
            | ObuType::RasFrame
    )
}

pub(crate) fn classify_obu(
    envelope: ObuEnvelope<'_>,
    _selected_layer: DecodeLayerSelection,
    layer_filter: &mut ActiveLayerFilter,
) -> Result<DecodePlannedObuRole> {
    let header = envelope.header;
    let obu_type = header.obu_type;

    if obu_type.is_reserved() {
        return Ok(DecodePlannedObuRole::Global);
    }

    if obu_type.requires_global_xlayer() && header.extended_layer_id != GLOBAL_XLAYER_ID {
        return unsupported(
            DecodeUnsupportedReason::InvalidLayerScope,
            envelope,
            "6.2.2",
            "this OBU type must use the AV2 global extended layer id",
        );
    }
    if header.extended_layer_id == GLOBAL_XLAYER_ID && !obu_type.permits_global_xlayer() {
        return unsupported(
            DecodeUnsupportedReason::InvalidLayerScope,
            envelope,
            "6.2.2",
            "this OBU type is not permitted to use the AV2 global extended layer id",
        );
    }
    if obu_type == ObuType::Msdo
        && (header.temporal_layer_id != TemporalLayerId::BASE
            || header.embedded_layer_id != EmbeddedLayerId::BASE)
    {
        return unsupported(
            DecodeUnsupportedReason::InvalidLayerScope,
            envelope,
            "6.6",
            "OBU_MSDO must use obu_tlayer_id == 0 and obu_mlayer_id == 0",
        );
    }
    if obu_type == ObuType::TemporalDelimiter {
        return Ok(DecodePlannedObuRole::Global);
    }

    layer_filter.observe_envelope(envelope)?;
    if is_coded_frame_obu(obu_type) {
        layer_filter.check_operating_point_for_frame(header.extended_layer_id, envelope.offset)?;
    }

    if header.extended_layer_id != GLOBAL_XLAYER_ID
        && !layer_filter.is_xlayer_selected(header.extended_layer_id)
    {
        return Ok(DecodePlannedObuRole::UnselectedLayer);
    }
    if !obu_type.requires_base_temporal_and_embedded_layer()
        && !layer_filter.is_layer_selected_for(
            header.extended_layer_id,
            header.embedded_layer_id,
            header.temporal_layer_id,
        )
    {
        return Ok(DecodePlannedObuRole::UnselectedLayer);
    }

    match header.obu_type {
        ObuType::Padding => Ok(DecodePlannedObuRole::Global),
        obu_type if obu_type.is_tile_group() && is_tile_group_continuation(envelope) => {
            Ok(DecodePlannedObuRole::FrameContinuation)
        }
        ObuType::ClosedLoopKey | ObuType::OpenLoopKey => Ok(DecodePlannedObuRole::FrameCandidate),
        ObuType::LeadingTileGroup
        | ObuType::RegularTileGroup
        | ObuType::Switch
        | ObuType::LeadingSef
        | ObuType::RegularSef
        | ObuType::LeadingTip
        | ObuType::RegularTip
        | ObuType::BridgeFrame
        | ObuType::RasFrame => Ok(DecodePlannedObuRole::InterFrameCandidate),
        ObuType::OperatingPointSet | ObuType::LayerConfigurationRecord | ObuType::AtlasSegment
            if header.extended_layer_id == GLOBAL_XLAYER_ID =>
        {
            Ok(DecodePlannedObuRole::Global)
        }
        ObuType::SequenceHeader
        | ObuType::OperatingPointSet
        | ObuType::LayerConfigurationRecord
        | ObuType::AtlasSegment
        | ObuType::MultiFrameHeader
        | ObuType::MetadataShort
        | ObuType::MetadataGroup
        | ObuType::BufferRemovalTiming
        | ObuType::QuantizationMatrix
        | ObuType::FilmGrain
        | ObuType::ContentInterpretation => Ok(DecodePlannedObuRole::SelectedLayerState),
        ObuType::Msdo | ObuType::Reserved0 | ObuType::Reserved(_) | ObuType::TemporalDelimiter => {
            Ok(DecodePlannedObuRole::Global)
        }
    }
}

/// The OBUs of `bytes` from absolute offset `from` to `end`, planned again as
/// the planner planned them; the first one gets plan index `index`.
#[derive(Clone)]
struct Replan<'a, F = ActiveLayerFilter> {
    cursor: AnnexBObuCursor<'a>,
    index: u64,
    source_kind: DecodeObuSourceKind,
    ivf_frame: Option<DecodeIvfFrameContext>,
    selected_layer: Option<DecodeLayerSelection>,
    layer_filter: F,
}

impl<'a, F> Replan<'a, F> {
    fn new(
        bytes: SourceBytes<'a>,
        from: u64,
        end: u64,
        index: u64,
        ivf_frame: Option<DecodeIvfFrameContext>,
        selected_layer: Option<DecodeLayerSelection>,
        layer_filter: F,
    ) -> Self {
        Self {
            selected_layer,
            layer_filter,
            cursor: AnnexBObuCursor::new(
                bytes.get(from, end).unwrap_or_default(),
                ByteOffset::new(from),
            ),
            index,
            source_kind: if ivf_frame.is_some() {
                DecodeObuSourceKind::Ivf
            } else {
                DecodeObuSourceKind::AnnexB
            },
            ivf_frame,
        }
    }
}

impl<F: core::borrow::BorrowMut<ActiveLayerFilter>> Iterator for Replan<'_, F> {
    type Item = Result<DecodePlannedObu>;

    fn next(&mut self) -> Option<Self::Item> {
        let envelope = match self.cursor.next_obu() {
            Ok(envelope) => envelope?,
            Err(error) => {
                self.cursor = AnnexBObuCursor::new(&[], ByteOffset::new(0));
                return Some(Err(DecodeError::MalformedSource {
                    issue: issue_from_core_error(
                        DecodeSourceIssueKind::AnnexBParseError,
                        None,
                        &error,
                    ),
                }));
            }
        };
        let index = self.index;
        self.index += 1;
        let filter = self.layer_filter.borrow_mut();
        let selected_layer = self.selected_layer.unwrap_or_default();
        let planned = classify_obu(envelope, selected_layer, filter).map(|role| {
            DecodePlannedObu::new(index, self.source_kind, self.ivf_frame, envelope, role)
        });
        DecodeLayerSelection::adopt_with_filter(
            &mut self.selected_layer,
            envelope.header.extended_layer_id,
            filter,
        );
        Some(planned)
    }
}

/// Finds the planned frame candidates again, in order, in the bytes the decode
/// pass reads. Scanning every OBU once reproduces their plan indices.
pub(crate) struct CandidateScan {
    resume: u64,
    index: u64,
    frames: u64,
    saw_any_frame: bool,
    selected_layer: Option<DecodeLayerSelection>,
    layer_filter: ActiveLayerFilter,
}

impl Default for CandidateScan {
    fn default() -> Self {
        Self::new(
            TemporalLayerSelection::All,
            EmbeddedLayerSelection::All,
            ExtendedLayerSelection::All,
        )
    }
}

impl CandidateScan {
    pub(crate) fn new(
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
        extended: ExtendedLayerSelection,
    ) -> Self {
        Self::new_with_selections(temporal, embedded, extended, OperatingPointSelection::None)
    }

    pub(crate) fn new_with_selections(
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
        extended: ExtendedLayerSelection,
        operating_point: OperatingPointSelection,
    ) -> Self {
        Self {
            resume: 0,
            index: 0,
            frames: 0,
            saw_any_frame: false,
            selected_layer: None,
            layer_filter: ActiveLayerFilter::new_with_selections(
                temporal,
                embedded,
                extended,
                operating_point,
            ),
        }
    }

    pub(crate) const fn layer_filter(&self) -> &ActiveLayerFilter {
        &self.layer_filter
    }

    pub(crate) fn finish_eof(&self, offset: ByteOffset) -> Result<()> {
        self.layer_filter
            .ensure_operating_point_at_eof(self.saw_any_frame, offset)
    }

    /// The next frame candidate in `bytes`, one temporal unit, after the last
    /// candidate found, counting OBUs and candidates against `limits`.
    pub(crate) fn next(
        &mut self,
        bytes: SourceBytes<'_>,
        ivf_frame: Option<DecodeIvfFrameContext>,
        limits: crate::DecodeLimits,
    ) -> Result<Option<DecodePlannedObu>> {
        let from = self.resume.max(bytes.base());
        let mut replan = Replan::new(
            bytes,
            from,
            bytes.end(),
            self.index,
            ivf_frame,
            self.selected_layer,
            &mut self.layer_filter,
        );
        for planned in replan.by_ref() {
            let planned = planned?;
            if is_coded_frame_obu(planned.header.obu_type) {
                self.saw_any_frame = true;
            }
            limits.ensure(DecodeLimitName::MaxObus, planned.index + 1)?;
            self.resume = planned.end();
            self.index = planned.index + 1;
            if planned.role.is_frame_candidate() {
                self.frames += 1;
                limits.ensure(DecodeLimitName::MaxFramesToDecode, self.frames)?;
                self.selected_layer = replan.selected_layer;
                return Ok(Some(planned));
            }
        }
        self.selected_layer = replan.selected_layer;
        Ok(None)
    }
}

/// The tile-group OBUs in `bytes` that continue `candidate`'s coded frame,
/// skipping padding, up to the end of the candidate's IVF frame record.
pub(crate) fn tile_group_continuations<'a>(
    bytes: SourceBytes<'a>,
    candidate: &'a DecodePlannedObu,
) -> impl Iterator<Item = DecodePlannedObu> + Clone + 'a {
    let end = candidate.ivf_frame.map_or(bytes.end(), |frame| {
        frame
            .frame_payload_offset
            .get()
            .saturating_add(u64::from(frame.frame_payload_size))
            .min(bytes.end())
    });
    let temporal_selection = TemporalLayerSelection::Target(candidate.header.temporal_layer_id);
    let embedded_selection = EmbeddedLayerSelection::Target(candidate.header.embedded_layer_id);
    let extended_selection = ExtendedLayerSelection::Target(candidate.header.extended_layer_id);
    Replan::new(
        bytes,
        candidate.end(),
        end,
        candidate.index + 1,
        candidate.ivf_frame,
        Some(DecodeLayerSelection::of_stream(
            core::iter::once(candidate.header.extended_layer_id),
            temporal_selection,
            embedded_selection,
            extended_selection,
        )),
        ActiveLayerFilter::new_with_extended(
            temporal_selection,
            embedded_selection,
            extended_selection,
        ),
    )
    .map_while(Result::ok)
    .filter(|planned| planned.obu_type() != ObuType::Padding)
    .take_while(move |planned| {
        planned.role.is_frame_continuation()
            && planned.obu_type() == candidate.obu_type()
            && planned.header.temporal_layer_id == candidate.header.temporal_layer_id
            && planned.header.embedded_layer_id == candidate.header.embedded_layer_id
            && planned.header.extended_layer_id == candidate.header.extended_layer_id
    })
}

fn is_tile_group_continuation(envelope: ObuEnvelope<'_>) -> bool {
    envelope
        .payload
        .first()
        .is_some_and(|first| first & 0x80 == 0)
}

fn unsupported(
    reason: DecodeUnsupportedReason,
    envelope: ObuEnvelope<'_>,
    spec_section: &'static str,
    message: &'static str,
) -> Result<DecodePlannedObuRole> {
    Err(DecodeError::UnsupportedStructure {
        unsupported: DecodeUnsupportedStructure {
            reason,
            obu_type: envelope.header.obu_type,
            offset: envelope.offset,
            spec_section,
            message,
        },
    })
}

pub(crate) fn issue_from_core_error(
    kind: DecodeSourceIssueKind,
    frame_index: Option<usize>,
    error: &splot_core::Error,
) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind,
        rule_id: None,
        spec_section: None,
        offset: core_error_offset(error),
        frame_index,
        message: error.to_string(),
    }
}

pub(crate) fn issue_from_ivf_error(error: &IvfError) -> DecodeSourceIssue {
    issue_from_ivf_source(
        DecodeSourceIssueKind::IvfContainerError,
        error.rule_id(),
        error.offset(),
        ivf_error_frame_index(error),
        error.to_string(),
    )
}

fn issue_from_ivf_warning(warning: &IvfWarning) -> DecodeSourceIssue {
    issue_from_ivf_source(
        DecodeSourceIssueKind::IvfWarning,
        warning.rule_id(),
        warning.offset(),
        ivf_warning_frame_index(warning),
        warning.to_string(),
    )
}

pub(crate) fn issue_from_unsupported_ivf_codec(fourcc: [u8; 4]) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind: DecodeSourceIssueKind::IvfUnsupportedCodec,
        rule_id: Some("decode/unsupported-ivf-codec"),
        spec_section: None,
        offset: Some(ByteOffset::new(8)),
        frame_index: None,
        message: format!(
            "IVF codec fourcc 0x{:02X}{:02X}{:02X}{:02X} is not AV02",
            fourcc[0], fourcc[1], fourcc[2], fourcc[3]
        ),
    }
}

fn issue_from_ivf_source(
    kind: DecodeSourceIssueKind,
    rule_id: &'static str,
    offset: ByteOffset,
    frame_index: Option<usize>,
    message: String,
) -> DecodeSourceIssue {
    DecodeSourceIssue {
        kind,
        rule_id: Some(rule_id),
        spec_section: None,
        offset: Some(offset),
        frame_index,
        message,
    }
}

fn ivf_frame_context(frame: splot_core::ivf::IvfFrame<'_>) -> DecodeIvfFrameContext {
    DecodeIvfFrameContext::new(frame.index, frame.payload_offset, frame.size, frame.pts)
}

fn ivf_error_frame_index(error: &IvfError) -> Option<usize> {
    match error {
        IvfError::TruncatedFrameHeader { frame_index, .. }
        | IvfError::TruncatedFramePayload { frame_index, .. } => Some(*frame_index),
        _ => None,
    }
}

fn ivf_warning_frame_index(warning: &IvfWarning) -> Option<usize> {
    match warning {
        IvfWarning::TrailingPartialFrameHeader { frame_index, .. } => Some(*frame_index),
        _ => None,
    }
}

fn core_error_offset(error: &splot_core::Error) -> Option<ByteOffset> {
    match error {
        splot_core::Error::UnexpectedEof { offset, .. }
        | splot_core::Error::InvalidLeb128 { offset, .. }
        | splot_core::Error::InvalidUvlc { offset, .. }
        | splot_core::Error::InvalidNs { offset, .. }
        | splot_core::Error::InvalidRg { offset, .. }
        | splot_core::Error::InvalidQuantizerMatrix { offset, .. }
        | splot_core::Error::InvalidObuHeader { offset, .. }
        | splot_core::Error::InvalidTrailingBits { offset, .. }
        | splot_core::Error::InvalidByteAlignment { offset, .. }
        | splot_core::Error::InvalidSequenceHeader { offset, .. }
        | splot_core::Error::InvalidTileParams { offset, .. }
        | splot_core::Error::ObuSizeOutOfRange { offset, .. }
        | splot_core::Error::InvalidObuExtension { offset, .. }
        | splot_core::Error::ObuPayloadOutOfRange { offset, .. }
        | splot_core::Error::InvalidLayerConfigRecord { offset, .. }
        | splot_core::Error::InvalidAtlasSegment { offset, .. }
        | splot_core::Error::InvalidPadding { offset, .. }
        | splot_core::Error::InvalidMetadata { offset, .. } => Some(*offset),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
