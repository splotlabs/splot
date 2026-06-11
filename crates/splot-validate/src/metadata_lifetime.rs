// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Metadata persistence / cancellation lifetime model (AV2 v1.0.0 § 6.16.3,
//! `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-3`).
//!
//! `muh_persistence_idc` "is used to signal the mode in which the signaled metadata
//! persists over time. This value can represent different modes, such as global
//! persistence for the entire video sequence, persistence for a group of frames of
//! a certain duration, or persistence for a single frame only" (§ 6.16.3), and
//! `muh_cancel_flag` "when set to 1, indicates that any previously signaled
//! metadata information for a metadata with type equal to muh_metadata_type is
//! cancelled for either the current extended layer if obu_xlayer_id is less than
//! GLOBAL_XLAYER_ID, or for a set of extended layers if obu_xlayer_id is equal to
//! GLOBAL_XLAYER_ID" (§ 6.16.3).
//!
//! The [`MetadataLifetimeStore`] models that lifetime so the validator (and later
//! per-frame applicability checks) can reason about which metadata is active where.
//! The store **never emits diagnostics**: § 6.16.3 attaches no bitstream
//! conformance requirement to the lifetime rules themselves — its only "shall"
//! ("Decoders shall ignore metadata that does not apply to the current operating
//! point based on these rules") constrains decoder applicability, not the
//! bitstream.

use std::collections::BTreeMap;

use splot_core::headers::metadata::{MetadataPayload, MetadataType};
use splot_core::headers::sequence::{MLayerDependencyMap, TLayerDependencyMap};
use splot_core::span::ByteOffset;
use splot_core::types::{EmbeddedLayerId, ExtendedLayerId, TemporalLayerId};

/// `muh_layer_idc` value `LAYER_GLOBAL` (AV2 § 6.16.3): "The metadata applies to
/// all layers if obu_xlayer_id is equal to GLOBAL_XLAYER_ID. If obu_xlayer_id is
/// less than GLOBAL_XLAYER_ID, layers with matching obu_xlayer_id only."
pub(crate) const LAYER_GLOBAL: u8 = 1;

/// `muh_layer_idc` value `LAYER_CURRENT` (AV2 § 6.16.3): "The metadata applies to
/// the current layer only as indicated by the specific values for obu_xlayer_id
/// and obu_mlayer_id in OBU header."
pub(crate) const LAYER_CURRENT: u8 = 2;

/// `muh_layer_idc` value `LAYER_VALUES` (AV2 § 6.16.3): "The metadata applies to a
/// set of specific layer values, which are explicitly signaled."
pub(crate) const LAYER_VALUES: u8 = 3;

/// `muh_persistence_idc` (AV2 § 6.16.3): "the mode in which the signaled metadata
/// persists over time".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceMode {
    /// `0` — `GLOBAL_PERSISTENCE`: "Global persistence for the entire video
    /// sequence. When this mode is signaled previously signaled global metadata of
    /// this type are overwritten. The cancel flag (muh_cancel_flag) does not do
    /// anything to it." (§ 6.16.3).
    Global,
    /// `1` — `BASIC_PERSISTENCE`: "Persistence until a new metadata unit of the
    /// same type is encountered that applies to the layer or the cancel flag
    /// (muh_cancel_flag) is encountered." (§ 6.16.3).
    Basic,
    /// `2` — `NO_PERSISTENCE`: "Used only for the current frame." (§ 6.16.3).
    No,
    /// `3` — `ENHANCED_PERSISTENCE`: "This one is similar to basic but can allow
    /// updates of metadata without full replacement." (§ 6.16.3). The spec defines
    /// no merge algorithm, so the store treats this as BASIC distinguished only by
    /// this marker.
    Enhanced,
    /// `4`-`7` — "Reserved for AOMedia use." (§ 6.16.3); preserves the raw value.
    /// Stored as-is with no lifetime semantics applied.
    Reserved(u8),
}

