// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header `segmentation_params()` writer — the inverse of the § 5.18.7.1
//! `segmentation_params()` parser
//! ([`crate::headers::frame::parse_segmentation_params`];
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`) on the intra path
//! (`ENC-BITSTREAM-WRITER`).
//!
//! This writer is additive: it reads a parsed [`SegmentationParams`] together with the
//! same sequence-derived ([`CoreSeqSegView`]) and resolved multi-frame-header
//! ([`MfhSegView`]) inputs the parser consumed, and serializes the structure back to
//! bits via [`BitWriter`] in the parser's exact field order. The fresh-`seg_info()`
//! branch reuses the shared § 5.4.9 body writer
//! ([`crate::write::write_seg_info`];
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-9`).
//!
//! The parser *derives* several [`SegmentationParams`] fields without reading bits —
//! `reuse_seg_info` is inferred when `allowChange == 0`; `segmentation_update_map` is
//! inferred `1` and `segmentation_temporal_update` is inferred `0` on this intra path;
//! `SegIdPreSkip` / `LastActiveSegId` are computed from the feature table; and the
//! reuse branch copies the stored feature data verbatim. A caller can construct a
//! [`SegmentationParams`] whose derived fields disagree with the bits that would be
//! emitted, which would reparse differently, so each such value is rejected up front
//! with a typed [`WriteError`] before any bit is written (reject-before-write). See
//! [`WriteError::NonCanonicalFrameHeader`].

use crate::headers::frame::{CoreSeqSegView, MfhSegView, SegmentationParams};
use crate::segment::{MAX_SEGMENTS, SEG_LVL_MAX, SegmentFeature, SegmentInfo};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};
use crate::write::segment::{check_seg_info_encodable, write_seg_info};

