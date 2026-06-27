// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **loop-filter** writers (`ENC-BITSTREAM-WRITER`) — the inverses of the
//! § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter parsers in
//! [`crate::headers::frame`]:
//!
//! - [`write_deblocking_filter_params`] — `deblocking_filter_params()` (§ 5.18.5.2,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2`): the `apply_deblocking_filter`
//!   flags (read from the bitstream on the direct arm or copied from the resolved multi-frame
//!   header on the MFH-update arm) and the per-index `df_delta_q_present` / `df_delta_q`
//!   cascade.
//! - [`write_gdf_params`] — `gdf_params()` (§ 5.18.7.9,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-9`): `gdf_frame_enable`, the gated
//!   `gdf_per_block`, and `gdf_pic_qc_idx` / `gdf_pic_scale_idx`.
//! - [`write_cdef_params`] — `cdef_params()` (§ 5.18.7.10,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`): `cdef_frame_enable`,
//!   `cdef_damping`, `cdef_strengths`, `cdef_on_skip_txfm_frame_enable`, and the per-strength
//!   luma/chroma sets.
//!
//! Like the other frame-header config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed structure back to bits via [`BitWriter`].
//! Each writer threads the same gating inputs the parser receives (the [`CoreSeqFilterView`]
//! sequence flags, `coded_lossless`, `num_planes`, the [`MfhDeblockingView`] /
//! [`GdfGeometry`]) and validates the whole structure before any bit is written
//! (reject-before-write): every reject path leaves `writer.bit_len() == 0`.
//!
//! **Canonical encodings (semantic round-trip universal; byte-exact on the canonical
//! subset).** Two CDEF strength fields admit a redundant encoding of the same modeled value,
//! exactly like the quantization writer's `read_delta_q`-zero collapse
//! ([`crate::write::frame_quant`], § 5.18.6.3). The parser reads a `cdef_*_pri_zero` flag and,
//! only when it is `0`, an `f(4)` strength; a stored strength of `0` therefore has two coded
//! forms (`*_pri_zero == 1`, or `*_pri_zero == 0` with a coded `f(4)` zero). The writer always
//! emits the shorter `*_pri_zero == 1` form, so `parse(write(x)) == x` holds for every
//! parser-reachable model while byte-exactness is guaranteed only on the canonical subset.

