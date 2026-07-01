// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.18.2 **inter** frame-header shared tail
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`, mirror :5183-5343).
//!
//! After the non-intra control region reaches
//! [`InterStop::ReachedSharedTail`](super::inter::InterStop) (just past
//! `disable_cdf_update`, mirror :5041), the § 5.18.2 grammar reads the **same shared
//! structure cluster** the intra path reads, plus the inter-specific arms the intra path
//! infers to no-bit defaults:
//!
//! ```text
//! tile_info( )               // § 5.18.7.2, mirror :5183
//! quantization_params( )     // § 5.18.6.1, mirror :5185
//! segmentation_params( )     // § 5.18.7.1, mirror :5189
//! setup_qm_params( )         // § 5.18.6.2, mirror :5191
//! delta_q_params( )          // § 5.18.7.8, mirror :5193
//! // per-segment lossless/QM derivation + allow_tcq + allow_parity_hiding (mirror :5209-5295)
//! deblocking_filter_params( )// § 5.18.5.2, mirror :5297 (inter: allow_df_sub_pu arm)
//! gdf_params( )              // § 5.18.7.9, mirror :5299
//! cdef_params( )             // § 5.18.7.10, mirror :5301
//! lr_params( )               // § 5.18.7.11, mirror :5303
//! ccso_params( )             // § 5.18.7.12, mirror :5305
//! read_tx_mode( )            // § 5.18.8.1, mirror :5307
//! frame_reference_mode( )    // § 5.18.8.3, mirror :5309 (inter: reference_select f(1))
//! skip_mode_params( )        // § 5.18.8.2, mirror :5311 (inter: skip_mode_present f(1))
//! if (!FrameIsIntra && enable_bawp) allow_bawp          f(1)   // mirror :5313
//! if (!FrameIsIntra && frame_enabled_motion_modes[DELTAWARP])
//!     allow_warpmv_mode                                 f(1)   // mirror :5327
//! reduced_tx_set                                        f(2)   // mirror :5337
//! global_motion_params( )    // § 5.18.9.1, mirror :5339 (inter: use_global_motion arm)
//! film_grain_config( )       // § 5.18.10.1, mirror :5341
//! ```
//!
//! This module models the **minimal-tool single-reference inter subset** the verified
//! fixtures exercise (`tests/conformance/vectors/valid/syn-2frame-inter-64x64.ivf`,
//! `syn-key-inter-64x64.ivf`): a single 64x64 zero-MV skip block with broad decode tools
//! off, `TipFrameMode == TIP_FRAME_DISABLED`, `!IsBridge`, `!bru_inactive`, and
//! `NumTotalRefs == 1`. The shared structure cluster is reused verbatim from the intra
//! path's sub-parsers (every § 5.18.6 / § 5.18.7 / § 5.18.5 structure is
//! `FrameIsIntra`-independent except for the gates below), with the inter inputs threaded:
//!
//! - `tile_info()` is parsed with `frame_is_intra == false` (the inter `SbSize`).
//! - `quantization_params()` with `tip_frame_as_output == false`.
//! - `deblocking_filter_params()` reads the inter `allow_df_sub_pu` arm
//!   (`enable_df_sub_pu && FrameType == INTER_FRAME`, § 5.18.5.2 mirror :5935).
//! - `frame_reference_mode()` reads `reference_select` `f(1)` (mirror :7747).
//! - `skip_mode_params()` reads `skip_mode_present` `f(1)` (`skipModeAllowed == 1` for a
//!   non-switch inter frame, mirror :7717).
//! - `global_motion_params()`'s inter arm is parsed via
//!   [`parse_global_motion_params`](super::global_motion::parse_global_motion_params)
//!   (the honest cross-frame stops there cover `use_global_motion == 1` warp models).
//!
//! ## Honest gating
//!
//! Anything outside the modeled subset stops honestly with
//! [`FrameHeaderParseStatus::UnsupportedUntilFeature`] (a coverage stop, never a
//! truncation) rather than guessing bit positions:
//!
//! - `segmentation_enabled == 1`: the § 5.18.7.1 `segmentation_update_map` /
//!   `segmentation_temporal_update` reads depend on `DerivedPrimaryRefFrame`, which is the
//!   `choose_primary_secondary_ref_frame()` (§ 5.18.2 mirror :5451) ranking over unmodeled
//!   `RefBaseQIdx`. The shared `parse_segmentation_params` only models the
//!   `DerivedPrimaryRefFrame == PRIMARY_REF_NONE` arm, so an enabled-segmentation inter
//!   frame cannot continue soundly.
//! - `global_motion_params()` reaching a cross-frame stop (`use_global_motion == 1` with
//!   per-reference warp models): the honest [`GlobalMotionStop`] is surfaced.
//! - the per-segment QM index loop reaching a `using_qmatrix` read it cannot evaluate, or a
//!   tile layout needing unmodeled sequence state: the sub-parser's `Unimplemented` is
//!   surfaced.
//!
//! An [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) inside the modeled tail
//! is converted by the caller to the facts-preserving
//! [`FrameHeaderParseStatus::StoppedInsideInterControl`].

