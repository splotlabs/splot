// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! LCR agreement checks against activated sequence headers.

use super::*;

impl ValidatorContext {
    /// AV2 § 6.8.9: the activated LCR's `lcr_mlayer_map[isGlobal][xId]` /
    /// `lcr_tlayer_map[isGlobal][xId][cMId]`, if present, must be
    /// dependency-closed under the activated sequence header's maps. The pairing
    /// is the sequence header activated for `xlayer` and that header's § 6.4.1
    /// LCR association — the snapshot taken at the header's latest observation
    /// (see [`ValidatorContext::lcr_associations`]), NOT a live resolution: a
    /// record redefined after the header is not the associated one. Only the
    /// `xId == xlayer` entry is constrained by this activation. Unresolved
    /// references are owned by the existing § 7.3.8.3 availability diagnostics,
    /// and an association without embedded-layer info has nothing to check. The
    /// diagnostics carry the associated LCR OBU's byte offset.
    pub(super) fn check_lcr_dependency_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let seq_lcr_id = association.lcr_id;
        let Some(maps) = association.maps.as_ref() else {
            return;
        };

        if let Some((curr, reference)) =
            mlayer_closure_violation(maps.mlayer_map, &general.mlayer_dependency_map)
        {
            let key = DependencyFindingKey::Lcr {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id: seq_lcr_id,
                lcr_offset: maps.offset,
                map: DependencyMapKind::Mlayer,
            };
            if self.emitted_dependency_findings.insert(key) {
                report.push(
                    Diagnostic::error(
                        "lcr/mlayer-dependency-missing",
                        format!(
                            "activated {} layer configuration record {seq_lcr_id} includes \
                             embedded layer {curr} but not embedded layer {reference} for \
                             extended layer {}, which the activated sequence header {}'s \
                             MLayerDependencyMap[{curr}][{reference}] requires",
                            if lcr_is_global { "global" } else { "local" },
                            xlayer.get(),
                            seq_header_id.get(),
                        ),
                    )
                    .with_spec_section("6.8.9")
                    .with_byte_offset(maps.offset),
                );
            }
        }