use crate::headers::frame::{
    CdefParams, CoreSeqFilterView, DeblockingFilterParams, GdfGeometry, GdfParams,
    MfhDeblockingView, gdf_per_block_is_coded,
};
use crate::headers::sequence::CdefOnSkipTxfm;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `CdefDamping` lower bound (AV2 v1.0.0 § 5.18.7.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`): `cdef_damping_minus_3` is
/// `f(2)`, so `CdefDamping = cdef_damping_minus_3 + 3` is `3..=6`.
const CDEF_DAMPING_MIN: u8 = 3;
/// `CdefDamping` upper bound (see [`CDEF_DAMPING_MIN`]).
const CDEF_DAMPING_MAX: u8 = 6;
/// `CdefStrengths` lower bound (AV2 v1.0.0 § 5.18.7.10): `cdef_strengths_minus_1` is `f(3)`,
/// so `CdefStrengths = cdef_strengths_minus_1 + 1` is `1..=8`.
const CDEF_STRENGTHS_MIN: u8 = 1;
/// `CdefStrengths` upper bound (see [`CDEF_STRENGTHS_MIN`]).
const CDEF_STRENGTHS_MAX: u8 = 8;
/// `cdef_y_pri_strength` / `cdef_uv_pri_strength` are `f(4)` when coded (AV2 v1.0.0
/// § 5.18.7.10), so each fits `0..16`.
const CDEF_PRI_STRENGTH_MAX_PLUS_1: u8 = 16;
/// `gdf_pic_qc_idx` / `gdf_pic_scale_idx` are `f(2)` (AV2 v1.0.0 § 5.18.7.9), so each fits
/// `0..4`.
const GDF_IDX_MAX_PLUS_1: u8 = 4;

/// Writes `deblocking_filter_params()` (AV2 v1.0.0 § 5.18.5.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2`), the inverse of
/// [`crate::headers::frame::parse_deblocking_filter_params`].
///
/// When `coded_lossless`, the parser writes no bits and leaves the whole structure at its
/// all-default value, so the model must be all-default. Otherwise the
/// `apply_deblocking_filter` flags come from one of two arms: on the MFH-update arm
/// (`mfh.is_some_and(|v| v.mfh_deblocking_filter_update)`) the parser copies the flags from
/// the resolved multi-frame header and reads no `apply` bits, so the model's flags must equal
/// that derived array; on the direct arm the writer emits `apply[0]` / `apply[1]` `f(1)` and,
/// when `num_planes > 1 && (apply[0] || apply[1])`, the chroma pair `apply[2]` / `apply[3]`
/// `f(1)` (with the chroma pair inferred `false` when not coded). Then for `i in 0..4`: when
/// `apply[i]`, `df_delta_q_present[i]` `f(1)` and, when present, `df_delta_q[i]` recoded as
/// `f(dfParBits)` (the parser stores `DfDeltaQ[i] = raw - (1 << (dfParBits - 1))`); when
/// absent, the offset is inferred (`DfDeltaQ[0]` for `i == 1`, else `0`). When `!apply[i]`,
/// nothing is written and the offset is inferred `0`.
///
/// `df_par_bits_minus_2` is the § 5.4.10 sequence field; `mfh` is the resolved multi-frame
/// header's deblocking-update state on the `cur_mfh_id > 0` path (`None` on the direct path).
/// The model is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// - [`WriteError::BitWidthTooLarge`] if `dfParBits = df_par_bits_minus_2 + 2` exceeds `32` on
///   the non-`coded_lossless` path (the `f(dfParBits)` field width the reader could never have
///   consumed). On the `coded_lossless` path the width is never coded, so it is not checked —
///   matching the parser, which returns the all-default structure before deriving `dfParBits`.
/// - [`WriteError::NonCanonicalFrameHeader`] if a `coded_lossless` model is non-default; if a
///   stored `apply_deblocking_filter` flag disagrees with the MFH-derived array or with an
///   inferred chroma `false`; if a gated-off `df_delta_q_present` / `df_delta_q` is non-default;
///   if an absent `df_delta_q[i]` disagrees with its inferred value; or if a coded
///   `df_delta_q[i]` falls outside the `f(dfParBits)` domain.
pub fn write_deblocking_filter_params(
    writer: &mut BitWriter,
    params: &DeblockingFilterParams,
    coded_lossless: bool,
    num_planes: u8,
    df_par_bits_minus_2: u8,
    mfh: Option<&MfhDeblockingView>,
) -> WriteResult<()> {
    let plan =
        check_deblocking_encodable(params, coded_lossless, num_planes, df_par_bits_minus_2, mfh)?;
    if coded_lossless {
        // § 5.18.5.2: if ( CodedLossless ) everything is default; no bits.
        return Ok(());
    }

    // § 5.18.5.2: on the direct arm write apply[0]/apply[1] (and the chroma pair when
    // NumPlanes > 1 and either luma flag is set); on the MFH-update arm copy them with no
    // bits (validated up front against the derived array).
    if !plan.use_mfh_update {
        writer.write_flag(params.apply_deblocking_filter[0])?;
        writer.write_flag(params.apply_deblocking_filter[1])?;
        if plan.chroma_pair_coded {
            writer.write_flag(params.apply_deblocking_filter[2])?;
            writer.write_flag(params.apply_deblocking_filter[3])?;
        }
    }

    // § 5.18.5.2: for ( i = 0; i < 4; i++ ): when apply[i], df_delta_q_present[i] f(1) and,
    // when present, df_delta_q[i] f(dfParBits) coded as raw = DfDeltaQ[i] + (1 << dfParBits-1).
    for (i, &apply) in params.apply_deblocking_filter.iter().enumerate() {
        if apply {
            writer.write_flag(params.df_delta_q_present[i])?;
            if params.df_delta_q_present[i] {
                // Recovered up front; `raw` is in the f(dfParBits) domain.
                let raw = (i64::from(params.df_delta_q[i]) + plan.half) as u32;
                writer.write_bits(raw, plan.df_par_bits)?;
            }
        }
    }
    Ok(())
}

