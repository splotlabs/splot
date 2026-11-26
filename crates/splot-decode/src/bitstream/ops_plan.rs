// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Planner-level `OBU_OPERATING_POINT_SET` selection and dependency closure.

use std::collections::BTreeMap;

use splot_core::annexb::ObuEnvelope;
use splot_core::bitio::BitReader;
use splot_core::headers::operating_point_set::{
    OperatingPointPayload, OperatingPointSet, OpsMlayerInfo, OpsMlayerSource,
    parse_operating_point_set,
};
use splot_core::obu::finish_obu_payload;
use splot_core::span::ByteOffset;
use splot_core::types::{
    EmbeddedLayerId, ExtendedLayerId, GLOBAL_XLAYER_ID, ObuType, TemporalLayerId,
};

use super::stream_plan::{
    DecodeSourceIssue, EmbeddedLayerSelection, ExtendedLayerSelection, LayerDependencyFilter,
    TemporalLayerSelection,
};
use crate::error::{DecodeError, Result};

const MAX_OPS_ID: u8 = 15;
const MAX_OP_INDEX: u8 = 6;
const MAX_INHERITANCE_DEPTH: usize = 16;

/// Caller-selected operating point within an `OBU_OPERATING_POINT_SET` (§ 6.10).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OperatingPointSelection {
    /// Do not filter by `OBU_OPERATING_POINT_SET`; use explicit layer selections.
    #[default]
    None,
    /// Select operating point `op_index` (`0..=6`) from global OPS `ops_id` (`0..=15`).
    Global {
        /// `ops_id` (`0..=15`).
        ops_id: u8,
        /// Operating point index (`0..=6`, `< ops_cnt`).
        op_index: u8,
    },
    /// Select operating point `op_index` (`0..=6`) from local OPS `(xlayer_id, ops_id)`.
    Local {
        /// Extended layer carrying the local OPS (`0..=30`).
        xlayer_id: ExtendedLayerId,
        /// `ops_id` (`0..=15`).
        ops_id: u8,
        /// Operating point index (`0..=6`, `< ops_cnt`).
        op_index: u8,
    },
}

