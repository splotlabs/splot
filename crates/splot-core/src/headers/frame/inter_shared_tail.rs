// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 5.18.2 inter shared tail after `disable_cdf_update`
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
//!
//! Reuses the tile, quantization, segmentation, and filtering parsers, then reads the
//! inter coding-mode and film-grain fields. Missing MFH or reference state stops at
//! the corresponding typed coverage boundary. The caller preserves earlier facts
//! when a modeled field reaches EOF.

use crate::error::{Error, Result};
use crate::headers::frame::filtering::{
    GdfGeometry, MfhDeblockingView, parse_cdef_params_with_reader,
    parse_deblocking_filter_params_with_reader, parse_gdf_params_with_reader,
};
use crate::headers::frame::global_motion::{
    GlobalMotionInput, GlobalMotionParams, GlobalMotionReferenceState,
    parse_global_motion_params_with_reader,
};
use crate::headers::frame::info::{
    CoreSeqView, FrameHeaderCore, FrameHeaderParseStatus, FrameReferenceStateView,
};
use crate::headers::frame::inter::{InterControl, PRIMARY_REF_NONE};
use crate::headers::frame::quant::{
    parse_delta_q_params_with_reader, parse_lossless_info_with_reader,
    parse_quantization_params_with_reader, parse_setup_qm_params_with_reader,
};
use crate::headers::frame::restoration::{
    LrGeometry, LrTemporalReferenceView, parse_ccso_params_for_inter_with_reader,
    parse_ccso_params_with_reader, parse_lr_params_for_inter_with_reader,
};
use crate::headers::frame::segmentation::MfhSegView;
use crate::headers::frame::tail::{
    FilmGrainConfig, TxMode, parse_film_grain_config_with_reader, read_tx_mode_with_reader,
};
use crate::headers::frame::tiling::parse_tile_info_with_reader;
use crate::syntax::{SyntaxObserver, SyntaxReader};

use super::RESTRICTED_ORDER_HINT;
use super::info::FrameType;

/// The Feature ID for an inter shared-tail coverage stop.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

/// `DELTAWARP` (AV2 v1.0.0 § 3): the delta-warp motion-mode index, gating
/// `allow_warpmv_mode` (`frame_enabled_motion_modes[DELTAWARP]`, § 5.18.2 mirror :5327).
const DELTAWARP: usize = 3;

/// MFH-derived state for an inter shared-tail parse.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterMfhState<'a> {
    pub(crate) seg: Option<&'a MfhSegView>,
    pub(crate) deblocking: Option<&'a MfhDeblockingView>,
    pub(crate) missing: bool,
}

/// The parsed § 5.18.2 inter-tail coding-mode arms after `ccso_params()` (AV2 v1.0.0
/// § 5.18.2, mirror :5307-5341). Every field is exactly determined by the reached bits and
/// the already-parsed sequence / inter-control state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InterTail {
    /// `TxMode` from `read_tx_mode()` (§ 5.18.8.1).
    pub tx_mode: TxMode,
    /// `reference_select` from `frame_reference_mode()` (§ 5.18.8.3): read `f(1)` on the
    /// inter path (mirror :7747).
    pub reference_select: bool,
    /// `skip_mode_present` from `skip_mode_params()` (§ 5.18.8.2): read `f(1)` when
    /// `skipModeAllowed == 1` (a non-switch inter frame), else inferred `0`.
    pub skip_mode_present: bool,
    /// `allow_bawp` (mirror :5313): read `f(1)` when `enable_bawp`, else `0`.
    pub allow_bawp: bool,
    /// `allow_warpmv_mode` (mirror :5327): read `f(1)` when
    /// `frame_enabled_motion_modes[DELTAWARP]`, else `0`.
    pub allow_warpmv_mode: bool,
    /// `reduced_tx_set` (`f(2)`, mirror :5337), always read.
    pub reduced_tx_set: u8,
    /// Complete per-reference `global_motion_params()` state (§ 5.18.9.1).
    pub global_motion: GlobalMotionParams,
    /// Parsed `film_grain_config()` (§ 5.18.10.1).
    pub film_grain: FilmGrainConfig,
}