impl PersistenceMode {
    /// Classifies a raw `muh_persistence_idc` value (`f(3)`, AV2 § 6.16.3).
    pub(crate) fn from_idc(idc: u8) -> Self {
        match idc {
            0 => Self::Global,
            1 => Self::Basic,
            2 => Self::No,
            3 => Self::Enhanced,
            other => Self::Reserved(other),
        }
    }
}

/// One active metadata unit tracked within a coded video sequence (AV2 § 6.16.3).
///
/// The § 6.16.3 propagation rules name the unit's source position "embedded layer
/// K and temporal layer T"; [`MetadataLifetimeStore::applies_to`] reads those
/// fields back under exactly those names.
#[derive(Debug, Clone)]
pub(crate) struct ActiveMetadataUnit {
    /// `metadata_type` of the unit (AV2 § 6.16.3 Table 6.17).
    pub metadata_type: MetadataType,
    /// The unit's `muh_persistence_idc`, classified (AV2 § 6.16.3).
    pub persistence: PersistenceMode,
    /// `muh_layer_idc` (short form: parsed from the 1-byte header; group form:
    /// per-unit) (AV2 § 6.16.3).
    pub layer_idc: u8,
    /// `obu_mlayer_id` of the carrying OBU — `K` in the § 6.16.3 propagation rules.
    pub source_mlayer: EmbeddedLayerId,
    /// `obu_tlayer_id` of the carrying OBU — `T` in the § 6.16.3 propagation rules.
    pub source_tlayer: TemporalLayerId,
    /// `muh_mlayer_map` for `LAYER_VALUES` explicit targeting (group form only,
    /// AV2 § 5.17.3; `None` for the short form, a non-`LAYER_VALUES` unit, or a
    /// global group unit whose `muh_xlayer_map` selected several per-layer maps).
    pub mlayer_map: Option<u8>,
    /// The typed metadata payload (AV2 § 5.17.4-§ 5.17.13).
    ///
    /// Not read by the store itself: it is the query surface for the validator
    /// test suite and the deferred per-frame metadata checks (tasks 8-9 of the
    /// `metadata-semantic-validation` change).
    #[cfg_attr(not(test), allow(dead_code))]
    pub payload: MetadataPayload,
    /// Source byte offset of the carrying OBU (same test/deferred query surface as
    /// [`ActiveMetadataUnit::payload`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub offset: ByteOffset,
    /// Temporal unit of the observation, used by the exact § 7.3.6
    /// coded-video-sequence scoping (see [`MetadataLifetimeStore::reset_cvs`]).
    pub tu_index: u64,
}

impl ActiveMetadataUnit {
    /// Returns `true` when this unit's `LAYER_VALUES` targeting explicitly names
    /// embedded layer `target` (AV2 § 6.16.3: "muh_mlayer_map contains a bitmask.
    /// The metadata unit is intended for an embedded layer m if bit m of
    /// muh_mlayer_map is equal to 1."; the `LAYER_VALUES` mode itself means "The
    /// metadata applies to a set of specific layer values, which are explicitly
    /// signaled").
    ///
    /// For a target above the unit's source layer `K`, a set bit also implies the
    /// § 6.16.3 NOTE's unit-level "explicit layer persistence indication"
    /// ("muh_layer_idc is equal to LAYER_VALUES (3) and muh_mlayer_map has bits
    /// set for embedded layers greater than obu_mlayer_id"), so the per-target
    /// check subsumes the unit-level one.
    fn explicitly_targets(&self, target: EmbeddedLayerId) -> bool {
        self.layer_idc == LAYER_VALUES
            && self
                .mlayer_map
                // EmbeddedLayerId is 3-bit (0..=7), so the u8 shift stays in range.
                .is_some_and(|map| map >> target.get() & 1 == 1)
    }

    /// Returns `true` when `other` targets the same layer scope as this unit:
    /// equal `muh_layer_idc`, source embedded and temporal layer, and explicit
    /// `muh_mlayer_map` targeting. Used for the conservative BASIC-replacement
    /// reading (see [`MetadataLifetimeStore::observe_unit`]).
    fn same_layer_scope(&self, other: &Self) -> bool {
        self.layer_idc == other.layer_idc
            && self.source_mlayer == other.source_mlayer
            && self.source_tlayer == other.source_tlayer
            && self.mlayer_map == other.mlayer_map
    }
}

