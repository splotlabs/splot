// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! OBU-header layer-id constraints from AV2 § 6.2.2: global-xlayer requirements,
//! base-layer-only OBU types, and temporal-layer-zero-only OBU types.

use splot_core::annexb::ObuEnvelope;

use super::{Check, emit};
use crate::diagnostic::{Severity, ValidationReport};

/// `OBU_MSDO` / `OBU_TEMPORAL_DELIMITER` must use `obu_xlayer_id == GLOBAL_XLAYER_ID` (§ 6.2.2).
pub(super) struct GlobalXLayerRequired;

impl Check for GlobalXLayerRequired {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-required"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_global_xlayer() && !header.extended_layer_id.is_global() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_xlayer_id == GLOBAL_XLAYER_ID (31), found {}",
                    header.obu_type.spec_name(),
                    header.extended_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` requires base embedded and temporal layers (§ 6.2.2).
pub(super) struct GlobalXLayerRequiresBaseLayers;

impl Check for GlobalXLayerRequiresBaseLayers {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-requires-base-layers"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global()
            && (header.embedded_layer_id.get() != 0 || header.temporal_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "obu_xlayer_id == GLOBAL_XLAYER_ID requires obu_mlayer_id and obu_tlayer_id == 0 \
                     (found mlayer={}, tlayer={})",
                    header.embedded_layer_id.get(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}

/// `obu_xlayer_id == GLOBAL_XLAYER_ID` is only allowed for certain OBU types (§ 6.2.2).
pub(super) struct GlobalXLayerAllowedTypes;

impl Check for GlobalXLayerAllowedTypes {
    fn id(&self) -> &'static str {
        "obu-header/global-xlayer-allowed-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.extended_layer_id.is_global() && !header.obu_type.permits_global_xlayer() {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} is not permitted to use obu_xlayer_id == GLOBAL_XLAYER_ID",
                    header.obu_type.spec_name()
                ),
            );
        }
    }
}

/// Sequence header, temporal delimiter, LCR, OPS, and atlas segment must be base-layer (§ 6.2.2).
pub(super) struct BaseLayerOnlyTypes;

impl Check for BaseLayerOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/base-layer-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_and_embedded_layer()
            && (header.temporal_layer_id.get() != 0 || header.embedded_layer_id.get() != 0)
        {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id and obu_mlayer_id == 0 (found tlayer={}, mlayer={})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get(),
                    header.embedded_layer_id.get()
                ),
            );
        }
    }
}

/// Closed/open-loop key, switch, and RAS frames must have `obu_tlayer_id == 0` (§ 6.2.2).
pub(super) struct TemporalLayerZeroOnlyTypes;

impl Check for TemporalLayerZeroOnlyTypes {
    fn id(&self) -> &'static str {
        "obu-header/temporal-layer-zero-only-types"
    }

    fn spec_section(&self) -> Option<&'static str> {
        Some("6.2.2")
    }

    fn run(&self, obu: &ObuEnvelope<'_>, report: &mut ValidationReport) {
        let header = &obu.header;
        if header.obu_type.requires_base_temporal_layer() && header.temporal_layer_id.get() != 0 {
            emit(
                report,
                self,
                Severity::Error,
                obu,
                format!(
                    "{} requires obu_tlayer_id == 0 (found {})",
                    header.obu_type.spec_name(),
                    header.temporal_layer_id.get()
                ),
            );
        }
    }
}
