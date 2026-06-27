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
use crate::headers::frame::info::{CoreSeqView, FrameHeaderCore, FrameHeaderParseStatus};
use crate::headers::frame::inter::InterControl;
use crate::headers::frame::quant::{
    parse_delta_q_params, parse_lossless_info, parse_quantization_params, parse_setup_qm_params,
};
use crate::headers::frame::restoration::{
    LrGeometry, LrParseOutcome, parse_ccso_params, parse_lr_params,
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
) -> Result<()> {
    // The inter shared tail this phase models is the non-bridge, non-TIP-output,
    // non-bru-inactive ordinary inter / switch path (the only one that reaches
    // ReachedSharedTail). TipFrameMode == TIP_FRAME_DISABLED there, so tip_frame_as_output
    // is false for tile_info() / quantization_params().
    let tip_frame_as_output = false;
    let num_total_refs = control.num_total_refs.unwrap_or(0);

    // ADMISSION GATE — the verified minimal-tool subset. The shared structure cluster is
    // reused from the INTRA-arm sub-parsers; two of them have inter-specific arms the intra
    // parser does NOT model, which would mis-position every following field if they fired:
    //   - lr_params() (§ 5.18.7.11): the temporal-prediction arm (`temporal_pred_flag[plane]`,
    //     gated on `numRefFrames > 0`) is dead on the intra path but LIVE on the inter path
    //     when restoration is enabled AND NumTotalRefs > 0.
    //   - ccso_params() (§ 5.18.7.12 mirror :7491-7501): the `reuse_ccso` / `sb_reuse_ccso` /
    //     `ccso_ref_idx` reads (gated on `!(FrameIsIntra || SWITCH_FRAME)`) are dead on the
    //     intra path but LIVE on the inter path whenever a coded `ccso_planes[plane]` is set.
    // Stop honestly BEFORE reading ANY shared-tail bit when either inter arm could fire, so
    // the parser never exposes a possibly-mis-positioned `setup_qm` / `using_qmatrix` etc. to
    // downstream checks (the "tighten admission to the verified subset" discipline). The
    // verified minimal fixture has restoration and CCSO disabled, so both arms are dead and
    // the intra sub-parsers are bit-identical. (gdf_params / cdef_params / deblocking — apart
    // from its caller-gated `allow_df_sub_pu` arm — and tile_info / quant / segmentation /
    // setup_qm / delta_q / lossless are all FrameIsIntra-arm-independent, so they stay sound.)
    let lr_inter_arm_possible = seq.restoration.enable_restoration && num_total_refs > 0;
    let ccso_inter_arm_possible = seq.ccso.enable_ccso;
    if lr_inter_arm_possible || ccso_inter_arm_possible {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }

    // mirror :5183: tile_info() (§ 5.18.7.2). FrameIsIntra == false on the inter path; the
    // ordinary inter path has IsBridge == 0 and TipFrameMode == TIP_FRAME_DISABLED.
    let Some(frame_size) = core.frame_size else {
        // The reference-grounded frame size was unresolvable (a hit on an unmodeled ref
        // slot left it None); tile_info()'s MiCols/MiRows derivation needs it, so stop
        // honestly rather than guess.
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    };
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

    // mirror :5185: quantization_params() (§ 5.18.6.1).
    let quantization = parse_quantization_params(reader, &seq.quant, tip_frame_as_output)?;

    // mirror :5187: set_primary_ref_frame_and_ctx( 1 ) reads no bits.

    // mirror :5189: segmentation_params() (§ 5.18.7.1). The whole structure is gated on
    // `segmentation_enabled` (its FIRST bit, mirror :6262). On the inter path the enabled
    // block's `segmentation_update_map` / `segmentation_temporal_update` reads depend on
    // `DerivedPrimaryRefFrame` (§ 5.18.7.1 mirror :6337), which the shared
    // `parse_segmentation_params` does NOT model (it assumes the
    // `DerivedPrimaryRefFrame == PRIMARY_REF_NONE` arm), and the §5.18.2
    // `DerivedPrimaryRefFrame` itself comes from `choose_primary_secondary_ref_frame()`'s
    // unmodeled `RefBaseQIdx` ranking (mirror :5451). So read ONLY `segmentation_enabled`
    // f(1) here: when it is 0 the structure is bit-identical to the disabled result (no
    // further bits), and when it is 1 the enabled inter block is unmodeled — stop honestly
    // BEFORE reading any of its bits rather than reuse the intra parser unsoundly.
    let segmentation_enabled = reader.read_flag()?;
    if segmentation_enabled {
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }
    let segmentation = crate::headers::frame::segmentation::SegmentationParams::disabled();

    // mirror :5191: setup_qm_params() (§ 5.18.6.2), gated on the parsed segmentation_enabled.
    let qm = parse_setup_qm_params(reader, &seq.quant, segmentation.segmentation_enabled)?;

    // mirror :5193: delta_q_params() (§ 5.18.7.8), gated on base_q_idx.
    let delta_q = parse_delta_q_params(reader, quantization.base_q_idx)?;

    // mirror :5199-5295: init_coeff_cdfs() / load_previous_segment_ids() read no bits; the
    // per-segment lossless/QM derivation loop (qm_index reads) + allow_tcq +
    // allow_parity_hiding. The derivation is FrameIsIntra-independent (a pure function of
    // base_q_idx / segmentation / quant deltas / sequence tcq+parity flags).
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

    // mirror :5297: deblocking_filter_params() (§ 5.18.5.2). The inter path reads the
    // allow_df_sub_pu f(1) arm when enable_df_sub_pu && FrameType == INTER_FRAME
    // (mirror :5935); on the cur_mfh_id == 0 direct path no MFH deblocking view is supplied.
    let read_allow_df_sub_pu = seq.filter.enable_df_sub_pu && frame_type == FrameType::Inter;
    core.deblocking_filter_params = Some(parse_deblocking_filter_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        seq.filter.df_par_bits_minus_2,
        read_allow_df_sub_pu,
        None,
    )?);

    // mirror :5299: gdf_params() (§ 5.18.7.9). The inter path SbSize is
    // frame_sb_size(frame_is_intra == false). The tile_info() geometry was just parsed.
    let gdf = {
        let Some(tile_info) = core.tile_info.as_ref() else {
            // tile_info was set to Some above on every reaching path; guard rather than
            // unwrap for direct API misuse.
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

    // mirror :5301: cdef_params() (§ 5.18.7.10).
    core.cdef_params = Some(parse_cdef_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.filter,
    )?);

    // mirror :5303: lr_params() (§ 5.18.7.11). The inter temporal-prediction arm was excluded
    // by the admission gate above (restoration off OR NumTotalRefs == 0), so the shared
    // intra-arm parser is bit-identical here. A plane signalling frame_filters_on consumes
    // the fixed-coded read_wienerns_filter() bank and preserves it on the completed LR model.
    let lr_geometry = LrGeometry::new(seq.tile.frame_sb_size(false), seq.chroma_format_idc);
    match parse_lr_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.restoration,
        lr_geometry,
        quantization.base_q_idx,
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
            // Store the structure facts parsed so far before the honest stop.
            store_shared_facts(core, segmentation, qm, delta_q, lossless, quantization);
            return Ok(());
        }
    }

    // mirror :5305: ccso_params() (§ 5.18.7.12). The inter reuse arm was excluded by the
    // admission gate above (CCSO disabled), so the shared intra-arm parser returns with no
    // bits (the `!enable_ccso` early return) and the reuse is sound for the verified subset.
    core.ccso_params = Some(parse_ccso_params(
        reader,
        coded_lossless,
        seq.quant.num_planes,
        &seq.ccso,
    )?);

    // The shared structure cluster parsed; store its facts before the inter tail.
    store_shared_facts(core, segmentation, qm, delta_q, lossless, quantization);

    // mirror :5307-5341: the inter tail.
    parse_inter_tail_arms(reader, core, seq, control, frame_type, coded_lossless)
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
    // mirror :5307: read_tx_mode() (§ 5.18.8.1) — FrameIsIntra-independent.
    let tx_mode = read_tx_mode(reader, coded_lossless)?;

    // mirror :5309 / § 5.18.8.3: frame_reference_mode() reads reference_select f(1) on the
    // inter path (FrameIsIntra == false, mirror :7747).
    let reference_select = reader.read_flag()?;

    // mirror :5311 / § 5.18.8.2: skip_mode_params(). skipModeAllowed = 0 only for
    // FrameIsIntra || SWITCH_FRAME; on an ordinary inter frame skipModeAllowed = 1, so
    // skip_mode_present f(1) is read. The SkipModeFrame[1] derivation (NumTotalRefs > 1)
    // reads NO extra bits — it only sets the per-frame skip references — so the read width
    // is unaffected by the unmodeled get_relative_dist / OrderHints state.
    let skip_mode_allowed = frame_type != FrameType::Switch;
    let skip_mode_present = if skip_mode_allowed {
        reader.read_flag()?
    } else {
        false
    };

    // mirror :5313: if ( !FrameIsIntra && enable_bawp ) allow_bawp f(1).
    let allow_bawp = if seq.inter.enable_bawp {
        reader.read_flag()?
    } else {
        false
    };

    // mirror :5327: if ( !FrameIsIntra && frame_enabled_motion_modes[DELTAWARP] )
    // allow_warpmv_mode f(1). The frame_enabled_motion_modes were parsed by the inter
    // control region.
    let delta_warp_enabled = control
        .frame_enabled_motion_modes
        .is_some_and(|modes| modes.get(DELTAWARP).copied().unwrap_or(false));
    let allow_warpmv_mode = if delta_warp_enabled {
        reader.read_flag()?
    } else {
        false
    };

    // mirror :5337: reduced_tx_set f(2).
    let reduced_tx_set = reader.read_bits_u8(2)?;

    // mirror :5339 / § 5.18.9.1: global_motion_params(). The inter arm reads
    // use_global_motion f(1) and, when set, the per-reference warp models — those reach the
    // honest cross-frame GlobalMotionStop. NumTotalRefs / ref_frame_idx come from the inter
    // control region.
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
        // use_global_motion == 1 with per-reference warp models needs the unmodeled
        // cross-frame SavedGmParams / OrderHints; stop honestly with the facts preserved.
        core.status = FrameHeaderParseStatus::UnsupportedUntilFeature {
            feature_id: FRAME_HEADER_INFO_FEATURE,
        };
        return Ok(());
    }

    // mirror :5341 / § 5.18.10.1: film_grain_config(). film_grain_params_present is the
    // §5.4.1 apply_grain gate; when it is unknown (a bounded sequence-header stop) the flag
    // is undecidable, so stop honestly before the grain read.
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

/// Stores the parsed shared-structure-cluster facts on `core`. Deferred until the borrows
/// of `quantization` / `qm` / `delta_q` taken by `parse_lossless_info` are released.
fn store_shared_facts(
    core: &mut FrameHeaderCore,
    segmentation: crate::headers::frame::segmentation::SegmentationParams,
    qm: crate::headers::frame::quant::SetupQmParams,
    delta_q: crate::headers::frame::quant::DeltaQParams,
    lossless: crate::headers::frame::quant::LosslessInfo,
    quantization: crate::headers::frame::quant::QuantizationParams,
) {
    core.quantization_params = Some(quantization);
    core.segmentation_params = Some(segmentation);
    core.setup_qm_params = Some(qm);
    core.delta_q_params = Some(delta_q);
    core.lossless_info = Some(lossless);
}