/// Per-`(obu_xlayer_id, metadata_type raw value)` active-metadata store
/// (AV2 § 6.16.3).
///
/// Each key holds the units currently active for that extended layer and metadata
/// type; several can coexist (e.g. BASIC units with different layer scopes
/// alongside a GLOBAL one). Records are scoped to the coded video sequence of the
/// keying extended layer (AV2 § 7.3.6, see [`MetadataLifetimeStore::reset_cvs`]).
#[derive(Debug, Default)]
pub(crate) struct MetadataLifetimeStore {
    active: BTreeMap<(ExtendedLayerId, u32), Vec<ActiveMetadataUnit>>,
}

impl MetadataLifetimeStore {
    /// Folds one observed non-cancel metadata unit into the active state, keyed by
    /// the carrying OBU's `obu_xlayer_id` and the unit's `metadata_type`, applying
    /// the § 6.16.3 persistence-mode semantics:
    ///
    /// - `GLOBAL_PERSISTENCE`: "previously signaled global metadata of this type
    ///   are overwritten" — the existing GLOBAL record under this key is replaced.
    /// - `BASIC_PERSISTENCE`: a prior record persists "until a new metadata unit of
    ///   the same type is encountered that applies to the layer". Interpretation
    ///   (conservative): "applies to the layer" is read as an **equal layer scope**
    ///   (same `muh_layer_idc`, source layer ids, and `muh_mlayer_map`), so only
    ///   exact-scope records are replaced — a broader applies-to reading could
    ///   silently drop state the stream still relies on.
    /// - `NO_PERSISTENCE`: "Used only for the current frame" — inserted as-is and
    ///   expired at the next coded frame (see
    ///   [`MetadataLifetimeStore::expire_no_persistence`]).
    /// - `ENHANCED_PERSISTENCE`: "similar to basic but can allow updates of
    ///   metadata without full replacement" — the spec defines no merge algorithm,
    ///   so it follows the BASIC path, distinguished only by the
    ///   [`PersistenceMode::Enhanced`] marker.
    /// - Reserved `4`-`7`: "Reserved for AOMedia use" — stored as-is, no semantics
    ///   applied (the stateless checks separately warn about the reserved value).
    pub(crate) fn observe_unit(&mut self, xlayer: ExtendedLayerId, unit: ActiveMetadataUnit) {
        let records = self
            .active
            .entry((xlayer, unit.metadata_type.value()))
            .or_default();
        match unit.persistence {
            PersistenceMode::Global => {
                records.retain(|record| !matches!(record.persistence, PersistenceMode::Global));
            }
            PersistenceMode::Basic | PersistenceMode::Enhanced => {
                records.retain(|record| {
                    matches!(record.persistence, PersistenceMode::Global)
                        || !record.same_layer_scope(&unit)
                });
            }
            PersistenceMode::No | PersistenceMode::Reserved(_) => {}
        }
        records.push(unit);
    }

