// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! High-level syntax availability and generic HLS reference helpers.

use super::*;

/// Availability of in-band HLS objects, for the § 7.3.8 reference checks.
///
/// Sequence-header availability (§ 7.3.8.6), multi-frame-header availability
/// (§ 7.3.8.7), layer-configuration-record availability (§ 7.3.8.3), and local
/// atlas-segment availability (§ 7.3.8.4) are modeled. MSDO / OPS availability records
/// and the global atlas reference (§ 7.3.8.4 uses "can be available", not a hard
/// requirement) remain deferred.
///
/// The store is kept **monotonic** (entries are never removed): an object included
/// earlier in the bitstream stays available, so the validator never falsely reports
/// it unavailable. AV2 § 7.3.8.1's "HLS OBUs must be resent at each random access
/// point" requirement needs random-access state to model and is intentionally not
/// enforced (a sound-over-complete false negative).
#[derive(Debug, Default)]
pub(super) struct HlsAvailabilityStore {
    /// `seq_header_id` values of sequence headers seen in-band so far (§ 7.3.8.6).
    pub(super) sequence_header_ids: BTreeSet<u32>,
    /// Multi-frame-header records keyed by `mfhId`, seen in-band so far (§ 7.3.8.7).
    pub(super) multi_frame_headers: BTreeMap<u32, MultiFrameHeaderRecord>,
    /// `lcr_xlayer_map` of each global LCR, keyed by `lcr_global_config_record_id`,
    /// seen in-band so far (§ 7.3.8.3).
    pub(super) global_lcr_xlayer_maps: BTreeMap<u8, u32>,
    /// `lcr_local_id` values of local LCRs, keyed by their `obu_xlayer_id`, seen
    /// in-band so far (§ 7.3.8.3).
    pub(super) local_lcr_ids: BTreeMap<ExtendedLayerId, BTreeSet<u8>>,
    /// § 5.8.8 embedded-layer maps of global LCR payloads, keyed by
    /// `(lcr_global_config_record_id, xId)`, for the § 6.8.9 dependency-map
    /// agreement checks. A redefinition overwrites the maps, mirroring
    /// [`Self::record_global_lcr`].
    pub(super) global_lcr_embedded: BTreeMap<(u8, ExtendedLayerId), LcrEmbeddedMaps>,
    /// § 5.8.8 embedded-layer maps of local LCRs, keyed by
    /// `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.9 dependency-map agreement
    /// checks.
    pub(super) local_lcr_embedded: BTreeMap<(ExtendedLayerId, u8), LcrEmbeddedMaps>,
    /// § 5.8.4 `lcr_seq_profile_tier_level_info(xlayerId)` declared maxima of local
    /// LCRs, keyed by `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.5 PTL-ceiling
    /// agreement checks. The § 6.8.5 sentences key the ceiling on the *local* LCR
    /// ("associated with the local LCR ... indicated in an extended layer with
    /// obu_xlayer_id equal to i"). Present only when the local record carried
    /// `lcr_profile_tier_level_info_present_flag == 1`; a redefinition replaces the
    /// entry wholesale (see [`Self::clear_local_lcr_extras`]).
    pub(super) local_lcr_ptl: BTreeMap<(ExtendedLayerId, u8), LcrPtlSnapshot>,
    /// § 5.8.4 `lcr_seq_profile_tier_level_info(i)` declared maxima of global LCRs,
    /// keyed by `(lcr_global_config_record_id, obu_xlayer_id)`, for the § 6.8.5
    /// PTL-ceiling agreement checks when the activated record is a global LCR. Present
    /// only when the global record carried `lcr_seq_profile_tier_level_info_present_flag
    /// == 1` for that xlayer; a redefinition clears and re-records this id's entries.
    pub(super) global_lcr_ptl: BTreeMap<(u8, ExtendedLayerId), LcrPtlSnapshot>,
    /// § 5.8.7 `lcr_rep_info(0, xId)` of local LCRs, keyed by
    /// `(obu_xlayer_id, lcr_local_id)`, for the § 6.8.8 rep-info equality agreement
    /// checks. Present only when the local record's `lcr_xlayer_info` carried rep info;
    /// a redefinition replaces the entry wholesale.
    pub(super) local_lcr_rep_info: BTreeMap<(ExtendedLayerId, u8), LcrRepInfoSnapshot>,
    /// § 5.8.7 `lcr_rep_info(1, xId)` of global LCR payloads, keyed by
    /// `(lcr_global_config_record_id, obu_xlayer_id)`, for the § 6.8.8 rep-info
    /// equality agreement checks when the activated record is a global LCR. Present only
    /// for an xlayer whose global payload carried rep info; a redefinition clears and
    /// re-records this id's entries.
    pub(super) global_lcr_rep_info: BTreeMap<(u8, ExtendedLayerId), LcrRepInfoSnapshot>,
    /// `(obu_xlayer_id, atlas_segment_id)` of local atlas segment OBUs seen in-band so
    /// far (§ 7.3.8.4).
    pub(super) local_atlases: BTreeSet<(ExtendedLayerId, u8)>,
}