impl OperatingPointSelection {
    pub(crate) const fn validate(self) -> Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Global { ops_id, op_index } => {
                if ops_id > MAX_OPS_ID {
                    return Err(DecodeError::InvalidOperatingPointSelection {
                        reason: "operating point set id must be within 0..=15",
                    });
                }
                if op_index > MAX_OP_INDEX {
                    return Err(DecodeError::InvalidOperatingPointSelection {
                        reason: "operating point index must be within 0..=6",
                    });
                }
                Ok(())
            }
            Self::Local {
                xlayer_id,
                ops_id,
                op_index,
            } => {
                if xlayer_id.get() > 30 {
                    return Err(DecodeError::InvalidOperatingPointSelection {
                        reason: "local operating point extended layer must be within 0..=30",
                    });
                }
                if ops_id > MAX_OPS_ID {
                    return Err(DecodeError::InvalidOperatingPointSelection {
                        reason: "operating point set id must be within 0..=15",
                    });
                }
                if op_index > MAX_OP_INDEX {
                    return Err(DecodeError::InvalidOperatingPointSelection {
                        reason: "operating point index must be within 0..=6",
                    });
                }
                Ok(())
            }
        }
    }

    pub(crate) fn validate_with(
        self,
        temporal: TemporalLayerSelection,
        embedded: EmbeddedLayerSelection,
        extended: ExtendedLayerSelection,
    ) -> Result<()> {
        self.validate()?;
        if !matches!(self, Self::None)
            && (!matches!(temporal, TemporalLayerSelection::All)
                || !matches!(embedded, EmbeddedLayerSelection::All)
                || !matches!(extended, ExtendedLayerSelection::All))
        {
            return Err(DecodeError::InvalidOperatingPointSelection {
                reason: "operating-point selection cannot be combined with explicit temporal, embedded, or extended layer selection",
            });
        }
        Ok(())
    }

    pub(crate) const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    const fn target(self) -> Option<(u8, u8, u8)> {
        match self {
            Self::None => None,
            Self::Global { ops_id, op_index } => Some((GLOBAL_XLAYER_ID.get(), ops_id, op_index)),
            Self::Local {
                xlayer_id,
                ops_id,
                op_index,
            } => Some((xlayer_id.get(), ops_id, op_index)),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedOpsTarget {
    pub(crate) xlayer_mask: u32,
    pub(crate) per_xlayer: Vec<(ExtendedLayerId, [u8; 8])>,
}

/// Resolves operating point membership as OPS OBUs activate, change, or reset
/// (AV2 § 6.10.2 and § 6.10.7). Use one tracker per input stream.
#[derive(Clone, Debug)]
pub struct OperatingPointTracker {
    selection: OperatingPointSelection,
    sets: BTreeMap<(u8, u8), OperatingPointSet>,
    resolved: Option<ResolvedOpsTarget>,
    ever_resolved: bool,
    unavailable_reason: Option<&'static str>,
}

impl OperatingPointTracker {
    /// Creates a tracker for a requested operating point.
    ///
    /// # Errors
    ///
    /// Returns an error if the OPS ID, local extended layer ID, or operating
    /// point index is invalid.
    pub fn new(selection: OperatingPointSelection) -> Result<Self> {
        selection.validate()?;
        Ok(Self::from_valid_selection(selection))
    }

    /// Observes an OBU in stream order. Only a base-layer OPS OBU can update
    /// the tracker.
    ///
    /// # Errors
    ///
    /// Returns an error if the OPS cannot activate in the decoder. After an
    /// error, previous membership is unavailable until a reset or update.
    pub fn observe(&mut self, obu: &ObuEnvelope<'_>) -> Result<()> {
        if self.selection.is_none() || !Self::accepts(obu) {
            return Ok(());
        }
        if let Err(error) = self.apply_obu(*obu) {
            self.sets.clear();
            self.resolved = None;
            self.unavailable_reason = Some("the operating point set failed decoder checks");
            return Err(error);
        }
        Ok(())
    }

    /// Returns the reason why the requested operating point is unavailable.
    /// A selection of [`OperatingPointSelection::None`] has no reason.
    #[must_use]
    pub const fn unavailable_reason(&self) -> Option<&'static str> {
        if self.selection.is_none() {
            return None;
        }
        self.unavailable_reason
    }

    /// Returns a concrete layer triple's membership at the last observed OBU.
    /// Returns `None` if there is no resolved operating point.
    ///
    /// Membership describes the decoder's OPS filter. A local selection
    /// filters embedded and temporal layers only within its own extended
    /// layer. Frame-level decode, dependency
    /// availability, and conformance checks can still reject the stream.
    #[must_use]
    pub fn contains(
        &self,
        xlayer: ExtendedLayerId,
        mlayer: EmbeddedLayerId,
        tlayer: TemporalLayerId,
    ) -> Option<bool> {
        let resolved = self.resolved.as_ref()?;
        if xlayer.get() >= 31 || resolved.xlayer_mask & (1 << xlayer.get()) == 0 {
            return Some(false);
        }
        if mlayer.get() >= 8 || tlayer.get() >= 4 {
            return Some(false);
        }
        let Some((_, masks)) = resolved.per_xlayer.iter().find(|(id, _)| *id == xlayer) else {
            return Some(true);
        };
        Some(masks[usize::from(mlayer.get())] & (1 << tlayer.get()) != 0)
    }

    pub(crate) fn accepts(obu: &ObuEnvelope<'_>) -> bool {
        obu.header.obu_type == ObuType::OperatingPointSet
            && obu.header.temporal_layer_id == TemporalLayerId::BASE
            && obu.header.embedded_layer_id == EmbeddedLayerId::BASE
    }

    pub(crate) fn from_valid_selection(selection: OperatingPointSelection) -> Self {
        Self {
            selection,
            sets: BTreeMap::new(),
            resolved: None,
            ever_resolved: false,
            unavailable_reason: Some("requested operating point is not present before frames"),
        }
    }

    pub(crate) const fn is_resolved(&self) -> bool {
        self.resolved.is_some()
    }

    pub(crate) const fn ever_resolved(&self) -> bool {
        self.ever_resolved
    }

    pub(crate) fn is_xlayer_selected(&self, xlayer: ExtendedLayerId) -> bool {
        let id = xlayer.get();
        if id > 30 {
            return false;
        }
        if let Some(resolved) = &self.resolved {
            return (resolved.xlayer_mask & (1u32 << id)) != 0;
        }
        match self.selection {
            OperatingPointSelection::Local { .. } => true,
            OperatingPointSelection::Global { .. } | OperatingPointSelection::None => false,
        }
    }

    pub(crate) fn should_fail_unavailable_frame(&self, xlayer: ExtendedLayerId) -> bool {
        if self.resolved.is_some() {
            return false;
        }
        match self.selection {
            OperatingPointSelection::None => false,
            OperatingPointSelection::Global { .. } => xlayer != GLOBAL_XLAYER_ID,
            OperatingPointSelection::Local { xlayer_id, .. } => xlayer_id == xlayer,
        }
    }

    pub(crate) fn unavailable_error(&self, offset: ByteOffset) -> DecodeError {
        let message = self
            .unavailable_reason
            .unwrap_or("requested operating point is unavailable")
            .to_owned();
        DecodeError::MalformedSource {
            issue: DecodeSourceIssue::frame_unit_conformance(offset, None, "6.10.1", message),
        }
    }

    pub(crate) fn resolved_entries(&self) -> &[(ExtendedLayerId, [u8; 8])] {
        self.resolved
            .as_ref()
            .map_or(&[][..], |resolved| &resolved.per_xlayer)
    }

    pub(crate) fn apply_obu(&mut self, envelope: ObuEnvelope<'_>) -> Result<bool> {
        let mut reader = BitReader::new(envelope.payload, envelope.payload_offset());
        let ops = parse_operating_point_set(&mut reader, envelope.header.extended_layer_id)
            .map_err(|error| DecodeError::MalformedSource {
                issue: DecodeSourceIssue::obu_payload_parse(
                    envelope.offset,
                    "5.10",
                    format!("failed to parse operating point set: {error}"),
                ),
            })?;
        finish_obu_payload(
            &mut reader,
            envelope.payload,
            envelope.header.obu_type.is_extensible_obu(),
        )
        .map_err(|error| DecodeError::MalformedSource {
            issue: DecodeSourceIssue::obu_payload_parse(
                envelope.offset,
                "5.10",
                format!("operating point set payload tail invalid: {error}"),
            ),
        })?;
        validate_ops_semantics(&ops, envelope.offset)?;

        let key_xlayer = ops.xlayer_id.get();
        if ops.reset_flag {
            if ops.xlayer_id.is_global() {
                self.sets.clear();
            } else {
                self.sets.retain(|&(x, _), _| x != key_xlayer);
            }
        }
        if ops.ops_cnt == 0 {
            self.sets.remove(&(key_xlayer, ops.ops_id));
        } else {
            self.sets.insert((key_xlayer, ops.ops_id), ops);
        }
        self.recompute_resolved();
        Ok(true)
    }

    pub(crate) fn apply_masks_to_filter(
        &self,
        xlayer: ExtendedLayerId,
        filter: &mut LayerDependencyFilter,
    ) {
        if let Some((_, masks)) = self.resolved_entries().iter().find(|(id, _)| *id == xlayer) {
            filter.set_output_tlayer_masks(*masks);
        }
    }

    fn recompute_resolved(&mut self) {
        let Some((key_xlayer, ops_id, op_index)) = self.selection.target() else {
            self.resolved = None;
            self.unavailable_reason = None;
            return;
        };
        let Some(ops) = self.sets.get(&(key_xlayer, ops_id)) else {
            self.resolved = None;
            self.unavailable_reason =
                Some("requested operating point set is missing or has been reset");
            return;
        };
        let Some(payload) = ops.payloads.get(usize::from(op_index)) else {
            self.resolved = None;
            self.unavailable_reason =
                Some("requested operating point index is out of range for active OPS");
            return;
        };
        let mut per_xlayer = Vec::with_capacity(payload.xlayer_entries.len());
        for entry in &payload.xlayer_entries {
            let Ok(masks) =
                resolve_entry_masks(&self.sets, key_xlayer, ops_id, op_index, entry.xlayer_id)
            else {
                self.resolved = None;
                self.unavailable_reason =
                    Some("operating point inherits from an unavailable operating point");
                return;
            };
            per_xlayer.push((entry.xlayer_id, masks));
        }
        let xlayer_mask = payload.xlayer_map.unwrap_or((1u32 << 31) - 1);
        self.resolved = Some(ResolvedOpsTarget {
            xlayer_mask,
            per_xlayer,
        });
        self.ever_resolved = true;
        self.unavailable_reason = None;
    }
}

pub(crate) fn validate_ops_semantics(ops: &OperatingPointSet, offset: ByteOffset) -> Result<()> {
    if ops.has_nonzero_local_reserved_bits() {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::sequence_header_conformance(
                offset,
                "6.10.2",
                format!(
                    "local OPS {} has non-zero ops_reserved_2bits",
                    ops.xlayer_id.get()
                ),
            ),
        });
    }
    if ops.has_reserved_mlayer_info_idc() {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::sequence_header_conformance(
                offset,
                "6.10.2",
                format!(
                    "global OPS {} uses reserved ops_mlayer_info_idc 3",
                    ops.ops_id
                ),
            ),
        });
    }
    for payload in &ops.payloads {
        validate_ops_payload(ops, payload, offset)?;
    }
    Ok(())
}