/// The re-derived § 5.18.5.2 state the deblocking writer needs before emitting any bit: which
/// `apply` arm is active, whether the chroma `apply` pair is coded, and the `dfParBits` width
/// plus its `half` offset.
struct DeblockingPlan {
    /// `mfh.is_some_and(|v| v.mfh_deblocking_filter_update)`: on the MFH-update arm the apply
    /// flags are copied with no bits read.
    use_mfh_update: bool,
    /// `num_planes > 1 && (apply[0] || apply[1])`: whether the chroma `apply` pair is coded
    /// (direct arm only).
    chroma_pair_coded: bool,
    /// `dfParBits = df_par_bits_minus_2 + 2` (the `f(dfParBits)` width).
    df_par_bits: u32,
    /// `1 << (dfParBits - 1)`, the `DfDeltaQ` offset, computed in `i64`.
    half: i64,
}

/// Validates a [`DeblockingFilterParams`] is a model the § 5.18.5.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2`) parser could have produced, and
/// re-derives the [`DeblockingPlan`], before any bit is written.
fn check_deblocking_encodable(
    params: &DeblockingFilterParams,
    coded_lossless: bool,
    num_planes: u8,
    df_par_bits_minus_2: u8,
    mfh: Option<&MfhDeblockingView>,
) -> WriteResult<DeblockingPlan> {
    // § 5.18.5.2: if ( CodedLossless ) the parser returns the all-default structure as its
    // FIRST action, before deriving dfParBits — so it never reads df_delta_q's f(dfParBits)
    // field and never raises BitWidthTooLarge on this path. Mirror that ordering: validate the
    // all-default model (a non-default model could not have been produced) but do NOT consult
    // df_par_bits here, so a coded_lossless model with an out-of-range df_par_bits_minus_2 (a
    // non-conformant § 5.4.10 field) is still writable, exactly as the parser accepts it.
    if coded_lossless {
        if params.apply_deblocking_filter != [false; 4]
            || params.df_delta_q_present != [false; 4]
            || params.df_delta_q != [0; 4]
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "deblocking_coded_lossless",
            });
        }
        // df_par_bits / half are unused on the coded_lossless path (the writer returns before
        // touching them); fill in harmless placeholders.
        return Ok(DeblockingPlan {
            use_mfh_update: false,
            chroma_pair_coded: false,
            df_par_bits: 0,
            half: 0,
        });
    }

    // § 5.18.5.2 (non-lossless): dfParBits = df_par_bits_minus_2 + 2; df_delta_q[i] is
    // f(dfParBits), which the reader rejects above 32 bits, so reject an over-wide width before
    // the shift / DfDeltaQ math (the parser raises the same BitWidthTooLarge after the apply
    // reads). dfParBits >= 2 here, so the shift below is valid; the `half` and `raw - half` /
    // `raw + half` math is done in i64 (like the parser) so wide non-conformant inputs stay
    // panic-free.
    let df_par_bits = u32::from(df_par_bits_minus_2) + 2;
    if df_par_bits > 32 {
        return Err(WriteError::BitWidthTooLarge {
            requested: df_par_bits,
            max: 32,
        });
    }
    let half = 1i64 << (df_par_bits - 1);

    // § 5.18.5.2: the apply source is the MFH copy on the update arm, else the bitstream.
    let use_mfh_update = mfh.is_some_and(|view| view.mfh_deblocking_filter_update);
    let chroma_pair_coded =
        num_planes > 1 && (params.apply_deblocking_filter[0] || params.apply_deblocking_filter[1]);
    if let Some(view) = mfh.filter(|view| view.mfh_deblocking_filter_update) {
        // The model's apply flags must equal the array the parser would copy from the MFH:
        // [0]/[1] always; [2]/[3] only when NumPlanes > 1 && (apply[0] || apply[1]), else 0.
        let mut derived = [false; 4];
        derived[0] = view.mfh_apply_deblocking_filter[0];
        derived[1] = view.mfh_apply_deblocking_filter[1];
        if num_planes > 1 && (derived[0] || derived[1]) {
            derived[2] = view.mfh_apply_deblocking_filter[2];
            derived[3] = view.mfh_apply_deblocking_filter[3];
        }
        if params.apply_deblocking_filter != derived {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "apply_deblocking_filter",
            });
        }
    } else if !chroma_pair_coded
        && (params.apply_deblocking_filter[2] || params.apply_deblocking_filter[3])
    {
        // Direct arm: when the chroma pair is not coded the parser leaves apply[2]/apply[3]
        // at their inferred 0; a stored `true` could not have been produced.
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "apply_deblocking_filter_chroma",
        });
    }

    // § 5.18.5.2: validate the per-index df_delta_q cascade.
    for (i, &apply) in params.apply_deblocking_filter.iter().enumerate() {
        if apply {
            if params.df_delta_q_present[i] {
                // df_delta_q[i] f(dfParBits): raw = DfDeltaQ[i] + half must be in 0..2^dfParBits.
                let raw = i64::from(params.df_delta_q[i]) + half;
                let max = if df_par_bits == 32 {
                    i64::from(u32::MAX)
                } else {
                    (1i64 << df_par_bits) - 1
                };
                if raw < 0 || raw > max {
                    return Err(WriteError::NonCanonicalFrameHeader { what: "df_delta_q" });
                }
            } else {
                // § 5.18.5.2: absent -> DfDeltaQ[i] = (i == 1) ? DfDeltaQ[0] : 0.
                let inferred = if i == 1 { params.df_delta_q[0] } else { 0 };
                if params.df_delta_q[i] != inferred {
                    return Err(WriteError::NonCanonicalFrameHeader { what: "df_delta_q" });
                }
            }
        } else {
            // § 5.18.5.2: when !apply[i] the parser reads nothing and infers
            // df_delta_q_present[i] = 0, DfDeltaQ[i] = 0.
            if params.df_delta_q_present[i] || params.df_delta_q[i] != 0 {
                return Err(WriteError::NonCanonicalFrameHeader { what: "df_delta_q" });
            }
        }
    }

    Ok(DeblockingPlan {
        use_mfh_update,
        chroma_pair_coded,
        df_par_bits,
        half,
    })
}