/// How a referenced HLS object resolves against available objects (AV2 § 7.3.8).
pub(super) enum HlsResolution {
    /// Available in the bitstream.
    InBand,
    /// Available only through caller-provided external HLS.
    External,
    /// Not available by any modeled means.
    Unavailable,
}

impl HlsAvailabilityStore {
    /// Records a sequence header (by `seq_header_id`) as available in-band
    /// (AV2 § 7.3.8.6).
    pub(super) fn record_sequence_header(&mut self, seq_header_id: u32) {
        self.sequence_header_ids.insert(seq_header_id);
    }

    /// Resolves a `seq_header_id` reference against in-band then caller-provided
    /// external availability (AV2 § 7.3.8.6).
    pub(super) fn resolve_sequence_header(
        &self,
        id: u32,
        options: &ValidationOptions,
    ) -> HlsResolution {
        if self.sequence_header_ids.contains(&id) {
            return HlsResolution::InBand;
        }
        if let ExternalHlsMode::Provided(set) = &options.external_hls
            && set.has_sequence_header(id)
        {
            return HlsResolution::External;
        }
        HlsResolution::Unavailable
    }

    /// Records a multi-frame header as available in-band, keyed by `mfhId`
    /// (AV2 § 7.3.8.7). A later redefinition of the same id overwrites the record but
    /// keeps the id available, preserving monotonic availability.
    // `record` is moved straight into `multi_frame_headers`; taking it by reference would
    // force a clone of the 432-byte record, so by-value is the zero-copy choice here.
    #[allow(clippy::large_types_passed_by_value)]
    pub(super) fn record_multi_frame_header(&mut self, record: MultiFrameHeaderRecord) {
        self.multi_frame_headers.insert(record.mfh_id.get(), record);
    }

    /// Returns the in-band multi-frame-header record for `mfhId`, if available.
    ///
    /// External multi-frame-header availability is not modeled (`ValidationOptions`
    /// declares only external sequence headers); it remains future work.
    pub(super) fn multi_frame_header(&self, id: MfhId) -> Option<&MultiFrameHeaderRecord> {
        self.multi_frame_headers.get(&id.get())
    }

    /// Records a global LCR (by `lcr_global_config_record_id`) and its `lcr_xlayer_map`
    /// as available in-band (AV2 § 7.3.8.3). A redefinition overwrites the map but
    /// keeps the id available, preserving monotonic availability.
    pub(super) fn record_global_lcr(&mut self, global_id: u8, xlayer_map: u32) {
        self.global_lcr_xlayer_maps.insert(global_id, xlayer_map);
    }