fn validate_ops_payload(
    ops: &OperatingPointSet,
    payload: &OperatingPointPayload,
    offset: ByteOffset,
) -> Result<()> {
    if payload.has_size_mismatch() {
        return Err(DecodeError::MalformedSource {
            issue: DecodeSourceIssue::sequence_header_conformance(
                offset,
                "6.10.2",
                format!(
                    "OPS {} payload {} declared {} bytes but parsed {} bytes",
                    ops.ops_id,
                    payload.index,
                    payload.declared_size_bytes,
                    payload.computed_size_bytes
                ),
            ),
        });
    }
    for entry in &payload.xlayer_entries {
        if let Some(ptl) = &entry.ptl_info
            && ptl.reserved_2bits != 0
        {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    offset,
                    "6.10.4",
                    format!(
                        "OPS {} payload {} xlayer {} has non-zero ops_ptl_reserved_2bits",
                        ops.ops_id,
                        payload.index,
                        entry.xlayer_id.get()
                    ),
                ),
            });
        }
        if let OpsMlayerSource::Inherited {
            embedded_ops_id,
            embedded_op_index,
        } = entry.mlayer
            && embedded_ops_id == ops.ops_id
            && (embedded_op_index >= ops.ops_cnt || embedded_op_index >= payload.index)
        {
            return Err(DecodeError::MalformedSource {
                issue: DecodeSourceIssue::sequence_header_conformance(
                    offset,
                    "6.10.2",
                    format!(
                        "OPS {} payload {} self-inheritance index {} is invalid",
                        ops.ops_id, payload.index, embedded_op_index
                    ),
                ),
            });
        }
    }
    Ok(())
}