    /// Applies a `muh_cancel_flag` unit (AV2 § 6.16.3): "any previously signaled
    /// metadata information for a metadata with type equal to muh_metadata_type is
    /// cancelled for either the current extended layer if obu_xlayer_id is less
    /// than GLOBAL_XLAYER_ID, or for a set of extended layers if obu_xlayer_id is
    /// equal to GLOBAL_XLAYER_ID". (`muh_metadata_type` is an editorial alias for
    /// `metadata_type` — the name appears nowhere in the § 5.17.2 / § 5.17.3
    /// syntax.)
    ///
    /// GLOBAL-persistence records survive: "The cancel flag (muh_cancel_flag) does
    /// not do anything to it" (§ 6.16.3).
    ///
    /// For a global cancel the spec says only "for a set of extended layers"
    /// without defining the set; a group-form cancel unit carries no layer maps
    /// (§ 5.17.3 skips every `muh_*` field but the type when `muh_cancel_flag` is
    /// set), so clearing the type across **all** extended layers is the only
    /// syntactically grounded reading (interpretation surfaced to the maintainer).
    /// A local cancel touches only its own extended layer's key — a record keyed
    /// under `GLOBAL_XLAYER_ID` applies to all layers and cannot be partially
    /// cancelled by removal, so it is left intact (conservative).
    pub(crate) fn cancel(&mut self, xlayer: ExtendedLayerId, metadata_type_raw: u32) {
        if xlayer.is_global() {
            for ((_, type_raw), records) in &mut self.active {
                if *type_raw == metadata_type_raw {
                    records.retain(|record| matches!(record.persistence, PersistenceMode::Global));
                }
            }
        } else if let Some(records) = self.active.get_mut(&(xlayer, metadata_type_raw)) {
            records.retain(|record| matches!(record.persistence, PersistenceMode::Global));
        }
        self.prune_empty();
    }

    /// Expires every `NO_PERSISTENCE` record (AV2 § 6.16.3: "Used only for the
    /// current frame"). Called from the validator's coded-frame hook at each
    /// frame-bearing OBU, which is the coded frame of its frame unit (§ 7.3.5): the
    /// first tile-group OBU of a coded frame, a SEF/TIP/bridge OBU, etc. A coded
    /// frame unit carries at most one such coded frame, so the hook fires exactly
    /// once per coded frame unit per layer — the coded-frame-unit granularity the
    /// store needs. A record from a unit's pre-frame region therefore lapses once
    /// that unit's coded frame has been observed.
    ///
    /// Residual: the expiry is per-coded-frame across all layers, not yet scoped to
    /// the record's own `(xlayer, mlayer)` frame, so a global NO_PERSISTENCE unit at
    /// the start of a temporal unit lapses at the first layer's coded frame rather
    /// than once per consuming layer. No consumer reads the store's per-frame view
    /// yet; the per-frame applicability checks deferred to tasks 8-9 of the
    /// `metadata-semantic-validation` change will scope the expiry to the consuming
    /// layer's frame when they land.
    // TODO(spec: AV2-5.17-METADATA): scope the NO_PERSISTENCE expiry to the
    // record's own layer's coded frame (per-(xlayer,mlayer) granularity) when the
    // deferred per-frame applicability checks (tasks 8-9) consume the store.
    pub(crate) fn expire_no_persistence(&mut self) {
        for records in self.active.values_mut() {
            records.retain(|record| !matches!(record.persistence, PersistenceMode::No));
        }
        self.prune_empty();
    }

    /// Starts a new coded video sequence for `xlayer` at temporal unit `tu_index`
    /// (AV2 § 7.3.6): drops the extended layer's records from earlier temporal
    /// units. Same-temporal-unit records survive — the new coded video sequence
    /// "is defined to start at each temporal unit that contains an OBU with
    /// obu_type equal to OBU_CLOSED_LOOP_KEY" (§ 7.3.6), so they joined it.
    /// Records keyed under `GLOBAL_XLAYER_ID` have no single owning extended layer
    /// and are pruned at every boundary event, mirroring the validator's other
    /// CVS-scoped stores (documented approximation). `GLOBAL_PERSISTENCE` records
    /// are dropped too: § 2 defines no plain "video sequence", so its "the entire
    /// video sequence" is read as the coded video sequence of the keying extended
    /// layer.
    pub(crate) fn reset_cvs(&mut self, xlayer: ExtendedLayerId, tu_index: u64) {
        for ((record_xlayer, _), records) in &mut self.active {
            if *record_xlayer == xlayer || record_xlayer.is_global() {
                records.retain(|record| record.tu_index >= tu_index);
            }
        }
        self.prune_empty();
    }