    /// Returns the `lcr_xlayer_map` of the available global LCR with
    /// `lcr_global_config_record_id == global_id`, if any (AV2 § 7.3.8.3).
    pub(super) fn global_lcr_xlayer_map(&self, global_id: u8) -> Option<u32> {
        self.global_lcr_xlayer_maps.get(&global_id).copied()
    }

    /// Records a local LCR (by `obu_xlayer_id` and `lcr_local_id`) as available in-band
    /// (AV2 § 7.3.8.3).
    pub(super) fn record_local_lcr(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_ids
            .entry(xlayer)
            .or_default()
            .insert(local_id);
    }

    /// Returns `true` if a local LCR with `lcr_local_id == local_id` is available in
    /// the extended layer `xlayer` (AV2 § 7.3.8.3).
    pub(super) fn has_local_lcr(&self, xlayer: ExtendedLayerId, local_id: u8) -> bool {
        self.local_lcr_ids
            .get(&xlayer)
            .is_some_and(|ids| ids.contains(&local_id))
    }

    /// Drops every stored embedded-layer map of the global LCR `global_id`. Called
    /// before re-recording a redefined global LCR so a payload set that drops an
    /// xlayer (or its embedded-layer info) cannot leave stale maps behind — the
    /// § 6.8.9 checks must only ever see the latest definition.
    pub(super) fn clear_global_lcr_embedded(&mut self, global_id: u8) {
        self.global_lcr_embedded
            .retain(|(id, _), _| *id != global_id);
    }

    /// Drops the stored embedded-layer maps of the local LCR `(xlayer, local_id)`;
    /// see [`Self::clear_global_lcr_embedded`].
    pub(super) fn clear_local_lcr_embedded(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_embedded.remove(&(xlayer, local_id));
    }

    /// Records a global LCR payload's § 5.8.8 embedded-layer maps for extended layer
    /// `xlayer` (§ 6.8.9 agreement checks).
    pub(super) fn record_global_lcr_embedded(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        maps: LcrEmbeddedMaps,
    ) {
        self.global_lcr_embedded.insert((global_id, xlayer), maps);
    }