/// Writes `gdf_params()` (AV2 v1.0.0 § 5.18.7.9,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-9`), the inverse of
/// [`crate::headers::frame::parse_gdf_params`].
///
/// When `coded_lossless || !filter.enable_gdf` the parser writes no bits and leaves
/// `gdf_frame_enable == false` with every `Option` `None`, so the model must match. Otherwise
/// `gdf_frame_enable` is inferred `1` for a single picture (no bit) or written `f(1)`; when
/// it is `false` the structure returns with no further bits. When `true`, `gdf_per_block` is
/// written `f(1)` only when `gdf_per_block_is_coded` holds (else inferred `0` and the stored
/// value must be `Some(false)`), then `gdf_pic_qc_idx` and `gdf_pic_scale_idx` `f(2)`.
///
/// `geometry` is the parsed `tile_info()` geometry that drives the `gdf_per_block` gate
/// (shared with the parser via `gdf_per_block_is_coded` so the two never drift). The model
/// is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] if a disabled (`coded_lossless || !enable_gdf`)
/// model is not `gdf_frame_enable == false` with all-`None` options; if a single-picture model
/// has `gdf_frame_enable == false`; if a `gdf_frame_enable == false` model has any `Some`
/// option; if a `gdf_frame_enable == true` model has any `None` option; if `gdf_per_block` is
/// `Some(true)` while its bit is inferred `0`; or if `gdf_pic_qc_idx` / `gdf_pic_scale_idx`
/// is `>= 4` (outside its `f(2)` field).
pub fn write_gdf_params(
    writer: &mut BitWriter,
    params: &GdfParams,
    coded_lossless: bool,
    filter: &CoreSeqFilterView,
    geometry: GdfGeometry<'_>,
) -> WriteResult<()> {
    let per_block_coded = check_gdf_encodable(*params, coded_lossless, *filter, geometry)?;

    if coded_lossless || !filter.enable_gdf {
        // § 5.18.7.9: if ( CodedLossless || !enable_gdf ) gdf_frame_enable = 0; no bits.
        return Ok(());
    }

    // § 5.18.7.9: single picture infers gdf_frame_enable = 1 (no bit); else f(1).
    if !filter.single_picture_header_flag {
        writer.write_flag(params.gdf_frame_enable)?;
    }
    if !params.gdf_frame_enable {
        // § 5.18.7.9: if ( !gdf_frame_enable ) return.
        return Ok(());
    }

    // § 5.18.7.9: gdf_per_block f(1) only when coded (else inferred 0, validated up front).
    // Pattern-match the Option so a None never reaches an unwrap (validated already).
    if let (true, Some(per_block)) = (per_block_coded, params.gdf_per_block) {
        writer.write_flag(per_block)?;
    }
    // § 5.18.7.9: gdf_pic_qc_idx f(2); gdf_pic_scale_idx f(2).
    if let Some(qc_idx) = params.gdf_pic_qc_idx {
        writer.write_bits_u8(qc_idx, 2)?;
    }
    if let Some(scale_idx) = params.gdf_pic_scale_idx {
        writer.write_bits_u8(scale_idx, 2)?;
    }
    Ok(())
}