use crate::bitio::BitReader;
use crate::error::{Error, Result};
use crate::headers::frame::filtering::{
    GdfGeometry, parse_cdef_params, parse_deblocking_filter_params, parse_gdf_params,
};
use crate::headers::frame::global_motion::{GlobalMotionInput, parse_global_motion_params};
use crate::headers::frame::info::{
    CoreSeqView, FrameHeaderCore, FrameHeaderParseStatus, FrameReferenceStateView,
};
use crate::headers::frame::inter::InterControl;
use crate::headers::frame::quant::{
    parse_delta_q_params, parse_lossless_info, parse_quantization_params, parse_setup_qm_params,
};
use crate::headers::frame::restoration::{
    LrGeometry, LrParseOutcome, parse_ccso_params, parse_ccso_params_for_inter,
    parse_lr_params_for_inter,
};
use crate::headers::frame::tail::{TxMode, parse_film_grain_config, read_tx_mode};
use crate::headers::frame::tiling::parse_tile_info;

use super::info::FrameType;

/// The Feature ID for an honest inter shared-tail coverage stop.
const FRAME_HEADER_INFO_FEATURE: &str = "AV2-5.18.2-FRAME-HEADER-INFO";

/// `DELTAWARP` (AV2 v1.0.0 § 3): the delta-warp motion-mode index, gating
/// `allow_warpmv_mode` (`frame_enabled_motion_modes[DELTAWARP]`, § 5.18.2 mirror :5327).
const DELTAWARP: usize = 3;

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
    /// `use_global_motion` from `global_motion_params()` (§ 5.18.9.1).
    pub use_global_motion: bool,
    /// `apply_grain` from `film_grain_config()` (§ 5.18.10.1).
    pub apply_grain: bool,
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
/// stops honestly with [`FrameHeaderParseStatus::UnsupportedUntilFeature`].
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload ends
/// mid-field (the caller converts this to a facts-preserving truncation status), or another
/// typed descriptor error if a sub-parser rejects its inputs.
pub(crate) fn parse_inter_shared_tail(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    control: &InterControl,
    frame_type: FrameType,
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
    trace_tail_position(reader, "start");
    core.tile_info = match parse_tile_info(reader, &seq.tile, frame_size, false, false, false) {
        Ok(tile_info) => Some(tile_info),
        Err(Error::Unimplemented { feature }) => {
            core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
                feature_id: feature,
            };
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    trace_tail_position(reader, "after_tile_info");

    let quantization = parse_quantization_params(reader, &seq.quant, tip_frame_as_output)?;
    trace_tail_position(reader, "after_quant");

    let segmentation_enabled = reader.read_flag()?;
    if segmentation_enabled {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }
    let segmentation = crate::headers::frame::segmentation::SegmentationParams::disabled();
    trace_tail_position(reader, "after_segmentation");

    let qm = parse_setup_qm_params(reader, &seq.quant, segmentation.segmentation_enabled)?;
    trace_tail_position(reader, "after_qm");

    let delta_q = parse_delta_q_params(reader, quantization.base_q_idx)?;
    trace_tail_position(reader, "after_delta_q");

    let lossless = parse_lossless_info(
        reader,
        &seq.quant,
        &quantization,
        &qm,
        &delta_q,
        &segmentation,
        seq.seg.max_segments,
    )?;
    let coded_lossless = lossless.coded_lossless;

    let read_allow_df_sub_pu = seq.filter.enable_df_sub_pu && frame_type == FrameType::Inter;
    core.deblocking_filter_params = Some(parse_deblocking_filter_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        read_allow_df_sub_pu,
        None,
    )?);
    trace_tail_position(reader, "after_deblock");

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
        parse_gdf_params(reader, coded_lossless, &seq.filter, geometry)?
    };
    core.gdf_params = Some(gdf);
    trace_tail_position(reader, "after_gdf");

    core.cdef_params = Some(parse_cdef_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.filter,
    )?);
    trace_tail_position(reader, "after_cdef");

    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(false), seq.chroma_format_idc);
    let lr_reference_filter_counts =
        lr_reference_filter_counts(reference_state, &control.ref_frame_idx, num_total_refs);
    match parse_lr_params_for_inter(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        seq.restoration,
        lr_geometry,
        quantization.base_q_idx,
        num_total_refs,
        lr_reference_filter_counts,
    )? {
        LrParseOutcome::Parsed(lr) => {
            core.lr_params = Some(lr);
        }
        LrParseOutcome::StoppedBeforeWienerNsFilter {
            feature_id,
            partial,
        } => {
            core.lr_params_partial = Some(partial);
            core.status = FrameHeaderParseStatus::StoppedBeforeWienerNsFilter { feature_id };
            store_shared_facts(core, &segmentation, qm, delta_q, lossless, quantization);
            return Ok(());
        }
    }
    trace_tail_position(reader, "after_lr");

    core.ccso_params = Some(if frame_type == FrameType::Switch {
        parse_ccso_params(reader, coded_lossless, seq.quant.num_planes, &seq.ccso)?
    } else {
        parse_ccso_params_for_inter(
            reader,
            coded_lossless,
            seq.quant.num_planes,
            seq.ccso,
            num_total_refs,
        )?
    });
    trace_tail_position(reader, "after_ccso");

    store_shared_facts(core, &segmentation, qm, delta_q, lossless, quantization);

    parse_inter_tail_arms(reader, core, seq, control, frame_type, coded_lossless)
}