/// Parses the § 5.18.2 inter shared tail (mirror :5183-5343) into `core`, after the inter
/// control region reached `InterStop::ReachedSharedTail`. The reader is positioned at the
/// shared `tile_info()` (mirror :5183).
///
/// On a clean parse of the modeled minimal subset the shared-tail facts are recorded on the
/// shared `core` fields (`tile_info`, `quantization_params`, `segmentation_params`,
/// `setup_qm_params`, `delta_q_params`, `lossless_info`, the loop-filter cluster) and the
/// inter-tail facts on `core.inter_tail`, and the status is set to
/// [`FrameHeaderParseStatus::InterHeaderComplete`]. Anything outside the modeled subset
/// stops with [`FrameHeaderParseStatus::UnsupportedUntilFeature`].
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field (the caller converts this to a facts-preserving truncation status), or another
/// typed descriptor error if a sub-parser rejects its inputs.
pub(crate) fn parse_inter_shared_tail<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    control: &InterControl,
    frame_type: FrameType,
    mfh: InterMfhState<'_>,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    let tip_frame_as_output = false;
    let num_total_refs = control.num_total_refs.unwrap_or(0);

    let Some(frame_size) = core.frame_size else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    core.tile_info =
        match parse_tile_info_with_reader(sr, &seq.tile, frame_size, false, false, false) {
            Ok(tile_info) => Some(tile_info),
            Err(Error::Unimplemented { feature }) => {
                core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                    feature_id: feature,
                };
                return Ok(());
            }
            Err(error) => return Err(error),
        };
    let quantization = parse_quantization_params_with_reader(sr, &seq.quant, tip_frame_as_output)?;

    if mfh.missing {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }

    let derived_primary_is_none = derived_primary_is_none(control, reference_state, frame_type);
    let Some(segmentation) =
        crate::headers::frame::segmentation::parse_inter_segmentation_params_with_reader(
            sr,
            &seq.seg,
            mfh.seg,
            derived_primary_is_none,
            frame_type == FrameType::Switch,
        )?
    else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    let qm = parse_setup_qm_params_with_reader(sr, &seq.quant, segmentation.segmentation_enabled)?;
    let delta_q = parse_delta_q_params_with_reader(sr, quantization.base_q_idx)?;

    let lossless = parse_lossless_info_with_reader(
        sr,
        &seq.quant,
        &quantization,
        &qm,
        delta_q,
        &segmentation,
        seq.seg.max_segments,
    )?;
    let coded_lossless = lossless.coded_lossless;

    let read_allow_df_sub_pu = seq.filter.enable_df_sub_pu && frame_type == FrameType::Inter;
    core.deblocking_filter_params = Some(parse_deblocking_filter_params_with_reader(
        sr,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        read_allow_df_sub_pu,
        mfh.deblocking,
    )?);
    let gdf = {
        let Some(tile_info) = core.tile_info.as_ref() else {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: FRAME_HEADER_INFO_FEATURE,
            };
            return Ok(());
        };
        let geometry = GdfGeometry {
            sb_size: seq.tile.frame_sb_size(false),
            mi_cols: tile_info.mi_col_starts.last().copied().unwrap_or(0),
            mi_rows: tile_info.mi_row_starts.last().copied().unwrap_or(0),
            tile_cols: tile_info.tile_cols,
            tile_rows: tile_info.tile_rows,
            mi_col_starts: &tile_info.mi_col_starts,
            mi_row_starts: &tile_info.mi_row_starts,
        };
        parse_gdf_params_with_reader(sr, coded_lossless, seq.filter, geometry)?
    };
    core.gdf_params = Some(gdf);

    core.cdef_params = Some(parse_cdef_params_with_reader(
        sr,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter,
    )?);

    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(false), seq.chroma_format_idc);
    let lr_num_total_refs = if frame_type == FrameType::Switch {
        0
    } else {
        num_total_refs
    };
    let (lr_reference_filter_counts, lr_reference_filter_taps) =
        lr_reference_filters(reference_state, &control.ref_frame_idx, lr_num_total_refs);
    let lr_params = match parse_lr_params_for_inter_with_reader(
        sr,
        coded_lossless,
        seq.quant.num_planes,
        seq.restoration,
        lr_geometry,
        lr_num_total_refs,
        lr_reference_filter_counts,
        &[
            &lr_reference_filter_taps[0],
            &lr_reference_filter_taps[1],
            &lr_reference_filter_taps[2],
        ],
        LrTemporalReferenceView::new(
            &control.ref_frame_idx,
            reference_state.lr_frame_filter_class_counts,
            reference_state.lr_frame_filter_taps,
        ),
    ) {
        Ok(params) => params,
        Err(Error::Unimplemented { feature }) => {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: feature,
            };
            store_shared_facts(core, &segmentation, qm, delta_q, lossless, quantization);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    core.lr_params = Some(lr_params);

    core.ccso_params = Some(if frame_type == FrameType::Switch {
        parse_ccso_params_with_reader(sr, coded_lossless, seq.quant.num_planes, seq.ccso)?
    } else {
        parse_ccso_params_for_inter_with_reader(
            sr,
            coded_lossless,
            seq.quant.num_planes,
            seq.ccso,
            num_total_refs,
        )?
    });

    store_shared_facts(core, &segmentation, qm, delta_q, lossless, quantization);

    parse_inter_tail_arms(
        sr,
        core,
        seq,
        control,
        frame_type,
        coded_lossless,
        reference_state,
    )
}

