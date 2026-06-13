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

/// Emits the Annex A.4 static level-limit diagnostics for a parsed frame against the
/// active sequence header's level (AV2 v1.0.0 Annex A.4 static conformance block,
/// mirror lines 615-629):
///
/// - `annex-a/frame-size-exceeds-level` (error): `FrameWidth * FrameHeight >
///   MaxPicSize` (line 618), `FrameWidth > MaxHSize` (line 619), or
///   `FrameHeight > MaxVSize` (line 620).
/// - `annex-a/frame-size-below-minimum` (error): `FrameWidth < 16` (line 628) or
///   `FrameHeight < 16` (line 629).
/// - `annex-a/tile-count-exceeds-level` (error): `NumTiles > MaxTiles` (line 621) or
///   `TileCols > MaxTileCols` (line 622). `NumTiles = TileCols * TileRows` and
///   `TileCols` come from the parsed `tile_info()`.
///
/// All of these are inside the "When the mapped level ID, LevelIdx is contained in the
/// tables above" block (mirror lines 615-616), so they apply only when `seq_level_idx`
/// maps to a defined level (`0..=21`). [`level_limits`] returns `None` for the
/// Maximum-parameters level 31 ("there are no level-based constraints", mirror lines
/// 659-660) and for the reserved indices `22..=30`, which disables every check here —
/// the minimum-dimension `>= 16` rule included (it lives in the same gated block).
pub(super) fn frame_annex_a_level_checks(
    core: &FrameHeaderCore,
    active_sequence: &SequenceHeader,
    obu: &ObuEnvelope<'_>,
    report: &mut ValidationReport,
) {
    let level_idx = active_sequence.general.seq_level_idx.get();
    // Annex A.4: level 31 and reserved levels are not in Tables A.8/A.9, so no
    // level-limit constraint binds. A bounds-checked lookup (no indexing panic).
    let Some(limits) = level_limits(level_idx) else {
        return;
    };

    if let Some(frame_size) = core.frame_size {
        let width = frame_size.width;
        let height = frame_size.height;
        let pic_size = u64::from(width) * u64::from(height);

        // Annex A.4 line 618: FrameWidth * FrameHeight <= MaxPicSize.
        if pic_size > limits.max_pic_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameWidth * FrameHeight ({width} * {height} = {pic_size}) exceeds \
                         MaxPicSize {} for seq_level_idx {level_idx}",
                        limits.max_pic_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 619: FrameWidth <= MaxHSize.
        if width > limits.max_h_v_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameWidth {width} exceeds MaxHSize {} for seq_level_idx {level_idx}",
                        limits.max_h_v_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 620: FrameHeight <= MaxVSize (same shared column value).
        if height > limits.max_h_v_size {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-exceeds-level",
                    format!(
                        "FrameHeight {height} exceeds MaxVSize {} for seq_level_idx {level_idx}",
                        limits.max_h_v_size
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 lines 628-629: FrameWidth >= 16 and FrameHeight >= 16.
        if width < MIN_FRAME_DIMENSION || height < MIN_FRAME_DIMENSION {
            report.push(
                Diagnostic::error(
                    "annex-a/frame-size-below-minimum",
                    format!(
                        "FrameWidth {width} / FrameHeight {height} must both be at least \
                         {MIN_FRAME_DIMENSION} for seq_level_idx {level_idx}"
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
    }

    if let Some(tile_info) = core.tile_info.as_ref() {
        let tile_cols = tile_info.tile_cols;
        let num_tiles = u64::from(tile_cols) * u64::from(tile_info.tile_rows);
        // Annex A.4 line 621: NumTiles <= MaxTiles.
        if num_tiles > u64::from(limits.max_tiles) {
            report.push(
                Diagnostic::error(
                    "annex-a/tile-count-exceeds-level",
                    format!(
                        "NumTiles {num_tiles} (TileCols {tile_cols} * TileRows {}) exceeds \
                         MaxTiles {} for seq_level_idx {level_idx}",
                        tile_info.tile_rows, limits.max_tiles
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }
        // Annex A.4 line 622: TileCols <= MaxTileCols.
        if tile_cols > limits.max_tile_cols {
            report.push(
                Diagnostic::error(
                    "annex-a/tile-count-exceeds-level",
                    format!(
                        "TileCols {tile_cols} exceeds MaxTileCols {} for seq_level_idx \
                         {level_idx}",
                        limits.max_tile_cols
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(obu.offset),
            );
        }

        // TODO(spec: AV2-A-LEVELS-TIERS): the per-tile constraints in the same Annex A.4
        // gated block (mirror lines 623-627) are not checked yet: TileWidth <=
        // Tile_Width_Scaling_Factor[seq_tier][LevelIdx] * MAX_TILE_WIDTH / 4, TileWidth
        // >= 64 for non-rightmost tiles, and TileWidth * TileHeight <=
        // Tile_Area_Scaling_Factor[seq_tier][LevelIdx] * 4096 * 2304 / 4. They need the
        // per-tile layout geometry and the tier-dependent scaling-factor tables
        // (currently private to splot-core's tile parser, which already bounds tile
        // sizing at parse via the same tables).
    }
}

/// Emits the OPS-carried Annex A profile/tier/level value-space diagnostics for each
/// included extended layer's `ops_seq_profile_tier_level_info()` (§ 5.11.2):
///
/// - `annex-a/profile-reserved` (error, Annex A.2 Table A.1, mirror line 85) when
///   `ops_seq_profile_idc` is in the reserved range `5..=30`; it conforms to no defined
///   profile, so it is as non-conformant as a reserved `seq_profile_idc`. Annex A maps
///   the OPS-derived profile id onto Table A.1 per sub-bitstream (§ 6.10.4, mirror lines
///   443-451), and the OPS PTL carries `ops_seq_profile_idc` per included extended layer
///   (§ 5.11.2).
/// - `annex-a/level-reserved` (error, Annex A.4 Table A.7, mirror line 321) when
///   `ops_level_idx` is in the reserved range `22..=30`; it maps to no defined level,
///   so it is as non-conformant as a reserved `seq_level_idx`.
/// - `annex-a/high-tier-below-4-0` (warning, Annex A.4 Table A.9 NOTE, mirror lines
///   436-437) when `ops_tier_flag == 1` (High) with `ops_level_idx < 4` (level 4.0).
///   Warning, not error: the only spec statement is the informative Table A.9 NOTE
///   ("seq_tier equal to 1 can only be signaled for level 4.0 and above") plus the
///   undefined HighMbps/HighCR cells below 4.0, so error severity would overclaim a
///   non-normative source. This is the *reachable* high-tier-below-4.0 arm: a
///   sub-bitstream's `seq_tier`/`seq_level_idx` "may be derived from the corresponding
///   ops_tier_flag and ops_level_idx values signaled in the operating_point_set_obu()"
///   (mirror lines 443-451), and the OPS PTL syntax signals both `ops_level_idx` and
///   `ops_tier_flag` unconditionally (§ 5.11.2) — unlike the sequence-header arm, where
///   `seq_tier` is only signaled for `seq_level_idx > 3` and the warning is
///   syntax-unreachable.
///
/// Annex A applies its level/tier constraints per sub-bitstream using the OPS-derived
/// `ops_tier_flag` / `ops_level_idx` values (mirror lines 443-451). Both values live in
/// each included extended layer's `ops_seq_profile_tier_level_info()`
/// ([`OpsSeqProfileTierLevelInfo::level_idx`] / [`OpsSeqProfileTierLevelInfo::tier_flag`],
/// § 5.11.2). The aggregate `ops_aggregate_level_idx` (§ 5.11.1) is a separate value
/// space tracked by `AV2-5.11.2-OPS-SEQ-PTL-INFO` and is not flagged here. Anchored at
/// the OPS OBU.
pub(super) fn check_ops_level_tier_value_space(
    obu: &ObuEnvelope<'_>,
    ops: &OperatingPointSet,
    report: &mut ValidationReport,
) {
    // TODO(spec: AV2-A-LEVELS-TIERS): this checks only the OPS-carried *value space*
    // (reserved ops_level_idx / high-tier-below-4.0). § 6.10.4
    // (docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-4) additionally
    // requires the operating point's bitstream to satisfy the Annex A.4 level limits
    // (frame size, tile geometry) with seq_level_idx set to ops_level_idx — i.e. the
    // static level-limit checks now run only against the activated seq_level_idx must
    // *also* run against each OPS-advertised ops_level_idx. That needs an
    // operating-point-to-frame mapping (which frames belong to which operating point)
    // the validator does not model yet, so the planned `annex-a/frame-exceeds-ops-level`
    // diagnostic is backlogged (see the Planned diagnostics backlog in
    // docs/VALIDATOR-ROADMAP.md, blocked on operating-point frame mapping).
    for payload in &ops.payloads {
        for entry in &payload.xlayer_entries {
            let Some(ptl) = entry.ptl_info.as_ref() else {
                continue;
            };
            // Annex A.2 Table A.1: a reserved ops_seq_profile_idc (5-30) conforms to no
            // defined profile. Annex A applies its profile constraints per sub-bitstream
            // using the OPS-derived profile id (§ 6.10.4, mirror lines 443-451).
            if is_reserved_profile(ptl.seq_profile_idc) {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-reserved",
                        format!(
                            "ops_seq_profile_idc {} for extended layer {} in OPS {} operating \
                             point {} is reserved (5..=30); it conforms to no AV2 profile defined \
                             in this version of the specification",
                            ptl.seq_profile_idc,
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
            // Annex A.4 Table A.9 NOTE (informative): a High tier (ops_tier_flag == 1)
            // can only be signaled for level 4.0 (LevelIdx 4) and above. Unlike the
            // sequence header (where seq_tier is gated on seq_level_idx > 3), the OPS PTL
            // syntax carries ops_tier_flag unconditionally, so this is a reachable case.
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
    /// Emits the Annex A.2 / Annex A.4 profile and level/tier *value-space* diagnostics
    /// for the sequence header activated for `xlayer`, once per activated header per
    /// coded video sequence:
    ///
    /// - `annex-a/profile-reserved` (error, Annex A.2 Table A.1, mirror line 85):
    ///   `seq_profile_idc` in the reserved range `5..=30`.
    /// - `annex-a/profile-chroma-format-mismatch` (error, Annex A.2 Table A.1, mirror
    ///   lines 61-90): `chroma_format_idc` outside the activated profile's allowed set.
    ///   Skipped for the Configurable profile (31), whose Table A.1 chroma column is a
    ///   dash, and for a reserved profile (the reserved-profile error already fires and
    ///   no allowed set is defined).
    /// - `annex-a/profile-bit-depth-mismatch` (error, Annex A.2 Table A.1, mirror lines
    ///   61-90): `bit_depth_idc` not `0` or `1` for profiles `0..=4`. Skipped for the
    ///   Configurable profile. The parsed [`BitDepthIdc`] only models `0`/`1`, so a
    ///   sequence header that reaches activation always has an in-range bit depth; this
    ///   check is defensive and currently never fires (documented below).
    /// - `annex-a/level-reserved` (error, Annex A.4 Table A.7, mirror line 321):
    ///   `seq_level_idx` in the reserved range `22..=30`. The Maximum-parameters value
    ///   31 is valid and not flagged.
    /// - `annex-a/high-tier-below-4-0` (warning, Annex A.4 Table A.9 NOTE, mirror lines
    ///   436-437): `seq_tier == High` with `seq_level_idx < 4` (level 4.0). Warning, not
    ///   error: the only spec statement is the informative Table A.9 NOTE ("seq_tier
    ///   equal to 1 can only be signaled for level 4.0 and above") plus the undefined
    ///   HighMbps/HighCR cells, so error severity would overclaim a non-normative source.
    ///   This sequence-header arm is syntax-*unreachable*: the § 5.4.1 parser only reads
    ///   `seq_tier` when `seq_level_idx > 3`, so a parseable header below level 4.0
    ///   always infers `Tier::Main`. The *reachable* arm is the OPS path — a
    ///   sub-bitstream's `seq_tier`/`seq_level_idx` may be derived from the OPS-signaled
    ///   `ops_tier_flag`/`ops_level_idx` (mirror lines 443-451), and the OPS PTL syntax
    ///   carries `ops_tier_flag` unconditionally (§ 5.11.2) — so it is also checked, in
    ///   [`check_ops_level_tier_value_space`].
    ///
    /// Anchored at the defining sequence-header OBU ([`Self::sequence_header_offsets`]),
    /// not the activating frame OBU.
    ///
    /// Emitted only for a *frame-confirmed* in-band activation (`frame_confirmed_xlayers`,
    /// the § 5.18.2 `load_sequence_header` path): a staged-but-unactivated header that no
    /// frame has loaded does not fire (§ 7.3.6 permits staging several headers, so the
    /// OBU-order fallback — even when momentarily the sole candidate — is a guess a later
    /// frame can contradict). Unlike the agreement checks, this runs even when the caller
    /// declares external HLS, because the active header recorded for `xlayer` is always
    /// the in-band one and its value-space facts are locally decidable regardless of any
    /// external sequence header (see [`Self::on_sequence_activation`]).
    pub(super) fn check_annex_a_value_space(
        &mut self,
        xlayer: ExtendedLayerId,
        report: &mut ValidationReport,
    ) {
        // Emit only for a *frame-confirmed* activation — one a parsed frame-header
        // reference loaded (§ 5.18.2 load_sequence_header). The OBU-order first-seen
        // fallback is a guess (§ 7.3.6 permits staging headers before any frame
        // activates one): even while a staged header is momentarily the sole in-band
        // candidate it can be superseded by a later staged header that a frame then
        // references instead, and a value-space error already emitted against the guess
        // could not be retracted. So, unlike the § 6.10.7 / § 6.8.9 agreement checks
        // (whose `agreement_activation_for` also admits the sole-header shortcut because
        // they emit nothing without an OPS/LCR present), the Annex A value-space check —
        // which fires unconditionally on a reserved/mismatched field — defers entirely to
        // frame-driven activation and re-enters here the moment the frame confirms it.
        if !self.frame_confirmed_xlayers.contains(&xlayer) {
            return;
        }
        let Some((seq_header_id, general)) = self.active_general_for(xlayer) else {
            return;
        };
        let offset = self
            .sequence_header_offsets
            .get(&seq_header_id)
            .copied()
            .unwrap_or(ByteOffset::new(0));
        // Emit once per activated header per coded video sequence: the same activation
        // can be re-confirmed by multiple frames in one coded video sequence (and a CLK
        // re-activation across a coded-video-sequence boundary legitimately re-emits). The
        // key carries a fingerprint of the checked value-space fields (§ 7.3.6 permits a
        // same-`seq_header_id` redefinition with different content): a redefinition that
        // changes any field this check inspects re-runs the checks rather than being
        // suppressed by the original activation's key.
        let epoch = self.cvs.cvs_generation_epoch(xlayer);
        let fingerprint = annex_a_value_space_fingerprint(&general);
        if !self
            .emitted_annex_a_value_space
            .insert((xlayer, seq_header_id, epoch, fingerprint))
        {
            return;
        }

        let profile_idc = general.seq_profile_idc.get();
        let level_idx = general.seq_level_idx.get();
        let is_configurable = profile_idc == crate::annex_a::CONFIGURABLE_PROFILE_IDC;

        // Annex A.2 Table A.1: reserved seq_profile_idc (5-30).
        if is_reserved_profile(profile_idc) {
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
        } else if !is_configurable {
            // Annex A.2 Table A.1: chroma_format_idc must be in the profile's allowed
            // set. Configurable (31) and reserved profiles have no defined set, so the
            // mismatch check applies only to profiles 0-4.
            if !profile_allows_chroma(profile_idc, general.chroma_format_idc) {
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
            // Annex A.2 Table A.1: bit_depth_idc must be 0 or 1 for profiles 0-4. The
            // parsed BitDepthIdc enum only represents 0 (10-bit) and 1 (8-bit) — any
            // other value is rejected at parse time as BitDepthOutOfRange before a
            // header can be activated — so this branch is defensively never reachable
            // today; it is kept to make the Table A.1 column explicit and to remain
            // correct if a future profile widens the bit-depth value space.
            let bit_depth_value = general.bit_depth_idc.get();
            if bit_depth_value > 1 {
                report.push(
                    Diagnostic::error(
                        "annex-a/profile-bit-depth-mismatch",
                        format!(
                            "bit_depth_idc {bit_depth_value} is not 0 or 1, the only values \
                             allowed for seq_profile_idc {profile_idc}"
                        ),
                    )
                    .with_spec_section("A.2")
                    .with_byte_offset(offset),
                );
            }
        }

        // Annex A.4 Table A.7: reserved seq_level_idx (22-30).
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

        // Annex A.4 Table A.9 NOTE (informative): seq_tier == 1 (High) can only be
        // signaled for level 4.0 (LevelIdx 4) and above. Warning, not error: the source
        // is a non-normative NOTE plus the undefined HighMbps/HighCR cells below 4.0.
        if matches!(general.seq_tier, Tier::High) && level_idx < HIGH_TIER_MIN_LEVEL_IDX {
            report.push(
                Diagnostic::warning(
                    "annex-a/high-tier-below-4-0",
                    format!(
                        "seq_tier is High (1) with seq_level_idx {level_idx} below level 4.0 \
                         (LevelIdx 4); the Table A.9 NOTE states High tier can only be signaled \
                         for level 4.0 and above (advisory: the source is an informative NOTE)"
                    ),
                )
                .with_spec_section("A.4")
                .with_byte_offset(offset),
            );
        }
    }
}