fn lr_reference_filter_counts(
    reference_state: &FrameReferenceStateView<'_>,
    ref_frame_idx: &[u32],
    num_total_refs: u32,
) -> [usize; 3] {
    let Some(slot_counts) = reference_state.lr_frame_filter_class_counts else {
        return [0; 3];
    };
    let mut counts = [0usize; 3];
    for slot in ref_frame_idx.iter().take(num_total_refs as usize) {
        let slot = *slot as usize;
        if reference_state
            .ref_valid
            .and_then(|valid| valid.get(slot).copied())
            == Some(false)
        {
            continue;
        }
        let Some(planes) = slot_counts.get(slot) else {
            continue;
        };
        counts[0] = counts[0].saturating_add(usize::from(planes[0]));
        counts[1] = counts[1].saturating_add(usize::from(planes[1]) + usize::from(planes[2]));
        counts[2] = counts[2].saturating_add(usize::from(planes[2]) + usize::from(planes[1]));
    }
    counts
}

/// Parses the § 5.18.2 inter tail after `ccso_params()` (mirror :5307-5341) and sets the
/// terminal status. On a clean parse of the modeled subset the tail is stored on
/// `core.inter_tail` and the status is [`FrameHeaderParseStatus::InterHeaderComplete`].
fn parse_inter_tail_arms(
    reader: &mut BitReader<'_>,
    core: &mut FrameHeaderCore,
    seq: &CoreSeqView,
    control: &InterControl,
    frame_type: FrameType,
    coded_lossless: bool,
) -> Result<()> {
    let tx_mode = read_tx_mode(reader, coded_lossless)?;

    let reference_select = reader.read_flag()?;

    let skip_mode_allowed = frame_type != FrameType::Switch;
    let skip_mode_present = if skip_mode_allowed {
        reader.read_flag()?
    } else {
        false
    };

    let allow_bawp = if seq.inter.enable_bawp {
        reader.read_flag()?
    } else {
        false
    };

    let delta_warp_enabled = control
        .frame_enabled_motion_modes
        .is_some_and(|modes| modes.get(DELTAWARP).copied().unwrap_or(false));
    let allow_warpmv_mode = if delta_warp_enabled {
        reader.read_flag()?
    } else {
        false
    };

    let reduced_tx_set = reader.read_bits_u8(2)?;

    let num_total_refs = control.num_total_refs.unwrap_or(0);
    let gm = parse_global_motion_params(
        reader,
        &GlobalMotionInput {
            frame_is_intra: false,
            frame_type,
            enable_global_motion: seq.inter.enable_global_motion,
            num_total_refs,
            ref_frame_idx: &control.ref_frame_idx,
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
    let film_grain = parse_film_grain_config(reader, &input)?;

    core.inter_tail = Some(InterTail {
        tx_mode,
        reference_select,
        skip_mode_present,
        allow_bawp,
        allow_warpmv_mode,
        reduced_tx_set,
        use_global_motion: gm.use_global_motion,
        apply_grain: film_grain.apply_grain,
    });
    core.status = FrameHeaderParseStatus::InterHeaderComplete;
    Ok(())
}

fn trace_tail_position(reader: &BitReader<'_>, label: &str) {
    if std::env::var_os("SPLOT_TRACE_INTER_HEADER_BITS").is_some() {
        eprintln!(
            "inter header bits {label} byte={} bit={} consumed={}",
            reader.byte_offset().get(),
            reader.bit_offset().get(),
            reader.consumed_bits()
        );
    }
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