        for &(mlayer, tlayer_mask) in &maps.tlayer_maps {
            let Some((curr, reference)) =
                tlayer_closure_violation(mlayer, tlayer_mask, &general.tlayer_dependency_map)
            else {
                continue;
            };
            let key = DependencyFindingKey::Lcr {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id: seq_lcr_id,
                lcr_offset: maps.offset,
                map: DependencyMapKind::Tlayer { mlayer },
            };
            if self.emitted_dependency_findings.insert(key) {
                report.push(
                    Diagnostic::error(
                        "lcr/tlayer-dependency-missing",
                        format!(
                            "activated {} layer configuration record {seq_lcr_id} includes \
                             temporal layer {curr} of embedded layer {mlayer} but not temporal \
                             layer {reference} for extended layer {}, which the activated \
                             sequence header {}'s \
                             TLayerDependencyMap[{mlayer}][{curr}][{reference}] requires",
                            if lcr_is_global { "global" } else { "local" },
                            xlayer.get(),
                            seq_header_id.get(),
                        ),
                    )
                    .with_spec_section("6.8.9")
                    .with_byte_offset(maps.offset),
                );
            }
        }
    }

    /// AV2 § 6.8.5: when `lcr_seq_profile_tier_level_info(i)` is present in the LCR
    /// activated by extended layer `i`'s frame-confirmed sequence header, the header's
    /// `seq_profile_idc`, `seq_level_idx`, `seq_tier`, and `seq_max_mlayer_cnt_minus_1 +
    /// 1` must each be less than or equal to the corresponding LCR-declared maximum
    /// (`lcr_seq_profile_idc[i]` / `lcr_max_level_idx[i]` / `lcr_tier_flag[i]` /
    /// `lcr_max_mlayer_count[i]`), with equality passing
    /// (mirror `06-syntax-structures-semantics.md#s-6-8-5`, lines 1774-1810).
    ///
    /// The pairing is the sequence header activated for `xlayer` and that header's
    /// § 6.4.1 LCR association (the [`LcrAssociation::ptl`] snapshot taken at the
    /// header's latest observation, NOT a live resolution — a record redefined after the
    /// header is not the associated one). The § 6.8.5 sentence keys the ceiling on the
    /// *local* LCR; the snapshot reads the local record's PTL for a local association and
    /// the global record's PTL for that xlayer for a global one. An association without
    /// PTL info has nothing to check (absent PTL compares nothing), and unresolved
    /// references are owned by the existing § 7.3.8.3 availability diagnostics. The
    /// diagnostics anchor at the associated LCR OBU (its declared maxima are the
    /// informative source). Suppressed under any Provided external-HLS mode (the
    /// association is § 6.4.1-resolved and an unmodeled external local LCR could shadow
    /// the in-band record) and gated on a strict frame-confirmed activation — see
    /// [`Self::check_lcr_dependency_agreement`] for the full rationale.
    pub(super) fn check_lcr_ptl_ceilings(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let Some(ptl) = association.ptl else {
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let lcr_id = association.lcr_id;
        let lcr_offset = ptl.offset;
        let scope = if lcr_is_global { "global" } else { "local" };

        let seq_profile = u32::from(general.seq_profile_idc.get());
        let seq_level = u32::from(general.seq_level_idx.get());
        let seq_tier = u32::from(u8::from(matches!(general.seq_tier, Tier::High)));
        let seq_mlayer_count = u32::from(general.seq_max_mlayer_count.get());

        let checks = [
            (
                LcrPtlField::Profile,
                "lcr/ptl-profile-exceeds-max",
                seq_profile,
                u32::from(ptl.seq_profile_idc),
                "seq_profile_idc",
                "lcr_seq_profile_idc",
            ),
            (
                LcrPtlField::Level,
                "lcr/ptl-level-exceeds-max",
                seq_level,
                u32::from(ptl.max_level_idx),
                "seq_level_idx",
                "lcr_max_level_idx",
            ),
            (
                LcrPtlField::Tier,
                "lcr/ptl-tier-exceeds-max",
                seq_tier,
                u32::from(ptl.tier_flag),
                "seq_tier",
                "lcr_tier_flag",
            ),
            (
                LcrPtlField::MlayerCount,
                "lcr/ptl-mlayer-count-exceeds-max",
                seq_mlayer_count,
                u32::from(ptl.max_mlayer_count),
                "seq_max_mlayer_cnt_minus_1 + 1",
                "lcr_max_mlayer_count",
            ),
        ];

        for (field, rule_id, header_value, lcr_max, header_name, lcr_name) in checks {
            if header_value <= lcr_max {
                continue;
            }
            let key = LcrPtlFindingKey {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id,
                lcr_offset,
                field,
                lcr_max,
                header_value,
            };
            if !self.emitted_lcr_ptl_findings.insert(key) {
                continue;
            }
            report.push(
                Diagnostic::error(
                    rule_id,
                    format!(
                        "sequence header {} activated for extended layer {} has {header_name} \
                         {header_value}, exceeding the activated {scope} layer configuration \
                         record {lcr_id}'s {lcr_name}[{}] = {lcr_max} (§ 6.8.5)",
                        seq_header_id.get(),
                        xlayer.get(),
                        xlayer.get(),
                    ),
                )
                .with_spec_section("6.8.5")
                .with_byte_offset(lcr_offset),
            );
        }
    }

    /// AV2 § 6.8.8: the activated LCR's `lcr_rep_info(isGlobal, j)`, when present, must
    /// agree with each sequence header activated by extended layer `j` — `lcr_max_pic_width`
    /// / `lcr_max_pic_height` equal `max_frame_width/height_minus_1 + 1`,
    /// `lcr_bit_depth_idc` / `lcr_chroma_format_idc` (when
    /// `lcr_format_info_present_flag == 1`) equal `bit_depth_idc` / `chroma_format_idc`,
    /// `lcr_cropping_window_present_flag` equals `seq_cropping_window_present_flag`, and
    /// (when the LCR cropping window is present) the four `lcr_cropping_win_*_offset`
    /// equal the `seq_cropping_win_*_offset` (mirror
    /// `06-syntax-structures-semantics.md#s-6-8-8`, lines 1925-1968). Each disagreement
    /// emits `lcr/rep-info-mismatch` (error) naming the field.
    ///
    /// Same pairing discipline as [`Self::check_lcr_ptl_ceilings`]: the [`LcrAssociation::rep_info`]
    /// snapshot, a strict frame-confirmed activation, absent rep-info (or absent
    /// format-info / cropping window) comparing nothing, and the LCR OBU as the diagnostic
    /// anchor. Likewise suppressed under any Provided external-HLS mode — see
    /// [`Self::check_lcr_dependency_agreement`].
    pub(super) fn check_lcr_rep_info_agreement(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let Some(rep) = association.rep_info else {
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let lcr_id = association.lcr_id;
        let lcr_offset = rep.offset;
        let scope = if lcr_is_global { "global" } else { "local" };

        let mut mismatches: Vec<(LcrRepInfoField, u64, u64, String)> = Vec::new();

        let header_width = general.max_frame_width.get();
        if rep.max_pic_width != header_width {
            mismatches.push((
                LcrRepInfoField::Width,
                u64::from(rep.max_pic_width),
                u64::from(header_width),
                format!(
                    "lcr_max_pic_width {} != max_frame_width_minus_1 + 1 = {header_width}",
                    rep.max_pic_width
                ),
            ));
        }
        let header_height = general.max_frame_height.get();
        if rep.max_pic_height != header_height {
            mismatches.push((
                LcrRepInfoField::Height,
                u64::from(rep.max_pic_height),
                u64::from(header_height),
                format!(
                    "lcr_max_pic_height {} != max_frame_height_minus_1 + 1 = {header_height}",
                    rep.max_pic_height
                ),
            ));
        }

        if let Some((lcr_bit_depth, lcr_chroma)) = rep.format {
            let header_bit_depth = u32::from(general.bit_depth_idc.get());
            if lcr_bit_depth != header_bit_depth {
                mismatches.push((
                    LcrRepInfoField::BitDepth,
                    u64::from(lcr_bit_depth),
                    u64::from(header_bit_depth),
                    format!(
                        "lcr_bit_depth_idc {lcr_bit_depth} != bit_depth_idc {header_bit_depth}"
                    ),
                ));
            }
            let header_chroma = u32::from(general.chroma_format_idc.get());
            if lcr_chroma != header_chroma {
                mismatches.push((
                    LcrRepInfoField::ChromaFormat,
                    u64::from(lcr_chroma),
                    u64::from(header_chroma),
                    format!(
                        "lcr_chroma_format_idc {lcr_chroma} != chroma_format_idc {header_chroma}"
                    ),
                ));
            }
        }

        let lcr_cropping_present = rep.cropping.is_some();
        let header_cropping_present = general.seq_cropping_window_present_flag;
        if lcr_cropping_present != header_cropping_present {
            mismatches.push((
                LcrRepInfoField::CroppingPresent,
                u64::from(lcr_cropping_present),
                u64::from(header_cropping_present),
                format!(
                    "lcr_cropping_window_present_flag {} != seq_cropping_window_present_flag {}",
                    u8::from(lcr_cropping_present),
                    u8::from(header_cropping_present),
                ),
            ));
        }
        if let Some((lcr_left, lcr_right, lcr_top, lcr_bottom)) = rep.cropping {
            let crop = general.cropping_window;
            for (field, lcr_value, header_value, name) in [
                (LcrRepInfoField::CropLeft, lcr_left, crop.left, "left"),
                (LcrRepInfoField::CropRight, lcr_right, crop.right, "right"),
                (LcrRepInfoField::CropTop, lcr_top, crop.top, "top"),
                (
                    LcrRepInfoField::CropBottom,
                    lcr_bottom,
                    crop.bottom,
                    "bottom",
                ),
            ] {
                if lcr_value != header_value {
                    mismatches.push((
                        field,
                        u64::from(lcr_value),
                        u64::from(header_value),
                        format!(
                            "lcr_cropping_win_{name}_offset {lcr_value} != \
                             seq_cropping_win_{name}_offset {header_value}"
                        ),
                    ));
                }
            }
        }

        for (field, lcr_value, header_value, fragment) in mismatches {
            let key = LcrRepInfoFindingKey {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id,
                lcr_offset,
                field,
                lcr_value,
                header_value,
            };
            if !self.emitted_lcr_rep_info_findings.insert(key) {
                continue;
            }
            report.push(
                Diagnostic::error(
                    "lcr/rep-info-mismatch",
                    format!(
                        "activated {scope} layer configuration record {lcr_id}'s rep info for \
                         extended layer {} disagrees with sequence header {} activated for that \
                         layer: {fragment} (§ 6.8.8)",
                        xlayer.get(),
                        seq_header_id.get(),
                    ),
                )
                .with_spec_section("6.8.8")
                .with_byte_offset(lcr_offset),
            );
        }
    }

    /// Emits `lcr/max-expected-dims-exceed-sequence-max` (AV2 § 6.8.9) when an activated
    /// LCR's `lcr_max_expected_width[..][j]` / `lcr_max_expected_height[..][j]` exceeds the
    /// activated sequence header's `max_frame_width/height_minus_1 + 1`.
    ///
    /// AV2 § 6.8.9 (mirror `06-syntax-structures-semantics.md#s-6-8-9`, :2135-2148): "It is a
    /// requirement of bitstream conformance that lcr_max_expected_width[ isGlobal ][ xId ][ j ]
    /// shall be less than or equal to max_frame_width_minus_1 + 1 obtained from the activated
    /// sequence header" (and the height analogue). This is the pure-arithmetic clause — the
    /// LCR's declared per-embedded-layer expected maximum against the activated sequence
    /// header maximum — decidable at activation from the snapshotted association alone.
    ///
    /// The companion `FrameWidth/FrameHeight <= lcr_max_expected_width/height` per-frame clause
    /// (mirror :2137-2139 / :2144-2146) is a named residual: it needs each frame's
    /// `(obu_xlayer_id, obu_mlayer_id) -> (xId, j)` mapping and FrameWidth/Height joined
    /// against the activated LCR per layer, which this phase does not thread.
    ///
    /// Suppression / activation gating mirrors [`Self::check_lcr_rep_info_agreement`]: it
    /// fires only under [`ExternalHlsMode::Disabled`] (an unmodeled external local LCR could
    /// shadow the in-band association under a Provided mode) and only on a strict
    /// frame-confirmed activation. Anchored at the defining LCR OBU. A
    /// `same_sh_max_resolution_flag == 1` layer (omitted width/height) inherits the sequence
    /// maxima and is trivially in bound, so it carries `None` and is skipped.
    pub(super) fn check_lcr_expected_dims_bounds(
        &mut self,
        xlayer: ExtendedLayerId,
        options: &ValidationOptions,
        report: &mut ValidationReport,
    ) {
        if !matches!(options.external_hls, ExternalHlsMode::Disabled) {
            return;
        }
        let Some((seq_header_id, general)) = self.frame_confirmed_activation_for(xlayer) else {
            return;
        };
        let Some(association) = self.lcr_associations.get(&(xlayer, seq_header_id)) else {
            return;
        };
        let Some(maps) = association.maps.as_ref() else {
            return;
        };
        let lcr_is_global = association.lcr_is_global;
        let lcr_id = association.lcr_id;
        let lcr_offset = maps.offset;
        let scope = if lcr_is_global { "global" } else { "local" };
        let header_max_width = general.max_frame_width.get();
        let header_max_height = general.max_frame_height.get();

        let mut violations: Vec<(u8, bool, u32, u32)> = Vec::new();
        for &(mlayer_index, width, height) in &maps.max_expected {
            if let Some(lcr_width) = width
                && lcr_width > header_max_width
            {
                violations.push((mlayer_index, true, lcr_width, header_max_width));
            }
            if let Some(lcr_height) = height
                && lcr_height > header_max_height
            {
                violations.push((mlayer_index, false, lcr_height, header_max_height));
            }
        }

        for (mlayer_index, is_width, lcr_value, header_max) in violations {
            let key = LcrExpectedDimsFindingKey {
                xlayer,
                seq_header_id,
                lcr_is_global,
                lcr_id,
                lcr_offset,
                mlayer_index,
                is_width,
                lcr_value,
                header_max,
            };
            if !self.emitted_lcr_expected_dims_findings.insert(key) {
                continue;
            }
            let fragment = if is_width {
                format!(
                    "lcr_max_expected_width[{mlayer_index}] {lcr_value} > \
                     max_frame_width_minus_1 + 1 = {header_max}"
                )
            } else {
                format!(
                    "lcr_max_expected_height[{mlayer_index}] {lcr_value} > \
                     max_frame_height_minus_1 + 1 = {header_max}"
                )
            };
            report.push(
                Diagnostic::error(
                    "lcr/max-expected-dims-exceed-sequence-max",
                    format!(
                        "activated {scope} layer configuration record {lcr_id}'s expected \
                         dimension for extended layer {} exceeds the maximum from sequence \
                         header {} activated for that layer: {fragment} (§ 6.8.9)",
                        xlayer.get(),
                        seq_header_id.get(),
                    ),
                )
                .with_spec_section("6.8.9")
                .with_byte_offset(lcr_offset),
            );
        }
    }
}