fn derived_primary_is_none(
    control: &InterControl,
    reference_state: &FrameReferenceStateView<'_>,
    frame_type: FrameType,
) -> Option<bool> {
    if frame_type == FrameType::Switch {
        return Some(true);
    }
    if control.signal_primary_ref_frame == Some(true) {
        return control
            .primary_ref_frame
            .map(|primary| primary == PRIMARY_REF_NONE);
    }
    let valid = reference_state.ref_valid?;
    let order_hint = reference_state.ref_order_hint?;
    let counter = reference_state.ref_counter?;
    let is_inter = reference_state.ref_frame_is_inter?;
    for &slot in &control.ref_frame_idx {
        let index = usize::try_from(slot).ok()?;
        let slot_valid = *valid.get(index)?;
        let slot_counter = *counter.get(index)?;
        let first_slot = valid
            .iter()
            .zip(counter)
            .take(index)
            .all(|(&prior_valid, &prior_counter)| !prior_valid || prior_counter != slot_counter);
        if slot_valid
            && first_slot
            && is_inter.get(index).copied()?
            && order_hint.get(index).copied()? != RESTRICTED_ORDER_HINT
        {
            return Some(false);
        }
    }
    Some(true)
}

type FilterTaps<'a> = [Vec<Option<&'a [i16]>>; 3];

/// Ordered § 5.20.10.6 reference counts and retained taps, including missing
/// classes so that a signaled match keeps its position.
pub(super) fn lr_reference_filters<'a>(
    reference_state: &FrameReferenceStateView<'a>,
    ref_frame_idx: &[u32],
    num_total_refs: u32,
) -> ([usize; 3], FilterTaps<'a>) {
    let mut counts = [0usize; 3];
    let mut entries: FilterTaps<'_> = Default::default();
    let Some(slot_counts) = reference_state.lr_frame_filter_class_counts else {
        return (counts, entries);
    };
    for &slot in ref_frame_idx.iter().take(num_total_refs as usize) {
        let slot = slot as usize;
        if reference_state
            .ref_valid
            .and_then(|valid| valid.get(slot).copied())
            == Some(false)
            || reference_state
                .ref_order_hint
                .and_then(|hints| hints.get(slot).copied())
                == Some(RESTRICTED_ORDER_HINT)
        {
            continue;
        }
        let Some(classes) = slot_counts.get(slot) else {
            continue;
        };
        counts[0] = counts[0].saturating_add(usize::from(classes[0]));
        counts[1] = counts[1].saturating_add(usize::from(classes[1]) + usize::from(classes[2]));
        let Some(planes) = reference_state
            .lr_frame_filter_taps
            .and_then(|taps| taps.get(slot))
        else {
            continue;
        };
        let remaining = super::restoration::MAX_WIENER_DICTIONARY_SLOTS - entries[0].len();
        entries[0].extend(
            (0..usize::from(classes[0]).min(remaining))
                .map(|class| planes[0].get(class).map(AsRef::as_ref)),
        );
        for (plane, checks) in [(1usize, [1usize, 2usize]), (2, [2, 1])] {
            for check in checks {
                if classes[check] > 0 {
                    entries[plane].push(planes[check].first().map(AsRef::as_ref));
                }
            }
        }
    }
    counts[2] = counts[1];
    (counts, entries)
}