fn resolve_entry_masks(
    sets: &BTreeMap<(u8, u8), OperatingPointSet>,
    key_xlayer: u8,
    mut cur_ops_id: u8,
    mut cur_op_index: u8,
    target_xlayer: ExtendedLayerId,
) -> core::result::Result<[u8; 8], ()> {
    for _ in 0..MAX_INHERITANCE_DEPTH {
        let ops = sets.get(&(key_xlayer, cur_ops_id)).ok_or(())?;
        let payload = ops.payloads.get(usize::from(cur_op_index)).ok_or(())?;
        let entry = payload
            .xlayer_entries
            .iter()
            .find(|entry| entry.xlayer_id == target_xlayer)
            .ok_or(())?;
        match &entry.mlayer {
            OpsMlayerSource::Absent => return Ok([0x0F; 8]),
            OpsMlayerSource::Explicit(info) => return Ok(masks_from_ops_mlayer(info)),
            OpsMlayerSource::Inherited {
                embedded_ops_id,
                embedded_op_index,
            } => {
                if *embedded_ops_id == cur_ops_id && *embedded_op_index >= cur_op_index {
                    return Err(());
                }
                cur_ops_id = *embedded_ops_id;
                cur_op_index = *embedded_op_index;
            }
            _ => return Err(()),
        }
    }
    Err(())
}

fn masks_from_ops_mlayer(info: &OpsMlayerInfo) -> [u8; 8] {
    let mut masks = [0u8; 8];
    for &(mlayer, tlayer_mask) in &info.tlayer_maps {
        if mlayer < 8 && (info.mlayer_map & (1u8 << mlayer)) != 0 {
            masks[usize::from(mlayer)] = tlayer_mask & 0x0F;
        }
    }
    masks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_resolves_inherited_global_membership_from_stream() {
        let stream = include_bytes!(
            "../../../../tests/conformance/vectors/valid/syn-msdo-ops-random-access.obu"
        );
        let obus = splot_core::annexb::parse_annex_b_obus(stream).unwrap();
        let selection = OperatingPointSelection::Global {
            ops_id: 1,
            op_index: 0,
        };
        let mut tracker = OperatingPointTracker::new(selection).unwrap();
        let x3 = ExtendedLayerId::from_bits(3);
        let x7 = ExtendedLayerId::from_bits(7);
        let m0 = EmbeddedLayerId::BASE;
        let t0 = TemporalLayerId::BASE;
        assert_eq!(tracker.contains(x3, m0, t0), None);
        tracker.observe(&obus[3]).unwrap();
        assert_eq!(tracker.contains(x3, m0, t0), None);
        tracker.observe(&obus[2]).unwrap();
        assert_eq!(tracker.contains(x3, m0, t0), Some(true));
        assert_eq!(tracker.contains(x7, m0, t0), Some(true));
        assert_eq!(tracker.contains(GLOBAL_XLAYER_ID, m0, t0), Some(false));
        assert_eq!(
            tracker.contains(x3, EmbeddedLayerId::from_bits(1), t0),
            Some(false)
        );
        assert_eq!(tracker.unavailable_reason(), None);
    }
}