    /// Returns the available global LCR's § 5.8.8 embedded-layer maps for
    /// `(global_id, xlayer)`, if signalled.
    pub(super) fn global_lcr_embedded(
        &self,
        global_id: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<&LcrEmbeddedMaps> {
        self.global_lcr_embedded.get(&(global_id, xlayer))
    }

    /// Records a local LCR's § 5.8.8 embedded-layer maps (§ 6.8.9 agreement checks).
    pub(super) fn record_local_lcr_embedded(
        &mut self,
        xlayer: ExtendedLayerId,
        local_id: u8,
        maps: LcrEmbeddedMaps,
    ) {
        self.local_lcr_embedded.insert((xlayer, local_id), maps);
    }

    /// Returns the available local LCR's § 5.8.8 embedded-layer maps for
    /// `(xlayer, local_id)`, if signalled.
    pub(super) fn local_lcr_embedded(
        &self,
        xlayer: ExtendedLayerId,
        local_id: u8,
    ) -> Option<&LcrEmbeddedMaps> {
        self.local_lcr_embedded.get(&(xlayer, local_id))
    }

    /// Drops the stored § 6.8.5 PTL and § 6.8.8 rep-info snapshots of the local LCR
    /// `(xlayer, local_id)` before re-recording a redefinition, mirroring
    /// [`Self::clear_local_lcr_embedded`] — a re-sent record that drops the PTL or
    /// rep-info must not leave stale entries for the § 6.8.5/§ 6.8.8 checks.
    pub(super) fn clear_local_lcr_extras(&mut self, xlayer: ExtendedLayerId, local_id: u8) {
        self.local_lcr_ptl.remove(&(xlayer, local_id));
        self.local_lcr_rep_info.remove(&(xlayer, local_id));
    }

    /// Drops every stored § 6.8.5 PTL and § 6.8.8 rep-info snapshot of the global LCR
    /// `global_id` before re-recording a redefinition, mirroring
    /// [`Self::clear_global_lcr_embedded`].
    pub(super) fn clear_global_lcr_extras(&mut self, global_id: u8) {
        self.global_lcr_ptl.retain(|(id, _), _| *id != global_id);
        self.global_lcr_rep_info
            .retain(|(id, _), _| *id != global_id);
    }

    /// Records a local LCR's § 5.8.4 PTL declared maxima (§ 6.8.5 ceiling checks).
    pub(super) fn record_local_lcr_ptl(
        &mut self,
        xlayer: ExtendedLayerId,
        local_id: u8,
        ptl: LcrPtlSnapshot,
    ) {
        self.local_lcr_ptl.insert((xlayer, local_id), ptl);
    }

    /// Returns the available local LCR's § 5.8.4 PTL declared maxima for
    /// `(xlayer, local_id)`, if signalled.
    pub(super) fn local_lcr_ptl(
        &self,
        xlayer: ExtendedLayerId,
        local_id: u8,
    ) -> Option<&LcrPtlSnapshot> {
        self.local_lcr_ptl.get(&(xlayer, local_id))
    }

    /// Records a global LCR's § 5.8.4 PTL declared maxima for extended layer `xlayer`
    /// (§ 6.8.5 ceiling checks).
    pub(super) fn record_global_lcr_ptl(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        ptl: LcrPtlSnapshot,
    ) {
        self.global_lcr_ptl.insert((global_id, xlayer), ptl);
    }

    /// Returns the available global LCR's § 5.8.4 PTL declared maxima for
    /// `(global_id, xlayer)`, if signalled.
    pub(super) fn global_lcr_ptl(
        &self,
        global_id: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<&LcrPtlSnapshot> {
        self.global_lcr_ptl.get(&(global_id, xlayer))
    }

    /// Records a local LCR's § 5.8.7 rep info (§ 6.8.8 equality checks).
    pub(super) fn record_local_lcr_rep_info(
        &mut self,
        xlayer: ExtendedLayerId,
        local_id: u8,
        rep: LcrRepInfoSnapshot,
    ) {
        self.local_lcr_rep_info.insert((xlayer, local_id), rep);
    }

    /// Returns the available local LCR's § 5.8.7 rep info for `(xlayer, local_id)`, if
    /// signalled.
    pub(super) fn local_lcr_rep_info(
        &self,
        xlayer: ExtendedLayerId,
        local_id: u8,
    ) -> Option<&LcrRepInfoSnapshot> {
        self.local_lcr_rep_info.get(&(xlayer, local_id))
    }

    /// Records a global LCR payload's § 5.8.7 rep info for extended layer `xlayer`
    /// (§ 6.8.8 equality checks).
    pub(super) fn record_global_lcr_rep_info(
        &mut self,
        global_id: u8,
        xlayer: ExtendedLayerId,
        rep: LcrRepInfoSnapshot,
    ) {
        self.global_lcr_rep_info.insert((global_id, xlayer), rep);
    }

    /// Returns the available global LCR's § 5.8.7 rep info for `(global_id, xlayer)`, if
    /// signalled.
    pub(super) fn global_lcr_rep_info(
        &self,
        global_id: u8,
        xlayer: ExtendedLayerId,
    ) -> Option<&LcrRepInfoSnapshot> {
        self.global_lcr_rep_info.get(&(global_id, xlayer))
    }

    /// Records a local atlas segment OBU (by `obu_xlayer_id` and `atlas_segment_id`)
    /// as available in-band (AV2 § 7.3.8.4).
    pub(super) fn record_local_atlas(&mut self, xlayer: ExtendedLayerId, atlas_id: u8) {
        self.local_atlases.insert((xlayer, atlas_id));
    }

    /// Returns `true` if a local atlas segment OBU with `atlas_segment_id == atlas_id`
    /// is available in the extended layer `xlayer` (AV2 § 7.3.8.4).
    pub(super) fn has_local_atlas(&self, xlayer: ExtendedLayerId, atlas_id: u8) -> bool {
        self.local_atlases.contains(&(xlayer, atlas_id))
    }
}

/// `true` when caller-provided external HLS declares at least one sequence header.
/// An externally activated sequence header has unmodeled dependency maps, so every
/// "activated sequence header" agreement check is unreliable and suppressed
/// (precedent: [`ValidatorContext::validate_active_sequence_limits`]).
pub(super) fn external_declares_sequence_header(options: &ValidationOptions) -> bool {
    if let ExternalHlsMode::Provided(set) = &options.external_hls {
        set.declares_any_sequence_header()
    } else {
        false
    }
}

/// Builds the advisory `hls/external-hls-disabled` warning (AV2 § 7.3.8.1) for a
/// sequence-header reference that is unavailable in-band under the default
/// (external-disabled) options.
pub(super) fn external_hls_disabled_advisory(
    seq_header_id: u32,
    obu: &ObuEnvelope<'_>,
) -> Diagnostic {
    // The finding assumes no external HLS. If the referenced sequence header is
    // supplied out-of-band, the caller can declare it via ValidationOptions to refine
    // the check (AV2 § 7.3.8.1).
    Diagnostic::warning(
        "hls/external-hls-disabled",
        format!(
            "sequence header {seq_header_id} is not available in-band and external HLS is \
             disabled; supply it via ValidationOptions if it is provided through external means"
        ),
    )
    .with_spec_section("7.3.8.1")
    .with_byte_offset(obu.offset)
}

impl ValidatorContext {
    /// Observes a multi-frame header OBU and checks that the sequence header it
    /// references via `mfh_seq_header_id` is available in-band or through
    /// caller-provided external HLS (AV2 § 7.3.8.6 / § 7.3.8.7).
    pub(super) fn observe_multi_frame_header(
        &mut self,
        obu: &ObuEnvelope<'_>,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and the mfh_seq_header_id range check are handled by the
        // stateless MultiFrameHeaderSyntax check; here we only resolve the reference.
        let Ok(mfh) = parse_multi_frame_header(&mut reader) else {
            return;
        };
        // An out-of-range id (>= MAX_SEQ_NUM) cannot name a valid sequence header and
        // is already flagged as mfh/seq-header-id-out-of-range; do not double-report.
        if !mfh.seq_header_id_in_range() {
            return;
        }

        let id = mfh.mfh_seq_header_id;
        match self.hls.resolve_sequence_header(id, options) {
            HlsResolution::InBand | HlsResolution::External => {}
            HlsResolution::Unavailable => {
                let external_note = if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    " (external HLS is disabled)"
                } else {
                    " in-band or through the supplied external HLS"
                };
                report.push(
                    Diagnostic::error(
                        "mfh/sequence-header-unavailable",
                        format!(
                            "multi-frame header references mfh_seq_header_id {id}, but no sequence \
                             header with that id is available{external_note}"
                        ),
                    )
                    .with_spec_section("7.3.8.6")
                    .with_byte_offset(obu.offset),
                );
                if matches!(options.external_hls, ExternalHlsMode::Disabled) {
                    report.push(external_hls_disabled_advisory(id, obu));
                }
            }
        }

        // Gate availability on the same validation the SequenceHeaderSyntax /
        // observe_sequence_header path uses: a fully parsed MFH (now including
        // seg_info()) must have a valid §5.2.1 payload tail (obu_extension_flag +
        // trailing_bits). A malformed tail makes the MFH not a valid available HLS
        // object, so a later cur_mfh_id reference must treat it as unavailable rather
        // than resolve through it.
        if finish_obu_payload(
            &mut reader,
            obu.payload,
            obu.header.obu_type.is_extensible_obu(),
        )
        .is_err()
        {
            return;
        }

        // Record this multi-frame header's in-band availability (AV2 § 7.3.8.7) so a
        // later frame header's cur_mfh_id reference can resolve it. Both ids are in
        // range here (mfh_seq_header_id checked above; mfhId checked now). The MFH is
        // recorded even when its own sequence-header reference is unavailable — that
        // is a separate finding above, not a reason to treat the MFH as absent.
        if mfh.mfh_id_in_range()
            && let Some(seq_id) = SequenceHeaderId::try_new(mfh.mfh_seq_header_id)
            && let Ok(mfh_id_value) = u32::try_from(mfh.mfh_id())
        {
            self.hls.record_multi_frame_header(MultiFrameHeaderRecord {
                mfh_id: MfhId::from_raw(mfh_id_value),
                mfh_seq_header_id: seq_id,
                mfh_tlayer_id: obu.header.temporal_layer_id,
                mfh_mlayer_id: obu.header.embedded_layer_id,
                // Carry the parsed §5.7 state a `cur_mfh_id > 0` frame header consumes at
                // §5.18.2 (default frame dimensions and the §5.18.7.1 segmentation arm)
                // and the deblocking-update groundwork bit.
                mfh_frame_size: mfh.mfh_frame_size,
                mfh_seg_info_present_flag: mfh.mfh_seg_info_present_flag,
                mfh_ext_seg_flag: mfh.mfh_ext_seg_flag,
                mfh_allow_seg_info_change: mfh.mfh_allow_seg_info_change,
                mfh_segment_info: mfh.segment_info,
                mfh_deblocking_filter_update: mfh.mfh_deblocking_filter_update,
                mfh_apply_deblocking_filter: mfh.mfh_apply_deblocking_filter,
                offset: obu.offset,
            });
            // AV2 § 7.3.8.1: note this MFH's in-band (re)send for the replay, and — when
            // its mfh_seq_header_id resolved in-band (so the linear check did not fire and
            // external HLS did not suppress) — buffer the § 7.3.8.6 sequence-header
            // reference this MFH makes (a MFH at a random access point references a
            // sequence header that must itself be available at that point).
            self.rap_replay.note_resend(
                RapHlsKey::MultiFrameHeader(mfh_id_value),
                obu.header.extended_layer_id,
            );
            if matches!(
                self.hls.resolve_sequence_header(id, options),
                HlsResolution::InBand
            ) {
                self.note_rap_reference(
                    RapHlsKey::SequenceHeader(id),
                    obu.header.extended_layer_id,
                    obu.offset,
                );
            }
        }
    }

    /// Observes an atlas segment info OBU and records a local atlas segment's in-band
    /// availability (AV2 § 7.3.8.4). A global atlas (§ 7.3.8.4 uses "can be available",
    /// not a hard requirement) is not recorded. Recording is gated on a successful
    /// parse and a valid § 5.2.1 payload tail.
    pub(super) fn observe_atlas_segment(&mut self, obu: &ObuEnvelope<'_>) {
        let mut reader = BitReader::new(obu.payload, obu.payload_offset());
        // Parse failures and syntax diagnostics are handled by the stateless
        // AtlasSegmentSyntax check; here we only record availability.
        let Ok(atlas) = parse_atlas_segment(&mut reader) else {
            return;
        };
        if finish_obu_payload(
            &mut reader,
            obu.payload,
            obu.header.obu_type.is_extensible_obu(),
        )
        .is_err()
        {
            return;
        }

        let xlayer = obu.header.extended_layer_id;
        if !xlayer.is_global() {
            self.hls.record_local_atlas(xlayer, atlas.atlas_segment_id);
            // AV2 § 7.3.8.1: note this *local* atlas segment's in-band (re)send (its own
            // extended layer) for the random-access-point availability replay, so a local
            // LCR's lcr_local_atlas_id referencing it at a random access point must find it
            // resent there. A global atlas is excluded (§ 7.3.8.4 "can be available").
            self.rap_replay.note_resend(
                RapHlsKey::Atlas {
                    xlayer: xlayer.get(),
                    id: atlas.atlas_segment_id,
                },
                xlayer,
            );
        }
    }
}