/// Validates a [`GdfParams`] is a model the § 5.18.7.9
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-9`) parser could have produced,
/// before any bit is written. Returns whether the `gdf_per_block` bit is coded.
fn check_gdf_encodable(
    params: GdfParams,
    coded_lossless: bool,
    filter: CoreSeqFilterView,
    geometry: GdfGeometry<'_>,
) -> WriteResult<bool> {
    // § 5.18.7.9: when GDF is disabled the parser leaves gdf_frame_enable = 0 and every option
    // None; a non-default model could not have been produced.
    if coded_lossless || !filter.enable_gdf {
        if params.gdf_frame_enable
            || params.gdf_per_block.is_some()
            || params.gdf_pic_qc_idx.is_some()
            || params.gdf_pic_scale_idx.is_some()
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "gdf_disabled",
            });
        }
        return Ok(false);
    }

    // § 5.18.7.9: a single picture infers gdf_frame_enable = 1.
    if filter.single_picture_header_flag && !params.gdf_frame_enable {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "gdf_frame_enable",
        });
    }

    if !params.gdf_frame_enable {
        // § 5.18.7.9: if ( !gdf_frame_enable ) return; every option stays None.
        if params.gdf_per_block.is_some()
            || params.gdf_pic_qc_idx.is_some()
            || params.gdf_pic_scale_idx.is_some()
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "gdf_frame_disabled",
            });
        }
        return Ok(false);
    }

    // § 5.18.7.9: enabled -> all three fields are present.
    let (Some(per_block), Some(qc_idx), Some(scale_idx)) = (
        params.gdf_per_block,
        params.gdf_pic_qc_idx,
        params.gdf_pic_scale_idx,
    ) else {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "gdf_frame_enabled",
        });
    };

    // § 5.18.7.9: gdf_per_block is coded only on the gdfBlkSize / tile gate (shared with the
    // parser); when inferred 0 the stored value must be Some(false).
    let per_block_coded = gdf_per_block_is_coded(filter, geometry);
    if !per_block_coded && per_block {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "gdf_per_block",
        });
    }
    // gdf_pic_qc_idx / gdf_pic_scale_idx f(2): only 0..=3 are representable.
    if qc_idx >= GDF_IDX_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "gdf_pic_qc_idx",
        });
    }
    if scale_idx >= GDF_IDX_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "gdf_pic_scale_idx",
        });
    }
    Ok(per_block_coded)
}

/// Writes `cdef_params()` (AV2 v1.0.0 § 5.18.7.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`), the inverse of
/// [`crate::headers::frame::parse_cdef_params`].
///
/// When `coded_lossless || !filter.enable_cdef` the parser writes no bits and leaves
/// `cdef_frame_enable == false` with every `Option` `None` and no strength sets, so the model
/// must match. Otherwise `cdef_frame_enable` is inferred `1` for a single picture (no bit) or
/// written `f(1)`; when `false` the structure returns with no further bits. When `true`,
/// `cdef_damping_minus_3 = CdefDamping - 3` `f(2)`, `cdef_strengths_minus_1 = CdefStrengths - 1`
/// `f(3)`, then `cdef_on_skip_txfm_frame_enable` per [`CdefOnSkipTxfm`] (`f(1)` for `Adaptive`,
/// inferred for `AlwaysOn` / `Disabled`), and `CdefStrengths` strength sets.
///
/// **Canonicalization** (see the module docs): each `*_pri_strength` of `0` is written as the
/// shorter `*_pri_zero == 1` form (no `f(4)`). Each `*_sec_strength` reverses the parser's
/// `3 -> 4` remap: the stored value is in `{0, 1, 2, 4}` and is recoded as `f(2)` of
/// `if v == 4 { 3 } else { v }`.
///
/// The model is fully validated before any bit is written (reject-before-write).
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] if a disabled model is non-default; if a
/// single-picture model has `cdef_frame_enable == false`; if a `cdef_frame_enable == false`
/// model carries any `Some` / strength set; if an enabled model has any `None` option; if
/// `cdef_damping` is outside `3..=6` or `cdef_strengths` outside `1..=8`; if `strengths.len()`
/// disagrees with `cdef_strengths`; if `cdef_on_skip_txfm_frame_enable` disagrees with an
/// inferred `AlwaysOn` / `Disabled` value; if a `*_pri_strength` is `>= 16`; if a
/// `*_sec_strength` is not in `{0, 1, 2, 4}`; or if a `num_planes <= 1` model carries a
/// non-zero `uv_*` strength.
pub fn write_cdef_params(
    writer: &mut BitWriter,
    params: &CdefParams,
    coded_lossless: bool,
    num_planes: u8,
    filter: &CoreSeqFilterView,
) -> WriteResult<()> {
    check_cdef_encodable(params, coded_lossless, num_planes, *filter)?;

    if coded_lossless || !filter.enable_cdef {
        // § 5.18.7.10: if ( CodedLossless || !enable_cdef ) cdef_frame_enable = 0; no bits.
        return Ok(());
    }

    // § 5.18.7.10: single picture infers cdef_frame_enable = 1 (no bit); else f(1).
    if !filter.single_picture_header_flag {
        writer.write_flag(params.cdef_frame_enable)?;
    }
    if !params.cdef_frame_enable {
        // § 5.18.7.10: if ( !cdef_frame_enable ) return.
        return Ok(());
    }

    // Pattern-match the options so a None never reaches an unwrap (validated up front).
    if let Some(damping) = params.cdef_damping {
        // § 5.18.7.10: cdef_damping_minus_3 f(2) = CdefDamping - 3 (CdefDamping >= 3 checked).
        writer.write_bits_u8(damping - CDEF_DAMPING_MIN, 2)?;
    }
    if let Some(strengths) = params.cdef_strengths {
        // § 5.18.7.10: cdef_strengths_minus_1 f(3) = CdefStrengths - 1 (CdefStrengths >= 1
        // checked).
        writer.write_bits_u8(strengths - CDEF_STRENGTHS_MIN, 3)?;
    }
    // § 5.18.7.10: cdef_on_skip_txfm_frame_enable per CdefOnSkipTxfm (f(1) only on Adaptive).
    if let (CdefOnSkipTxfm::Adaptive, Some(on_skip)) = (
        filter.cdef_on_skip_txfm,
        params.cdef_on_skip_txfm_frame_enable,
    ) {
        writer.write_flag(on_skip)?;
    }

    // § 5.18.7.10: for ( i = 0; i < CdefStrengths; i++ ).
    for set in &params.strengths {
        write_cdef_pri_strength(writer, set.y_pri_strength)?;
        write_cdef_sec_strength(writer, set.y_sec_strength)?;
        if num_planes > 1 {
            write_cdef_pri_strength(writer, set.uv_pri_strength)?;
            write_cdef_sec_strength(writer, set.uv_sec_strength)?;
        }
    }
    Ok(())
}