/// `SEG_LVL_SKIP`: index of the skip segment feature (AV2 v1.0.0 § 3,
/// `docs/spec/av2/1.0.0/03-symbols.md`), the `SegIdPreSkip` threshold in § 5.18.7.1
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`). Mirrors the private
/// constant in [`crate::headers::frame`]'s `segmentation` module.
const SEG_LVL_SKIP: usize = 1;

/// The all-disabled feature table, the parser's value when `segmentation_enabled == 0`
/// and the reuse default when the reuse source is absent (AV2 v1.0.0 § 5.18.7.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`).
const ALL_DISABLED: [[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS] =
    [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS];

/// Re-derives the § 5.18.7.1 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`)
/// `(haveSegParams, allowChange, reuseSource)` triple exactly as
/// [`crate::headers::frame::parse_segmentation_params`], so the writer signals or infers
/// `reuse_seg_info` from the same inputs the parser used.
///
/// `reuse_source` carries the feature data a `reuse_seg_info` copy draws from
/// (`MfhFeatureData` on the MFH branch, `SeqFeatureData` on the sequence branch); it is
/// `None` when no stored data is available.
fn derive_seg_params(
    seg: &CoreSeqSegView,
    mfh: Option<&MfhSegView>,
) -> (bool, bool, Option<SegmentInfo>) {
    // AV2 § 5.18.7.1 branch order: the MFH arm (the caller builds `mfh` only when
    // `cur_mfh_id > 0 && mfh_seg_info_present_flag[cur_mfh_id]` holds) takes priority over
    // the sequence arm, which takes priority over the zero fallback.
    if let Some(mfh) = mfh {
        let have = mfh.mfh_ext_seg_flag == seg.enable_ext_seg;
        (
            have,
            have && mfh.mfh_allow_seg_info_change,
            Some(mfh.mfh_segment_info),
        )
    } else if seg.seq_seg_info_present_flag {
        (true, seg.seq_allow_seg_info_change, seg.seq_segment_info)
    } else {
        (false, false, None)
    }
}

/// Re-derives `SegIdPreSkip` / `LastActiveSegId` over `0 <= i < min(MaxSegments,
/// MAX_SEGMENTS)` exactly as [`crate::headers::frame::parse_segmentation_params`]
/// (AV2 v1.0.0 § 5.18.7.1, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`).
///
/// The bound is clamped at [`MAX_SEGMENTS`] so a hostile `max_segments` cannot index out
/// of `features` (matching the parser, which clamps via `seg_info()`).
fn derive_seg_id_state(
    features: &[[SegmentFeature; SEG_LVL_MAX]; MAX_SEGMENTS],
    max_segments: u8,
) -> (bool, u8) {
    let count = (max_segments as usize).min(MAX_SEGMENTS);
    let mut seg_id_pre_skip = false;
    let mut last_active_seg_id = 0u8;
    for (i, segment) in features.iter().enumerate().take(count) {
        for (j, feature) in segment.iter().enumerate() {
            if feature.enabled {
                // i < MAX_SEGMENTS (16), so it fits in u8.
                last_active_seg_id = i as u8;
                if j >= SEG_LVL_SKIP {
                    seg_id_pre_skip = true;
                }
            }
        }
    }
    (seg_id_pre_skip, last_active_seg_id)
}

/// Writes `segmentation_params()` (AV2 v1.0.0 § 5.18.7.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`) on the intra path, the exact
/// inverse of [`crate::headers::frame::parse_segmentation_params`].
///
/// `seg` and `mfh` carry the same sequence-derived and resolved-multi-frame-header inputs
/// the parser consumed; pass `mfh == None` for the `cur_mfh_id == 0` direct-reference path
/// (or any path where the MFH gate is `0`). The writer emits `segmentation_enabled` `f(1)`
/// always, then — when enabled — `reuse_seg_info` `f(1)` only on the `allowChange` path,
/// followed by the fresh `seg_info(MaxSegments)` (§ 5.4.9) body when `reuse_seg_info == 0`.
/// `segmentation_update_map` (inferred `1`) and `segmentation_temporal_update` (inferred
/// `0`) and `SegIdPreSkip` / `LastActiveSegId` are derived, never coded.
///
/// The model is fully validated before any bit is written, so a rejected model leaves
/// `writer` unchanged.
///
/// # Errors
/// - [`WriteError::NonCanonicalFrameHeader`] if any derived/inferred field disagrees with
///   the parser's re-derivation over the same inputs — an inferred `reuse_seg_info` that
///   does not equal `haveSegParams`; a reuse `features` table that does not equal the
///   reuse source; a `segmentation_update_map` / `segmentation_temporal_update` that does
///   not match the inferred intra-path constants; a `SegIdPreSkip` / `LastActiveSegId`
///   that does not match the table re-derivation; or a disabled model carrying any
///   non-default field.
/// - [`WriteError::NonCanonicalSequenceValue`] propagated from
///   [`crate::write::write_seg_info`] if the fresh `features` table is not a § 5.4.9 body
///   the parser could have produced (e.g. data outside its clip window).
pub fn write_segmentation_params(
    writer: &mut BitWriter,
    params: &SegmentationParams,
    seg: &CoreSeqSegView,
    mfh: Option<&MfhSegView>,
) -> WriteResult<()> {
    check_segmentation_encodable(params, seg, mfh)?;

    // AV2 § 5.18.7.1: segmentation_enabled f(1) — always.
    writer.write_bit(u8::from(params.segmentation_enabled))?;

    if params.segmentation_enabled {
        let (_have_seg_params, allow_change, _reuse_source) = derive_seg_params(seg, mfh);

        // AV2 § 5.18.7.1: reuse_seg_info f(1) is coded only when allowChange; otherwise it
        // is inferred (= haveSegParams) and no bit is written. The up-front check has
        // already verified the inferred value matches `params.reuse_seg_info`.
        if allow_change {
            writer.write_bit(u8::from(params.reuse_seg_info))?;
        }

        if params.reuse_seg_info {
            // AV2 § 5.18.7.1 reuse branch: FeatureEnabled / FeatureData copy the stored data
            // with no bits of their own (validated against the reuse source up front).
        } else {
            // AV2 § 5.18.7.1: (FeatureEnabled, FeatureData) = seg_info(MaxSegments) (§ 5.4.9).
            // The body was pre-validated by check_segmentation_encodable, so this cannot
            // emit a partial buffer.
            let info = SegmentInfo {
                num_segments: (seg.max_segments as usize).min(MAX_SEGMENTS) as u8,
                features: params.features,
            };
            write_seg_info(writer, &info, seg.max_segments)?;
        }

        // AV2 § 5.18.7.1: segmentation_update_map (inferred 1) and
        // segmentation_temporal_update (inferred 0) on the intra path
        // (DerivedPrimaryRefFrame == PRIMARY_REF_NONE) are not coded.
    }
    // AV2 § 5.18.7.1 else-branch: FeatureEnabled / FeatureData stay all zero; no bits.

    Ok(())
}

/// Validates that `params` is a [`SegmentationParams`] the § 5.18.7.1
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-1`) parser could have produced
/// for `seg` / `mfh`, returning a typed [`WriteError`] before any bit is written. Every
/// reject path leaves the writer's `bit_len()` at `0`.
///
/// Mirrors the parser's derivation so the writer only emits bits from a model that
/// reparses identically: the inferred `reuse_seg_info`, the reuse-vs-fresh feature data,
/// the inferred intra-path map/temporal flags, and the `SegIdPreSkip` / `LastActiveSegId`
/// derivation are each re-derived and compared.
fn check_segmentation_encodable(
    params: &SegmentationParams,
    seg: &CoreSeqSegView,
    mfh: Option<&MfhSegView>,
) -> WriteResult<()> {
    if params.segmentation_enabled {
        let (have_seg_params, allow_change, reuse_source) = derive_seg_params(seg, mfh);

        if allow_change {
            // reuse_seg_info is coded as f(1); any boolean is reproducible.
        } else if params.reuse_seg_info != have_seg_params {
            // The parser infers reuse_seg_info = haveSegParams when allowChange == 0; a model
            // disagreeing with that inference would reparse with a different value.
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_reuse_seg_info",
            });
        }

        if params.reuse_seg_info {
            // AV2 § 5.18.7.1 reuse branch: the parser copies the stored feature data
            // verbatim (the all-disabled default when the source is absent). A model whose
            // features differ from the reuse source could not have been produced here.
            let source = reuse_source.map_or(ALL_DISABLED, |info| info.features);
            if params.features != source {
                return Err(WriteError::NonCanonicalFrameHeader {
                    what: "segmentation_reuse_features",
                });
            }
        } else {
            // AV2 § 5.18.7.1 fresh branch: validate the seg_info(MaxSegments) body up front
            // (§ 5.4.9) so write_seg_info cannot reject mid-write.
            let info = SegmentInfo {
                num_segments: (seg.max_segments as usize).min(MAX_SEGMENTS) as u8,
                features: params.features,
            };
            check_seg_info_encodable(&info, seg.max_segments)?;
        }

        // AV2 § 5.18.7.1 intra path: segmentation_update_map is inferred 1 and
        // segmentation_temporal_update is inferred 0; neither is coded, so a model that
        // disagrees would reparse with the inferred constants.
        if !params.segmentation_update_map {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_update_map",
            });
        }
        if params.segmentation_temporal_update {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_temporal_update",
            });
        }
    } else {
        // AV2 § 5.18.7.1 else-branch: the parser leaves every derived field at its disabled
        // default. Any non-default value could not have been produced.
        if params.reuse_seg_info {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_disabled_reuse_seg_info",
            });
        }
        if params.features != ALL_DISABLED {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_disabled_features",
            });
        }
        if params.segmentation_update_map {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_disabled_update_map",
            });
        }
        if params.segmentation_temporal_update {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "segmentation_disabled_temporal_update",
            });
        }
    }

    // AV2 § 5.18.7.1: SegIdPreSkip / LastActiveSegId are always derived from the feature
    // table (over 0 <= i < min(MaxSegments, MAX_SEGMENTS)); a model carrying different
    // derived values would reparse differently regardless of the enabled flag.
    let (seg_id_pre_skip, last_active_seg_id) =
        derive_seg_id_state(&params.features, seg.max_segments);
    if params.seg_id_pre_skip != seg_id_pre_skip {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "segmentation_seg_id_pre_skip",
        });
    }
    if params.last_active_seg_id != last_active_seg_id {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "segmentation_last_active_seg_id",
        });
    }

    Ok(())
}

// The unit/rejection tests and the property tests live in sibling files (to keep this
// module under the advisory source-line limit); `include!` pastes them into this module so
// their `super::*` resolves to the writer and its private helpers.
#[cfg(test)]
include!("frame_segmentation_tests.rs");

#[cfg(test)]
include!("frame_segmentation_proptests.rs");