    /// Returns the active units for `(xlayer, metadata_type_raw)`.
    ///
    /// Query surface for the validator test suite and the deferred per-frame
    /// metadata checks (tasks 8-9 of the `metadata-semantic-validation` change).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn active_units(
        &self,
        xlayer: ExtendedLayerId,
        metadata_type_raw: u32,
    ) -> &[ActiveMetadataUnit] {
        self.active
            .get(&(xlayer, metadata_type_raw))
            .map_or(&[], Vec::as_slice)
    }

    /// Returns whether `record` applies to embedded layer `target_mlayer` (`M`) and
    /// temporal layer `target_tlayer` (`C`), per the § 6.16.3 propagation rules
    /// ("When metadata is indicated as persistent and is specified at embedded
    /// layer K and temporal layer T, the metadata applies to other layers according
    /// to the following rules"):
    ///
    /// - "Temporal persistence: Within embedded layer K, the metadata persists to
    ///   temporal layer C if TLayerDependencyMap\[K\]\[C\]\[T\] is equal to 1. If
    ///   TLayerDependencyMap\[K\]\[C\]\[T\] is equal to 0, the metadata does not
    ///   apply to temporal layer C."
    /// - "Multi-layer persistence: The metadata persists from embedded layer K to
    ///   embedded layer M (where M > K) if the metadata has explicit layer
    ///   persistence indication and MLayerDependencyMap\[M\]\[K\] is equal to 1."
    /// - "Combined persistence: When metadata persists from embedded layer K to
    ///   embedded layer M, it applies to temporal layer C within embedded layer M
    ///   if TLayerDependencyMap\[M\]\[C\]\[T\] is equal to 1."
    ///
    /// The multi-layer bullet's "explicit layer persistence indication" is read
    /// **per target**: the metadata reaches embedded layer `M` only when bit `M`
    /// of `muh_mlayer_map` is set (§ 6.16.3: "The metadata unit is intended for an
    /// embedded layer m if bit m of muh_mlayer_map is equal to 1") AND
    /// `MLayerDependencyMap[M][K]` is 1. Reading the bullet's unit-level NOTE
    /// alone would propagate metadata to layers whose map bit is 0 — layers never
    /// "explicitly signaled" — contradicting the § 6.16.3 `LAYER_VALUES`
    /// definition ("a set of specific layer values, which are explicitly
    /// signaled") and collapsing it into the all-layers / range modes the
    /// `muh_layer_idc` intro lists as distinct. A set bit at `M > K` implies the
    /// NOTE's unit-level indication, so the per-target reading satisfies every
    /// § 6.16.3 sentence simultaneously.
    ///
    /// No bullet reaches an embedded layer below `K`, so `M < K` reads `false`.
    /// Pure query — **never emits diagnostics** (the bullets bind decoder
    /// applicability: "Decoders shall ignore metadata that does not apply to the
    /// current operating point based on these rules", § 6.16.3). The caller
    /// supplies the maps (see `ValidatorContext::metadata_applies_to` for the
    /// active-sequence-header resolution and its fallback).
    pub(crate) fn applies_to(
        record: &ActiveMetadataUnit,
        target_mlayer: EmbeddedLayerId,
        target_tlayer: TemporalLayerId,
        t_map: &TLayerDependencyMap,
        m_map: &MLayerDependencyMap,
    ) -> bool {
        let source_mlayer = record.source_mlayer; // K
        let source_tlayer = record.source_tlayer; // T
        if target_mlayer == source_mlayer {
            // Temporal persistence within embedded layer K.
            return t_map.depends_on(source_mlayer, target_tlayer, source_tlayer);
        }
        if target_mlayer > source_mlayer {
            // Multi-layer persistence (K -> M) plus combined persistence for C:
            // bit M of muh_mlayer_map must explicitly target M (see
            // ActiveMetadataUnit::explicitly_targets).
            return record.explicitly_targets(target_mlayer)
                && m_map.depends_on(target_mlayer, source_mlayer)
                && t_map.depends_on(target_mlayer, target_tlayer, source_tlayer);
        }
        false
    }

    /// Removes keys whose record list became empty, keeping the map tidy.
    fn prune_empty(&mut self) {
        self.active.retain(|_, records| !records.is_empty());
    }
}