/// Writes a CDEF primary-strength field (AV2 v1.0.0 § 5.18.7.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`): `cdef_*_pri_zero` `f(1)` and,
/// when the strength is non-zero, the strength `f(4)`. Canonicalizes a `0` strength to the
/// `*_pri_zero == 1` form (validated to be in the `f(4)` domain up front).
fn write_cdef_pri_strength(writer: &mut BitWriter, strength: u8) -> WriteResult<()> {
    if strength == 0 {
        // cdef_*_pri_zero f(1) = 1 -> strength inferred 0 (no f(4)); the canonical form.
        writer.write_bit(1)
    } else {
        // cdef_*_pri_zero f(1) = 0 -> strength f(4).
        writer.write_bit(0)?;
        writer.write_bits_u8(strength, 4)
    }
}

/// Writes a CDEF secondary-strength field (AV2 v1.0.0 § 5.18.7.10,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`): the stored value is in
/// `{0, 1, 2, 4}` (the parser remaps the coded `f(2)` value `3 -> 4`), so the coded value is
/// `if v == 4 { 3 } else { v }`, written `f(2)`. The domain is validated up front.
fn write_cdef_sec_strength(writer: &mut BitWriter, strength: u8) -> WriteResult<()> {
    let coded = if strength == 4 { 3 } else { strength };
    writer.write_bits_u8(coded, 2)
}

