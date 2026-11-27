// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Annex A profile, tier, level, and frame-limit value-space checks.

use super::*;

/// Smallest `seq_level_idx` at which the High tier (`seq_tier == 1`) may be signaled
/// (Annex A.4 Table A.7 maps LevelIdx 4 to level 4.0, mirror line 281; the Table A.9
/// NOTE, mirror lines 436-437, restricts High tier to "level 4.0 and above").
pub(super) const HIGH_TIER_MIN_LEVEL_IDX: u8 = 4;
/// A fingerprint of the sequence-header fields the Annex A value-space checks inspect
/// (`seq_profile_idc`, `chroma_format_idc`, `bit_depth_idc`, `seq_tier`, `seq_level_idx`).
/// Part of the [`ValidatorContext::emitted_annex_a_value_space`] dedup key so a § 7.3.6
/// same-`seq_header_id` redefinition with different checked content re-runs the checks
/// instead of being suppressed by the original activation's entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct AnnexAValueSpaceFingerprint {
    pub(super) profile_idc: u8,
    pub(super) chroma_format_idc: u8,
    pub(super) bit_depth_idc: u8,
    pub(super) tier: u8,
    pub(super) level_idx: u8,
}

/// Projects the Annex A value-space dedup fingerprint out of an activated sequence
/// header's general fields (see [`AnnexAValueSpaceFingerprint`]).
pub(super) fn annex_a_value_space_fingerprint(
    general: &SequenceHeaderGeneral,
) -> AnnexAValueSpaceFingerprint {
    AnnexAValueSpaceFingerprint {
        profile_idc: general.seq_profile_idc.get(),
        chroma_format_idc: general.chroma_format_idc.get(),
        bit_depth_idc: general.bit_depth_idc.get(),
        tier: u8::from(matches!(general.seq_tier, Tier::High)),
        level_idx: general.seq_level_idx.get(),
    }
}

/// Checks each included extended layer’s OPS profile, level and tier (Annex A.2/A.4).
/// High tier below 4.0 is advisory: the constraint comes from the informative Table A.9
/// NOTE. OPS signals the tier unconditionally, making this warning reachable.
pub(super) fn check_ops_level_tier_value_space(
    obu: &ObuEnvelope<'_>,
    ops: &OperatingPointSet,
    report: &mut ValidationReport,
) {
    // TODO(spec: AV2-A-LEVELS-TIERS): also check each ops_level_idx against the Annex A.4 limits.
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            let Some(ptl) = entry.ptl_info.as_ref() else {
                continue;
            };
            if ptl.seq_profile_idc.is_reserved() {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-reserved",
                        format!(
                            "ops_seq_profile_idc {} for extended layer {} in OPS {} operating \
                             point {} is reserved (5..=30); it conforms to no AV2 profile defined \
                             in this version of the specification",
                            ptl.seq_profile_idc.get(),
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.2")
                    .with_byte_offset(obu.offset),
                );
            }
            if is_reserved_level(ptl.level_idx) {
                report.push(
                    Diagnostic::error(
                        "annex-a/level-reserved",
                        format!(
                            "ops_level_idx {} for extended layer {} in OPS {} operating point {} \
                             is reserved (22..=30); it maps to no AV2 level defined in this \
                             version of the specification",
                            ptl.level_idx,
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.4")
                    .with_byte_offset(obu.offset),
                );
            }
            if ptl.tier_flag && ptl.level_idx < HIGH_TIER_MIN_LEVEL_IDX {
                report.push(
                    Diagnostic::warning(
                        "annex-a/high-tier-below-4-0",
                        format!(
                            "ops_tier_flag is High (1) with ops_level_idx {} below level 4.0 \
                             (LevelIdx 4) for extended layer {} in OPS {} operating point {}; the \
                             Table A.9 NOTE states High tier can only be signaled for level 4.0 \
                             and above (advisory: the source is an informative NOTE)",
                            ptl.level_idx,
                            entry.xlayer_id.get(),
                            ops.ops_id,
                            payload.index
                        ),
                    )
                    .with_spec_section("A.4")
                    .with_byte_offset(obu.offset),
                );
            }
        }
    }
}

impl ValidatorContext {
    /// Checks Annex A.2/A.4 value space once per frame-confirmed header and CVS epoch,
    /// anchored at the defining OBU. Redefinitions with changed checked fields re-run it.
    /// Snapshot construction checks external definitions. The parser infers Main tier below level 4.0.
    pub(super) fn check_annex_a_value_space(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let Some(offset) = self
            .hls
            .sequence(xlayer, seq_header_id)
            .and_then(|stored| stored.offset)
        else {
            return;
        };
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        let fingerprint = annex_a_value_space_fingerprint(&general);
        if !self
            .emitted_annex_a_value_space
            .insert((xlayer, seq_header_id, epoch, fingerprint))
        {
            return;
        }

        check_sequence_value_space(&general, offset, report);
    }
}

/// Checks sequence header facts that do not depend on stream activation.
pub(super) fn check_sequence_value_space(
    general: &SequenceHeaderGeneral,
    offset: ByteOffset,
    report: &mut ValidationReport,
) {
    let profile_idc = general.seq_profile_idc.get();
    let level_idx = general.seq_level_idx.get();

    if general.seq_profile_idc.is_reserved() {
        report.push(
            Diagnostic::error(
                "annex-a/profile-reserved",
                format!(
                    "seq_profile_idc {profile_idc} is reserved (5..=30); it conforms to no \
                     AV2 profile defined in this version of the specification"
                ),
            )
            .with_spec_section("A.2")
            .with_byte_offset(offset),
        );
    } else if !general
        .seq_profile_idc
        .allows_chroma(general.chroma_format_idc)
    {
        report.push(
            Diagnostic::error(
                "annex-a/profile-chroma-format-mismatch",
                format!(
                    "chroma_format_idc {} is not in the allowed set of seq_profile_idc {}",
                    general.chroma_format_idc.get(),
                    profile_idc
                ),
            )
            .with_spec_section("A.2")
            .with_byte_offset(offset),
        );
    }

    if is_reserved_level(level_idx) {
        report.push(
            Diagnostic::error(
                "annex-a/level-reserved",
                format!(
                    "seq_level_idx {level_idx} is reserved (22..=30); it maps to no AV2 level \
                     defined in this version of the specification"
                ),
            )
            .with_spec_section("A.4")
            .with_byte_offset(offset),
        );
    }
}