/// Parses the § 5.18.2 inter tail after `ccso_params()` (mirror :5307-5341) and sets the
/// terminal status. On a clean parse of the modeled subset the tail is stored on
/// `core.inter_tail` and the status is [`FrameHeaderParseStatus::InterHeaderComplete`].
fn parse_inter_tail_arms<O: SyntaxObserver>(
    sr: &mut SyntaxReader<'_, '_, O>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    control: &InterControl,
    frame_type: FrameType,
    coded_lossless: bool,
    reference_state: &FrameReferenceStateView<'_>,
) -> Result<()> {
    let tx_mode = read_tx_mode_with_reader(sr, coded_lossless)?;

    let reference_select = sr.read_flag("reference_select")?;

    let skip_mode_allowed = frame_type != FrameType::Switch;
    let skip_mode_present = if skip_mode_allowed {
        sr.read_flag("skip_mode_present")?
    } else {
        sr.inferred_bool("skip_mode_present", "FrameType == SWITCH_FRAME", false);
        false
    };

    let allow_bawp = if seq.inter.enable_bawp {
        sr.read_flag("allow_bawp")?
    } else {
        sr.inferred_bool("allow_bawp", "!enable_bawp", false);
        false
    };

    let delta_warp_enabled = control
        .frame_enabled_motion_modes
        .is_some_and(|modes| modes.get(DELTAWARP).copied().unwrap_or(false));
    let allow_warpmv_mode = if delta_warp_enabled {
        sr.read_flag("allow_warpmv_mode")?
    } else {
        sr.inferred_bool(
            "allow_warpmv_mode",
            "!frame_enabled_motion_modes[DELTAWARP]",
            false,
        );
        false
    };

    let reduced_tx_set = sr.read_bits_u8("reduced_tx_set", 2)?;

    let num_total_refs = control.num_total_refs.unwrap_or(0);
    let gm = parse_global_motion_params_with_reader(
        sr,
        &GlobalMotionInput {
            frame_is_intra: false,
            frame_type,
            enable_global_motion: seq.inter.enable_global_motion,
            num_total_refs,
            ref_frame_idx: &control.ref_frame_idx,
            reference_state: match (
                core.order_hint,
                reference_state.ref_order_hint,
                reference_state.ref_num_total_refs,
                reference_state.saved_global_motion_order_hints,
                reference_state.saved_global_motion_params,
            ) {
                (
                    Some(order_hint),
                    Some(ref_order_hint),
                    Some(ref_num_total_refs),
                    Some(saved_order_hints),
                    Some(saved_gm_params),
                ) => Some(GlobalMotionReferenceState {
                    order_hint,
                    ref_order_hint,
                    ref_num_total_refs,
                    saved_order_hints,
                    saved_gm_params,
                }),
                _ => None,
            },
        },
    )?;
    if gm.stop.is_some() {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }

    let Some(film_grain_params_present) = seq.film_grain_params_present else {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
    let input = crate::headers::frame::tail::FrameTailInput {
        coded_lossless,
        film_grain_params_present,
        single_picture_header_flag: seq.single_picture_header_flag,
        immediate_output_frame: core.immediate_output_frame.unwrap_or(false),
        implicit_output_frame: core.implicit_output_frame.unwrap_or(false),
    };
    let film_grain = parse_film_grain_config_with_reader(sr, input)?;

    core.inter_tail = Some(InterTail {
        tx_mode,
        reference_select,
        skip_mode_present,
        allow_bawp,
        allow_warpmv_mode,
        reduced_tx_set,
        global_motion: gm,
        film_grain,
    });
    core.status = FrameHeaderParseStatus::InterHeaderComplete;
    Ok(())
}

/// Stores the parsed shared-structure-cluster facts on `core`. Deferred until the borrows
/// of `quantization` / `qm` / `delta_q` taken by `parse_lossless_info` are released.
fn store_shared_facts(
    core: &mut FrameHeaderCore,
    segmentation: &crate::headers::frame::segmentation::SegmentationParams,
    qm: crate::headers::frame::quant::SetupQmParams,
    delta_q: crate::headers::frame::quant::DeltaQParams,
    lossless: crate::headers::frame::quant::LosslessInfo,
    quantization: crate::headers::frame::quant::QuantizationParams,
) {
    core.quantization_params = Some(quantization);
    core.segmentation_params = Some(*segmentation);
    core.setup_qm_params = Some(qm);
    core.delta_q_params = Some(delta_q);
    core.lossless_info = Some(lossless);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headers::frame::restoration::SlotFrameFilterTaps;

    fn derive(
        counters: &[u32],
        is_inter: &[bool],
        order_hints: &[u32],
        refs: &[u32],
        signal: Option<bool>,
        primary: Option<u8>,
    ) -> Option<bool> {
        let valid = vec![true; counters.len()];
        let sizes = vec![64; counters.len()];
        let base_q = vec![100; counters.len()];
        let state = FrameReferenceStateView::from_slots_with_base_q_idx(
            &valid,
            order_hints,
            &sizes,
            &sizes,
            &base_q,
        )
        .with_primary_reference_state(counters, is_inter);
        let control = InterControl {
            signal_primary_ref_frame: signal,
            primary_ref_frame: primary,
            ref_frame_idx: refs.iter().copied().collect(),
            ..InterControl::default()
        };
        derived_primary_is_none(&control, &state, FrameType::Inter)
    }

    #[test]
    fn primary_availability_uses_frame_type_restriction_and_distinct_slot() {
        assert_eq!(
            derive(&[0], &[false], &[0], &[0], Some(false), None),
            Some(true)
        );
        assert_eq!(
            derive(&[0], &[true], &[u32::MAX], &[0], Some(false), None),
            Some(true)
        );
        assert_eq!(
            derive(&[0, 0], &[false, true], &[0, 1], &[1], Some(false), None),
            Some(true)
        );
        assert_eq!(
            derive(&[0, 1], &[false, true], &[0, 1], &[1], Some(false), None),
            Some(false)
        );
        assert_eq!(
            derive(&[0], &[true], &[0], &[0], Some(true), Some(7)),
            Some(true)
        );
        assert_eq!(
            derive(&[0], &[false], &[0], &[0], Some(true), Some(0)),
            Some(false)
        );
    }

    #[test]
    fn reference_filter_entries_follow_reference_and_chroma_plane_order() {
        let counts = [[2, 1, 1], [4, 1, 1], [1, 0, 1]];
        let mut taps: [SlotFrameFilterTaps; 3] = Default::default();
        for (slot, plane, sample) in [(0, 0, 1), (0, 1, 2), (0, 2, 3), (2, 2, 4)] {
            let mut class = super::super::restoration::WienerNsCoeffs::default();
            assert!(class.push(sample).is_some());
            assert!(taps[slot][plane].push(class).is_some());
        }
        let state = FrameReferenceStateView::from_slots_with_base_q_idx(
            &[true, false, true],
            &[0; 3],
            &[64; 3],
            &[64; 3],
            &[100; 3],
        )
        .with_lr_frame_filter_class_counts(&counts)
        .with_lr_frame_filter_taps(&taps);
        let (counts, entries) = lr_reference_filters(&state, &[2, 1, 0, 0], 3);
        assert_eq!(counts, [3, 3, 3]);
        let samples = entries.map(|plane| {
            plane
                .into_iter()
                .map(|taps| taps.map(|v| v[0]))
                .collect::<Vec<_>>()
        });
        assert_eq!(samples[0], [None, Some(1), None]);
        assert_eq!(samples[1], [Some(4), Some(2), Some(3)]);
        assert_eq!(samples[2], [Some(4), Some(3), Some(2)]);
    }

    /// A reference slot with a non-zero class count but no retained taps (a
    /// `frame_filters_on` plane whose bank is absent, stored as `None`) must
    /// still contribute a count-sized `None` placeholder so the entry list
    /// stays positionally aligned with the reference filter count.
    #[test]
    fn reference_filter_entries_keep_placeholder_for_none_taps() {
        let valid = [true];
        let hints = [0u32];
        let sizes = [64u32];
        let base_q = [100u32];
        let class_counts = [[1u8, 0, 0]];
        let taps: [SlotFrameFilterTaps; 1] = Default::default();
        let state = FrameReferenceStateView::from_slots_with_base_q_idx(
            &valid, &hints, &sizes, &sizes, &base_q,
        )
        .with_lr_frame_filter_class_counts(&class_counts)
        .with_lr_frame_filter_taps(&taps);
        let refs = [0u32];

        let (counts, entries) = lr_reference_filters(&state, &refs, 1);

        assert_eq!(counts[0], 1);
        assert_eq!(entries[0].len(), 1);
        assert_eq!(entries[0][0], None);
    }

    #[test]
    fn reference_filter_search_skips_restricted_references() {
        let class_counts = [[1u8, 1, 0], [2, 0, 1]];
        let taps: [SlotFrameFilterTaps; 2] = Default::default();
        let state = FrameReferenceStateView::from_slots_with_base_q_idx(
            &[true; 2],
            &[0, RESTRICTED_ORDER_HINT],
            &[64; 2],
            &[64; 2],
            &[100; 2],
        )
        .with_lr_frame_filter_class_counts(&class_counts)
        .with_lr_frame_filter_taps(&taps);

        let (counts, entries) = lr_reference_filters(&state, &[1, 0], 2);

        assert_eq!(counts, [1, 1, 1]);
        assert_eq!(entries[0].len(), 1);
    }
}