/// Validates a [`CdefParams`] is a model the § 5.18.7.10
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-10`) parser could have produced,
/// before any bit is written.
fn check_cdef_encodable(
    params: &CdefParams,
    coded_lossless: bool,
    num_planes: u8,
    filter: CoreSeqFilterView,
) -> WriteResult<()> {
    // § 5.18.7.10: when CDEF is disabled the parser leaves cdef_frame_enable = 0, every option
    // None, and no strength sets; a non-default model could not have been produced.
    if coded_lossless || !filter.enable_cdef {
        if params.cdef_frame_enable
            || params.cdef_damping.is_some()
            || params.cdef_strengths.is_some()
            || params.cdef_on_skip_txfm_frame_enable.is_some()
            || !params.strengths.is_empty()
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "cdef_disabled",
            });
        }
        return Ok(());
    }

    // § 5.18.7.10: a single picture infers cdef_frame_enable = 1.
    if filter.single_picture_header_flag && !params.cdef_frame_enable {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_frame_enable",
        });
    }

    if !params.cdef_frame_enable {
        // § 5.18.7.10: if ( !cdef_frame_enable ) return; every option None, no strengths.
        if params.cdef_damping.is_some()
            || params.cdef_strengths.is_some()
            || params.cdef_on_skip_txfm_frame_enable.is_some()
            || !params.strengths.is_empty()
        {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "cdef_frame_disabled",
            });
        }
        return Ok(());
    }

    // § 5.18.7.10: enabled -> all three scalar options are present.
    let (Some(damping), Some(strengths), Some(_on_skip)) = (
        params.cdef_damping,
        params.cdef_strengths,
        params.cdef_on_skip_txfm_frame_enable,
    ) else {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_frame_enabled",
        });
    };

    // cdef_damping_minus_3 f(2): CdefDamping in 3..=6 (guard the subtraction).
    if !(CDEF_DAMPING_MIN..=CDEF_DAMPING_MAX).contains(&damping) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_damping",
        });
    }
    // cdef_strengths_minus_1 f(3): CdefStrengths in 1..=8 (guard the subtraction).
    if !(CDEF_STRENGTHS_MIN..=CDEF_STRENGTHS_MAX).contains(&strengths) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_strengths",
        });
    }
    // The number of strength sets must equal CdefStrengths (the parser pushes exactly that
    // many).
    if params.strengths.len() != usize::from(strengths) {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_strengths_len",
        });
    }

    // cdef_on_skip_txfm_frame_enable: inferred for AlwaysOn / Disabled, coded for Adaptive.
    check_cdef_on_skip_txfm(
        params.cdef_on_skip_txfm_frame_enable,
        filter.cdef_on_skip_txfm,
    )?;

    // Validate every strength set's coded domains (luma always; chroma only when NumPlanes > 1,
    // else inferred 0).
    for set in &params.strengths {
        check_cdef_pri_strength(set.y_pri_strength)?;
        check_cdef_sec_strength(set.y_sec_strength)?;
        if num_planes > 1 {
            check_cdef_pri_strength(set.uv_pri_strength)?;
            check_cdef_sec_strength(set.uv_sec_strength)?;
        } else if set.uv_pri_strength != 0 || set.uv_sec_strength != 0 {
            // § 5.18.7.10: monochrome reads no UV fields; the parser leaves them 0.
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "cdef_uv_monochrome",
            });
        }
    }
    Ok(())
}

/// Validates a `cdef_on_skip_txfm_frame_enable` against its [`CdefOnSkipTxfm`] arm (AV2 v1.0.0
/// § 5.18.7.10): `Adaptive` reads `f(1)` (any value); `AlwaysOn` infers `true`; `Disabled`
/// infers `false`.
fn check_cdef_on_skip_txfm(value: Option<bool>, arm: CdefOnSkipTxfm) -> WriteResult<()> {
    // The option is Some here (validated by the caller); pattern-match to avoid an unwrap.
    let Some(value) = value else {
        return Ok(());
    };
    match arm {
        CdefOnSkipTxfm::Adaptive => Ok(()),
        CdefOnSkipTxfm::AlwaysOn => {
            if value {
                Ok(())
            } else {
                Err(WriteError::NonCanonicalFrameHeader {
                    what: "cdef_on_skip_txfm_frame_enable",
                })
            }
        }
        CdefOnSkipTxfm::Disabled => {
            if value {
                Err(WriteError::NonCanonicalFrameHeader {
                    what: "cdef_on_skip_txfm_frame_enable",
                })
            } else {
                Ok(())
            }
        }
    }
}

/// Validates a CDEF primary-strength value (AV2 v1.0.0 § 5.18.7.10): when coded it is `f(4)`,
/// so it must fit `0..16` (a `0` is canonicalized to the `*_pri_zero == 1` form).
fn check_cdef_pri_strength(strength: u8) -> WriteResult<()> {
    if strength >= CDEF_PRI_STRENGTH_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_pri_strength",
        });
    }
    Ok(())
}

/// Validates a CDEF secondary-strength value (AV2 v1.0.0 § 5.18.7.10): the parser remaps the
/// coded `f(2)` value `3 -> 4`, so a stored value must be in `{0, 1, 2, 4}` (`3` is impossible
/// and anything above `4` could not have been produced).
fn check_cdef_sec_strength(strength: u8) -> WriteResult<()> {
    if matches!(strength, 0 | 1 | 2 | 4) {
        Ok(())
    } else {
        Err(WriteError::NonCanonicalFrameHeader {
            what: "cdef_sec_strength",
        })
    }
}

// The unit/reject tests and the property tests live in sibling files (each kept under the
// advisory source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writers and private helpers above.
#[cfg(test)]
include!("frame_filters_tests.rs");
#[cfg(test)]
include!("frame_filters_proptests.rs");
