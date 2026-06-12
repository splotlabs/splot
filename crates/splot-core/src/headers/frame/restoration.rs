// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header loop-restoration and CCSO parameters: `lr_params()`
//! (AV2 v1.0.0 § 5.18.7.11, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-11`)
//! and `ccso_params()` (§ 5.18.7.12, `#s-5-18-7-12`).
//!
//! These are the next two § 5.18.2 tail structures after the `deblocking_filter_params()`
//! / `gdf_params()` / `cdef_params()` filter cluster
//! ([`super::filtering`]) and before `read_tx_mode()` (§ 5.18.8.1). They are the two
//! largest filtering syntax bodies. On the **intra path** (`FrameIsIntra`, the only path
//! this phase models) `NumTotalRefs == 0` (§ 5.18.2, mirror :4573), so the
//! reference-frame-state machinery in both structures collapses:
//!
//! - `lr_params()`'s `temporal_pred_flag[plane]` is never read (it is gated on
//!   `numRefFrames > 0`, and `numRefFrames == 0` on the intra path), so the temporal-copy
//!   branch and `rst_ref_pic_idx` are dead, and every signalled tool reads its frame-level
//!   classes/size flags directly.
//! - `ccso_params()`'s `reuse_ccso` / `sb_reuse_ccso` / `ccso_ref_idx` reads are gated on
//!   `!(FrameIsIntra || FrameType == SWITCH_FRAME)`, so they are dead on the intra path and
//!   `load_ccso_params()` (a reference-frame-update process call) never fires.
//!
//! These structures are gated by the parsed `sequence_filter_config()` (§ 5.4.10):
//! `enable_restoration` / the `lr_tools_disable[*]` flags gate `lr_params()`, and
//! `enable_ccso` gates `ccso_params()` ([`CoreSeqRestorationView`] /
//! [`CoreSeqCcsoView`]).
//!
//! **Honest stop inside `lr_params()`.** When a plane signals
//! `frame_filters_on[plane] == 1` (the `RESTORE_WIENER_NONSEP` / `RESTORE_SWITCHABLE`
//! frame-level-filter arm), `lr_params()` calls `read_wienerns_filter(plane, 0, 0, 1)`
//! (§ 5.18.7.11, mirror :7377) at its tail. That sub-call decodes a Wiener non-separable
//! filter bank: `search_frame_filters()`, `predict_group()`, and
//! `decode_signed_subexp_with_ref()` over the `Wiener_Ns_Taps_*` tables — a large,
//! entropy-adjacent body not yet modeled. This parser therefore reads `lr_params()` in
//! full up to (but not into) that loop and reports the honest stop
//! [`LrParseOutcome::StoppedBeforeWienerNsFilter`] when any plane has
//! `frame_filters_on` set; the common all-zero case (no frame-level Wiener filter) parses
//! to completion. This mirrors the PR #57 precedent of stopping at the first structure that
//! needs unmodeled machinery rather than guessing.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};

/// Matrix Feature ID for the `read_wienerns_filter()` frame-level Wiener bank decode that
/// `lr_params()` enters when a plane signals `frame_filters_on` (§ 5.18.7.11, mirror :7377).
pub(crate) const WIENERNS_FILTER_FEATURE: &str = "AV2-5.18.7-SEGMENTATION-TILING";

/// `RESTORATION_TILESIZE_MAX` (AV2 v1.0.0 § 3, `docs/spec/av2/1.0.0/03-symbols.md`):
/// maximum size of a loop-restoration tile.
const RESTORATION_TILESIZE_MAX: u32 = 512;

/// `RESTORE_SWITCHABLE_TYPES` (AV2 § 3): `RESTORE_SWITCHABLE == 3`, the number of
/// switchable loop-restoration types scanned by the per-plane `indexToTool` loop.
const RESTORE_SWITCHABLE_TYPES: usize = 3;

/// `Decode_Num_Filter_Classes[8]` (AV2 § 5.18.7.11, mirror :7410): maps the f(3)
/// `num_filter_classes_idx` to `NumFilterClasses`.
const DECODE_NUM_FILTER_CLASSES: [u8; 8] = [1, 2, 3, 4, 6, 8, 12, 16];

/// `CCSO_INPUT_INTERVAL` (AV2 § 3): number of CCSO edge classes (mirror :7572).
const CCSO_INPUT_INTERVAL: u32 = 3;

/// `CCSO_BAND_NUM` (AV2 § 3): maximum number of bands allowed in CCSO. The § 6.17.7.8
/// conformance bound is `1 << ccso_max_band_log2 <= CCSO_BAND_NUM`.
pub const CCSO_BAND_NUM: u32 = 64;

/// `CCSO_Quant_Sz[4][4]` (AV2 § 7, mirror 07-decoding-process.md:12097): the CCSO
/// quantization step looked up by `[ccso_scale_idx][ccso_quant_idx]`; a step of `0`
/// suppresses the `ccso_edge_clf` read (§ 5.18.7.12, mirror :7552).
const CCSO_QUANT_SZ: [[u16; 4]; 4] = [
    [16, 8, 32, 0],
    [56, 40, 64, 128],
    [48, 24, 96, 192],
    [80, 112, 160, 256],
];

/// `FrameRestorationType[plane]` (AV2 § 5.18.7.11 / § 6.17.7.7, mirror semantics
/// :5680-5688): the loop-restoration tool selected for a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRestorationType {
    /// `RESTORE_NONE` (0).
    None,
    /// `RESTORE_PC_WIENER` (1).
    PcWiener,
    /// `RESTORE_WIENER_NONSEP` (2).
    WienerNonsep,
    /// `RESTORE_SWITCHABLE` (3).
    Switchable,
}

impl FrameRestorationType {
    /// Maps `indexToTool[tool_index]` (a `0..=RESTORE_SWITCHABLE` value) to the enum.
    /// `indexToTool` only ever holds `RESTORE_NONE`, the enabled tool ids, or
    /// `RESTORE_SWITCHABLE`, all in range, so an out-of-range index defensively maps to
    /// `RESTORE_NONE` rather than panicking.
    const fn from_tool(tool: u8) -> Self {
        match tool {
            1 => Self::PcWiener,
            2 => Self::WienerNonsep,
            3 => Self::Switchable,
            _ => Self::None,
        }
    }

    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "restore_none",
            Self::PcWiener => "restore_pc_wiener",
            Self::WienerNonsep => "restore_wiener_nonsep",
            Self::Switchable => "restore_switchable",
        }
    }
}

/// The § 5.4.10 sequence restoration-tool flags `lr_params()` consumes
/// (`sequence_filter_config()`), gathered from the parsed `SequenceFilterConfig`.
///
/// `lr_tools_disable[isChroma][tool]` controls which tools the per-plane `indexToTool`
/// scan enables: index `0` is luma, index `1` is chroma. `RESTORE_PC_WIENER` (1) and
/// `RESTORE_WIENER_NONSEP` (2) are the two switchable tools; `lr_uv_pc_wiener_disabled`
/// is inferred `1` whenever restoration is enabled (§ 5.4.10, mirror :1382).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSeqRestorationView {
    /// `enable_restoration` (§ 5.4.10): gates `lr_params()` past the disabled return.
    pub enable_restoration: bool,
    /// `lr_tools_disable[0][RESTORE_PC_WIENER]`.
    pub lr_pc_wiener_disabled: bool,
    /// `lr_tools_disable[0][RESTORE_WIENER_NONSEP]`.
    pub lr_wiener_nonsep_disabled: bool,
    /// `lr_tools_disable[1][RESTORE_PC_WIENER]` (inferred `1` when restoration is on).
    pub lr_uv_pc_wiener_disabled: bool,
    /// `lr_tools_disable[1][RESTORE_WIENER_NONSEP]`.
    pub lr_uv_wiener_nonsep_disabled: bool,
}

impl CoreSeqRestorationView {
    /// `lr_tools_disable[isChroma][tool]` for the per-plane `indexToTool` scan, where
    /// `tool` is `1` (`RESTORE_PC_WIENER`) or `2` (`RESTORE_WIENER_NONSEP`).
    const fn lr_tool_disabled(self, is_chroma: bool, tool: usize) -> bool {
        match (is_chroma, tool) {
            (false, 1) => self.lr_pc_wiener_disabled,
            (false, 2) => self.lr_wiener_nonsep_disabled,
            (true, 1) => self.lr_uv_pc_wiener_disabled,
            (true, 2) => self.lr_uv_wiener_nonsep_disabled,
            // No other RESTORE_* index is scanned (RESTORE_SWITCHABLE_TYPES == 3, and the
            // scan runs i in 1..3). Treat any other index as not-disabled defensively.
            _ => false,
        }
    }
}

/// The § 5.4.10 sequence CCSO flag `ccso_params()` consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSeqCcsoView {
    /// `enable_ccso` (§ 5.4.10): gates `ccso_params()` past the disabled return.
    pub enable_ccso: bool,
    /// `single_picture_header_flag` (§ 5.4.1): infers `ccso_frame_flag = 1` without a
    /// bit (§ 5.18.7.12, mirror :7474).
    pub single_picture_header_flag: bool,
}

/// Frame geometry inputs `lr_params()` consumes for the per-plane size signaling
/// (§ 5.18.7.11): the frame `SbSize` and the chroma subsampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LrGeometry {
    /// The frame `SbSize` (§ 5.18.2): selects the `BLOCK_256X256` / `BLOCK_128X128` arms
    /// of the luma/chroma size signaling.
    pub sb_size: SuperblockSize,
    /// `SubsamplingX` (§ 6.4.1): part of `Max(SubsamplingX, SubsamplingY)` for the chroma
    /// `LoopRestorationSize`.
    pub subsampling_x: u8,
    /// `SubsamplingY` (§ 6.4.1).
    pub subsampling_y: u8,
}

impl LrGeometry {
    /// Derives the geometry from the frame `SbSize` and the sequence `chroma_format_idc`
    /// (AV2 § 6.4.1, mirror :340-346).
    #[must_use]
    pub const fn new(sb_size: SuperblockSize, chroma: ChromaFormatIdc) -> Self {
        let (subsampling_x, subsampling_y) = match chroma {
            // CHROMA_FORMAT_420 / CHROMA_FORMAT_400 -> (1, 1); _444 -> (0, 0); _422 -> (1, 0).
            ChromaFormatIdc::Yuv420 | ChromaFormatIdc::Monochrome => (1, 1),
            ChromaFormatIdc::Yuv444 => (0, 0),
            ChromaFormatIdc::Yuv422 => (1, 0),
        };
        Self {
            sb_size,
            subsampling_x,
            subsampling_y,
        }
    }
}

/// One plane's parsed `lr_params()` per-plane state (AV2 § 5.18.7.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LrPlaneParams {
    /// `FrameRestorationType[plane]` (selected via `tool_index ns(n)`).
    pub restoration_type: FrameRestorationType,
    /// `frame_filters_on[plane]`: whether the plane signals a frame-level Wiener filter.
    pub frame_filters_on: bool,
    /// `NumFilterClasses` derived from `num_filter_classes_idx` (plane 0 only, when
    /// `frame_filters_on[0]` and not temporal — always non-temporal on the intra path);
    /// `None` when not signalled.
    pub num_filter_classes: Option<u8>,
}

/// Parsed `lr_params()` (AV2 v1.0.0 § 5.18.7.11) on the intra path.
///
/// The fields after `UsesLr` are present only when the structure parsed past the disabled
/// return. `LoopRestorationSize[0..3]` is the derived per-plane restoration unit size.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LrParams {
    /// `UsesLr`: any plane uses loop restoration.
    pub uses_lr: bool,
    /// Per-plane parsed state (`NumPlanes` entries; empty when restoration is disabled).
    pub planes: Vec<LrPlaneParams>,
    /// `LoopRestorationSize[0..3]` derived per plane.
    pub loop_restoration_size: [u32; 3],
}

/// The partially-parsed `lr_params()` facts committed before the honest
/// [`LrParseOutcome::StoppedBeforeWienerNsFilter`] stop (AV2 v1.0.0 § 5.18.7.11).
///
/// When a plane signals `frame_filters_on[plane]`, `lr_params()` reads every modeled
/// field — the per-plane `indexToTool` selection, `frame_filters_on`, the luma
/// `NumFilterClasses`, and the luma/chroma size-signaling flags — **before** it would
/// enter the unmodeled `read_wienerns_filter()` bank decode (mirror :7377). Those facts
/// are real and consumed; this struct carries them so consumers (inspect, the validator)
/// see the parsed prefix instead of an opaque `None`.
///
/// This is deliberately a *distinct* type from [`LrParams`]: a complete parse yields
/// `LrParams`, a stopped parse yields `LrPartialParams`. The two are never interchangeable,
/// so no consumer can mistake a partial parse for a complete one (a partial parse never
/// observed the frame-level Wiener bank that follows). The fields mirror the completed
/// `LrParams` fields up to the stop point; `loop_restoration_size` is derived because the
/// size-signaling phase completes before the Wiener loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LrPartialParams {
    /// `UsesLr`: any plane uses loop restoration (derived before the stop).
    pub uses_lr: bool,
    /// Per-plane parsed state committed before the stop (`NumPlanes` entries).
    pub planes: Vec<LrPlaneParams>,
    /// `LoopRestorationSize[0..3]` derived per plane (the size-signaling flags are read
    /// before the Wiener loop, so this is exact).
    pub loop_restoration_size: [u32; 3],
}

/// The result of attempting to parse `lr_params()` on the intra path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LrParseOutcome {
    /// `lr_params()` parsed to completion (no plane signalled a frame-level Wiener
    /// filter, so `read_wienerns_filter()` read nothing).
    Parsed(LrParams),
    /// A plane signalled `frame_filters_on[plane]`, so the structure entered
    /// `read_wienerns_filter(plane, 0, 0, 1)` (§ 5.18.7.11, mirror :7377). That
    /// frame-level Wiener bank decode is not yet modeled; the honest stop carries the
    /// blocking Feature ID and the partially-parsed LR facts committed up to the loop. No
    /// bits past the last completed read were consumed.
    StoppedBeforeWienerNsFilter {
        /// Implementation-matrix Feature ID for the `read_wienerns_filter()` coverage.
        feature_id: &'static str,
        /// The LR facts parsed before the stop (per-plane tool/frame_filters_on/classes,
        /// `UsesLr`, and the derived `LoopRestorationSize`).
        partial: LrPartialParams,
    },
}

/// Parses `lr_params()` (AV2 v1.0.0 § 5.18.7.11) on the intra path.
///
/// `coded_lossless` is the frame `CodedLossless`; `num_planes` is `NumPlanes`; `view`
/// carries the § 5.4.10 restoration-tool flags; `geometry` is the frame `SbSize` and
/// chroma subsampling; `base_q_idx` is the frame `base_q_idx` (read only to mirror the
/// spec's `get_filter_set_index` derivation, which signals no bits). On the intra path
/// `FrameIsIntra` so `numRefFrames == 0`: the temporal-prediction arm and its
/// `temporal_pred_flag` / `rst_ref_pic_idx` reads are dead.
///
/// Returns [`LrParseOutcome::StoppedBeforeWienerNsFilter`] (never an error) when a plane
/// signals a frame-level Wiener filter — the `read_wienerns_filter()` decode is unmodeled.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-field before a modeled read completes.
pub fn parse_lr_params(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqRestorationView,
    geometry: LrGeometry,
    base_q_idx: u32,
) -> Result<LrParseOutcome> {
    // AV2 § 5.18.7.11: if ( CodedLossless || !enable_restoration ) all planes RESTORE_NONE,
    // UsesLr = 0, frame_filters_on[i] = 0; return (no bits).
    let _ = base_q_idx; // `get_filter_set_index(base_q_idx)` signals no bits (SubclassLookup only).
    if coded_lossless || !view.enable_restoration {
        return Ok(LrParseOutcome::Parsed(LrParams {
            uses_lr: false,
            planes: Vec::new(),
            loop_restoration_size: default_restoration_size(geometry),
        }));
    }

    let mut uses_luma_lr = false;
    let mut uses_chroma_lr = false;
    let mut planes: Vec<LrPlaneParams> = Vec::with_capacity(usize::from(num_planes));
    // `frame_filters_on` triggering the unmodeled read_wienerns_filter loop: once any plane
    // sets it, the structure's tail `read_wienerns_filter()` is unmodeled, so we stop after
    // finishing every plain (f/ns) read in the per-plane and size-signaling phases.
    let mut any_frame_filters_on = false;

    // AV2 § 5.18.7.11: for ( plane = 0; plane < NumPlanes; plane++ ).
    for plane in 0..usize::from(num_planes) {
        let is_chroma = plane > 0;
        // indexToTool[0] = RESTORE_NONE; for i in 1..RESTORE_SWITCHABLE_TYPES add enabled
        // tools; indexToTool[toolsCount] = RESTORE_SWITCHABLE.
        let mut index_to_tool = [0u8; RESTORE_SWITCHABLE_TYPES + 1];
        let mut tools_count = 1usize; // indexToTool[0] = RESTORE_NONE.
        for i in 1..RESTORE_SWITCHABLE_TYPES {
            if !view.lr_tool_disabled(is_chroma, i) {
                index_to_tool[tools_count] = i as u8;
                tools_count += 1;
            }
        }
        // indexToTool[toolsCount] = RESTORE_SWITCHABLE (3).
        index_to_tool[tools_count] = RESTORE_SWITCHABLE_TYPES as u8;
        let allow_switchable = tools_count > 2;
        // n = toolsCount + allowSwitchable; tool_index ns(n).
        let n = tools_count as u32 + u32::from(allow_switchable);
        let tool_index = reader.read_ns(n)?;
        // FrameRestorationType[plane] = indexToTool[tool_index].
        let tool = index_to_tool.get(tool_index as usize).copied().unwrap_or(0);
        let restoration_type = FrameRestorationType::from_tool(tool);

        if restoration_type != FrameRestorationType::None {
            if plane == 0 {
                uses_luma_lr = true;
            } else {
                uses_chroma_lr = true;
            }
        }

        // frame_filters_on[plane] = 0; temporal_pred_flag[plane] = 0.
        let mut frame_filters_on = false;
        let mut num_filter_classes: Option<u8> = None;

        // r == RESTORE_WIENER_NONSEP || r == RESTORE_SWITCHABLE.
        if matches!(
            restoration_type,
            FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
        ) {
            // frame_filters_on[plane] f(1).
            frame_filters_on = reader.read_bit()? != 0;
            if frame_filters_on {
                any_frame_filters_on = true;
                // AV2 § 5.18.7.11: numRefFrames = (FrameIsIntra || FrameType == SWITCH_FRAME)
                // ? 0 : NumTotalRefs. On the intra path FrameIsIntra, so numRefFrames == 0:
                // temporal_pred_flag[plane] is NOT read (gated on numRefFrames > 0) and the
                // whole temporal-copy branch (rst_ref_pic_idx, RefFrameLrWienerNs copy) is dead.

                // if ( plane == 0 && frame_filters_on[0] ): temporal_pred_flag == 0 here, so
                // num_filter_classes_idx f(3); NumFilterClasses = Decode_Num_Filter_Classes[idx].
                if plane == 0 {
                    let idx = reader.read_bits_u8(3)?;
                    // idx is f(3) -> 0..=7, always in range of the 8-entry table.
                    let classes = DECODE_NUM_FILTER_CLASSES
                        .get(usize::from(idx))
                        .copied()
                        .unwrap_or(1);
                    num_filter_classes = Some(classes);
                }
            }
        }

        planes.push(LrPlaneParams {
            restoration_type,
            frame_filters_on,
            num_filter_classes,
        });
    }

    // AV2 § 5.18.7.11: UsesLr = usesLumaLr || usesChromaLr; the per-plane LoopRestorationSize
    // derivation. shift selection reads luma/chroma size flags.
    let uses_lr = uses_luma_lr || uses_chroma_lr;
    let max_subsampling = u32::from(geometry.subsampling_x.max(geometry.subsampling_y));
    let mut loop_restoration_size = [
        RESTORATION_TILESIZE_MAX >> 3,
        RESTORATION_TILESIZE_MAX >> (3 + max_subsampling),
        0,
    ];

    if uses_luma_lr {
        // lr_luma_use_half_size f(1); else if SbSize == BLOCK_256X256 shift = 0; else read
        // lr_luma_use_max_size f(1); ...; else lr_luma_use_quarter_size f(1).
        let shift = read_lr_size_shift(reader, geometry.sb_size)?;
        loop_restoration_size[0] = RESTORATION_TILESIZE_MAX >> shift;
    }

    if uses_chroma_lr {
        // LoopRestorationSize[1] = RESTORATION_TILESIZE_MAX >> Max(SubsamplingX, SubsamplingY).
        let base = RESTORATION_TILESIZE_MAX >> max_subsampling;
        let shift = read_lr_size_shift(reader, geometry.sb_size)?;
        loop_restoration_size[1] = base >> shift;
    }

    // LoopRestorationSize[2] = LoopRestorationSize[1].
    loop_restoration_size[2] = loop_restoration_size[1];

    // AV2 § 5.18.7.11 (mirror :7373-7381): for each plane with frame_filters_on[plane] &&
    // !temporal_pred_flag[plane], read_wienerns_filter(plane, 0, 0, 1). On the intra path
    // temporal_pred_flag is always 0, so any frame_filters_on plane enters the unmodeled
    // read_wienerns_filter() decode. Stop honestly there rather than guessing the bank decode.
    if any_frame_filters_on {
        // The per-plane and size-signaling phases above are complete, so UsesLr and the
        // derived LoopRestorationSize are exact facts; carry them with the per-plane state
        // so consumers see the parsed prefix rather than an opaque stop.
        return Ok(LrParseOutcome::StoppedBeforeWienerNsFilter {
            feature_id: WIENERNS_FILTER_FEATURE,
            partial: LrPartialParams {
                uses_lr,
                planes,
                loop_restoration_size,
            },
        });
    }

    Ok(LrParseOutcome::Parsed(LrParams {
        uses_lr,
        planes,
        loop_restoration_size,
    }))
}

/// Reads the luma/chroma restoration size `shift` (AV2 § 5.18.7.11, mirror :7287-7369).
/// The luma and chroma arms read the same three-flag structure (`*_use_half_size`,
/// `*_use_max_size`, `*_use_quarter_size`) with the same `SbSize`-dependent inferences, so
/// they share one helper.
fn read_lr_size_shift(reader: &mut BitReader<'_>, sb_size: SuperblockSize) -> Result<u32> {
    // *_use_half_size f(1).
    if reader.read_bit()? != 0 {
        // shift = 1.
        return Ok(1);
    }
    if sb_size == SuperblockSize::Block256x256 {
        // shift = 0.
        return Ok(0);
    }
    // *_use_max_size f(1).
    if reader.read_bit()? != 0 {
        // shift = 0.
        return Ok(0);
    }
    if sb_size == SuperblockSize::Block128x128 {
        // shift = 2.
        return Ok(2);
    }
    // *_use_quarter_size f(1); shift = quarter ? 2 : 3.
    if reader.read_bit()? != 0 {
        Ok(2)
    } else {
        Ok(3)
    }
}

/// `LoopRestorationSize` when restoration is disabled or before any signalling
/// (AV2 § 5.18.7.11, mirror :7281-7285): the default unit sizes.
const fn default_restoration_size(geometry: LrGeometry) -> [u32; 3] {
    let max_subsampling = if geometry.subsampling_x > geometry.subsampling_y {
        geometry.subsampling_x as u32
    } else {
        geometry.subsampling_y as u32
    };
    let chroma = RESTORATION_TILESIZE_MAX >> (3 + max_subsampling);
    [RESTORATION_TILESIZE_MAX >> 3, chroma, chroma]
}

/// One plane's parsed `ccso_params()` state (AV2 § 5.18.7.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcsoPlaneParams {
    /// `ccso_planes[plane]`: whether CCSO is enabled for the plane.
    pub ccso_planes: bool,
    /// `ccso_bo_only[plane]`: a smaller set of CCSO parameters is present.
    pub ccso_bo_only: Option<bool>,
    /// `ccso_scale_idx[plane]` (`f(2)`).
    pub ccso_scale_idx: Option<u8>,
    /// `ccso_quant_idx[plane]` (`f(2)`; `0` when `ccso_bo_only`).
    pub ccso_quant_idx: Option<u8>,
    /// `ccso_ext_filter[plane]` (`f(3)`; `0` when `ccso_bo_only`). § 6.17.7.8 forbids `7`.
    pub ccso_ext_filter: Option<u8>,
    /// `ccso_edge_clf[plane]` (`f(1)` when `quantStep != 0`, else `0`).
    pub ccso_edge_clf: Option<bool>,
    /// `ccso_max_band_log2[plane]` (`f(2 + ccso_bo_only)`). § 6.17.7.8 bounds
    /// `1 << ccso_max_band_log2 <= CCSO_BAND_NUM`.
    pub ccso_max_band_log2: Option<u8>,
}

/// Parsed `ccso_params()` (AV2 v1.0.0 § 5.18.7.12) on the intra path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CcsoParams {
    /// `ccso_frame_flag`: `1` for a single picture (no bit), else read; `None` when CCSO
    /// is disabled (the early return leaves all `ccso_planes` `0`).
    pub ccso_frame_flag: Option<bool>,
    /// Per-plane parsed state (`NumPlanes` entries; empty when CCSO is disabled or the
    /// frame flag is `0`).
    pub planes: Vec<CcsoPlaneParams>,
}

/// Parses `ccso_params()` (AV2 v1.0.0 § 5.18.7.12) on the intra path.
///
/// `coded_lossless` is the frame `CodedLossless`; `num_planes` is `NumPlanes`; `view`
/// carries the § 5.4.10 `enable_ccso` and § 5.4.1 `single_picture_header_flag`. On the
/// intra path `FrameIsIntra`, so the `reuse_ccso` / `sb_reuse_ccso` / `ccso_ref_idx` arm
/// (gated on `!(FrameIsIntra || FrameType == SWITCH_FRAME)`) is dead and
/// `load_ccso_params()` never fires. The `CcsoLumaSizeLog2` / tile-alignment (`a`)
/// derivations signal no bits, so they are not modeled here.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends mid-field.
pub fn parse_ccso_params(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: &CoreSeqCcsoView,
) -> Result<CcsoParams> {
    // AV2 § 5.18.7.12: ccso_planes[plane] = 0 for all planes; if ( CodedLossless ||
    // !enable_ccso ) return (no bits).
    if coded_lossless || !view.enable_ccso {
        return Ok(CcsoParams {
            ccso_frame_flag: None,
            planes: Vec::new(),
        });
    }

    // AV2 § 5.18.7.12: the `a` tile-alignment scan and CcsoLumaSizeLog2 derivation signal
    // no bits (pure derivations of CcsoLumaSizeLog2), so they are not modeled here.

    // if ( single_picture_header_flag ) ccso_frame_flag = 1; else ccso_frame_flag f(1).
    let ccso_frame_flag = if view.single_picture_header_flag {
        true
    } else {
        reader.read_bit()? != 0
    };
    // if ( !ccso_frame_flag ) return.
    if !ccso_frame_flag {
        return Ok(CcsoParams {
            ccso_frame_flag: Some(false),
            planes: Vec::new(),
        });
    }

    let mut planes: Vec<CcsoPlaneParams> = Vec::with_capacity(usize::from(num_planes));
    // AV2 § 5.18.7.12: for ( plane = 0; plane < NumPlanes; plane++ ).
    for _plane in 0..usize::from(num_planes) {
        // ccso_planes[plane] f(1).
        let ccso_planes = reader.read_bit()? != 0;
        let mut plane_params = CcsoPlaneParams {
            ccso_planes,
            ccso_bo_only: None,
            ccso_scale_idx: None,
            ccso_quant_idx: None,
            ccso_ext_filter: None,
            ccso_edge_clf: None,
            ccso_max_band_log2: None,
        };

        // AV2 § 5.18.7.12: the reuse arm (reuse_ccso / sb_reuse_ccso / ccso_ref_idx /
        // load_ccso_params) is gated on !(FrameIsIntra || FrameType == SWITCH_FRAME); on the
        // intra path FrameIsIntra, so reuse_ccso[plane] == 0 and the arm is dead.

        // if ( ccso_planes[plane] && !reuse_ccso[plane] ) -> on intra reuse_ccso == 0.
        if ccso_planes {
            // ccso_bo_only[plane] f(1); ccso_scale_idx[plane] f(2).
            let ccso_bo_only = reader.read_bit()? != 0;
            let ccso_scale_idx = reader.read_bits_u8(2)?;
            let (ccso_quant_idx, ccso_ext_filter, ccso_edge_clf) = if ccso_bo_only {
                // ccso_quant_idx = 0; ccso_ext_filter = 0; ccso_edge_clf = 0 (no bits).
                (0u8, 0u8, false)
            } else {
                // ccso_quant_idx[plane] f(2); ccso_ext_filter[plane] f(3).
                let ccso_quant_idx = reader.read_bits_u8(2)?;
                let ccso_ext_filter = reader.read_bits_u8(3)?;
                // quantStep = CCSO_Quant_Sz[ccso_scale_idx][ccso_quant_idx].
                let quant_step = CCSO_QUANT_SZ
                    .get(usize::from(ccso_scale_idx))
                    .and_then(|row| row.get(usize::from(ccso_quant_idx)))
                    .copied()
                    .unwrap_or(0);
                let ccso_edge_clf = if quant_step == 0 {
                    // ccso_edge_clf = 0 (no bit).
                    false
                } else {
                    // ccso_edge_clf[plane] f(1).
                    reader.read_bit()? != 0
                };
                (ccso_quant_idx, ccso_ext_filter, ccso_edge_clf)
            };

            // n = 2 + ccso_bo_only; ccso_max_band_log2[plane] f(n).
            let band_bits = 2 + u32::from(ccso_bo_only);
            let ccso_max_band_log2 = reader.read_bits_u8(band_bits)?;

            // maxEdgeInterval = CCSO_INPUT_INTERVAL - ccso_edge_clf; if ( ccso_bo_only )
            // maxEdgeInterval = 1. maxBand = 1 << ccso_max_band_log2.
            let max_edge_interval = if ccso_bo_only {
                1u32
            } else {
                CCSO_INPUT_INTERVAL - u32::from(ccso_edge_clf)
            };
            // maxBand uses a widened shift: ccso_max_band_log2 is f(2..=3) (0..=7), so the
            // shift never exceeds 7 and `1u32 << 7` cannot overflow.
            let max_band = 1u32 << u32::from(ccso_max_band_log2);

            // for d0 in 0..maxEdgeInterval, d1 in 0..maxEdgeInterval, band in 0..maxBand:
            // ccso_offset_idx tu(7).
            for _d0 in 0..max_edge_interval {
                for _d1 in 0..max_edge_interval {
                    for _band in 0..max_band {
                        // ccso_offset_idx tu(7): truncated unary in 0..=7 (§ 4.11.9).
                        read_tu(reader, 7)?;
                    }
                }
            }

            plane_params.ccso_bo_only = Some(ccso_bo_only);
            plane_params.ccso_scale_idx = Some(ccso_scale_idx);
            plane_params.ccso_quant_idx = Some(ccso_quant_idx);
            plane_params.ccso_ext_filter = Some(ccso_ext_filter);
            plane_params.ccso_edge_clf = Some(ccso_edge_clf);
            plane_params.ccso_max_band_log2 = Some(ccso_max_band_log2);
        }

        planes.push(plane_params);
    }

    Ok(CcsoParams {
        ccso_frame_flag: Some(true),
        planes,
    })
}

/// Reads a `tu(mx)` truncated-unary value (AV2 § 4.11.9): up to `mx` `1`-bits terminated
/// by a `0`, or `mx` when every bit is `1`.
fn read_tu(reader: &mut BitReader<'_>, mx: u32) -> Result<u32> {
    for idx in 0..mx {
        if reader.read_bit()? == 0 {
            return Ok(idx);
        }
    }
    Ok(mx)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    #[derive(Default)]
    struct Bits {
        bits: Vec<u8>,
    }

    impl Bits {
        fn bit(&mut self, bit: u8) {
            self.bits.push(bit & 1);
        }

        fn f(&mut self, value: u32, width: u32) {
            for shift in (0..width).rev() {
                self.bit(((value >> shift) & 1) as u8);
            }
        }

        /// `ns(n)` encoding of `value` (0..n-1), the inverse of [`BitReader::read_ns`].
        fn ns(&mut self, value: u32, n: u32) {
            let w = u32::BITS - n.leading_zeros();
            let m = (1u32 << w) - n;
            if value < m {
                // The short codeword: value in (w-1) bits.
                self.f(value, w - 1);
            } else {
                // The long codeword: (value + m) in w bits.
                self.f(value + m, w);
            }
        }

        /// `tu(mx)` encoding of `value` (0..=mx).
        fn tu(&mut self, value: u32, mx: u32) {
            for _ in 0..value {
                self.bit(1);
            }
            if value < mx {
                self.bit(0);
            }
        }

        fn into_bytes(self) -> Vec<u8> {
            let mut bytes = Vec::new();
            for chunk in self.bits.chunks(8) {
                let mut byte = 0u8;
                for (i, bit) in chunk.iter().enumerate() {
                    byte |= *bit << (7 - i);
                }
                bytes.push(byte);
            }
            bytes
        }
    }

    fn reader(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data, ByteOffset::new(0))
    }

    fn restoration_enabled() -> CoreSeqRestorationView {
        // enable_restoration with both luma switchable tools enabled (so the per-plane scan
        // can produce SWITCHABLE) and the inferred chroma PC-Wiener disable.
        CoreSeqRestorationView {
            enable_restoration: true,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: true,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    fn geom_128_420() -> LrGeometry {
        LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420)
    }

    // ---- lr_params ----

    #[test]
    fn lr_coded_lossless_reads_no_bits() {
        let mut r = reader(&[]);
        let outcome =
            parse_lr_params(&mut r, true, 3, &restoration_enabled(), geom_128_420(), 100).unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert!(!params.uses_lr);
                assert!(params.planes.is_empty());
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn lr_disabled_seq_flag_reads_no_bits() {
        let mut view = restoration_enabled();
        view.enable_restoration = false;
        let mut r = reader(&[]);
        let outcome = parse_lr_params(&mut r, false, 3, &view, geom_128_420(), 100).unwrap();
        assert!(matches!(outcome, LrParseOutcome::Parsed(p) if !p.uses_lr));
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn lr_all_planes_restore_none_completes_no_size_bits() {
        // Both switchable tools enabled for luma -> toolsCount = 3, allowSwitchable = true,
        // n = 4. tool_index = 0 -> RESTORE_NONE for every plane. Chroma: PC-Wiener disabled
        // (inferred), WIENER_NONSEP enabled -> toolsCount = 2, allowSwitchable = false, n = 2,
        // tool_index 0 -> RESTORE_NONE. No frame_filters_on, no size flags.
        let mut bits = Bits::default();
        bits.ns(0, 4); // plane 0 tool_index ns(4) == 0 -> RESTORE_NONE
        bits.ns(0, 2); // plane 1 tool_index ns(2) == 0 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 tool_index ns(2) == 0 -> RESTORE_NONE
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(
            &mut r,
            false,
            3,
            &restoration_enabled(),
            geom_128_420(),
            100,
        )
        .unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert!(!params.uses_lr);
                assert_eq!(params.planes.len(), 3);
                for plane in &params.planes {
                    assert_eq!(plane.restoration_type, FrameRestorationType::None);
                    assert!(!plane.frame_filters_on);
                }
                // Default sizes: luma 512>>3 == 64, chroma 512>>(3+1) == 32.
                assert_eq!(params.loop_restoration_size, [64, 32, 32]);
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn lr_luma_pc_wiener_no_frame_filters_reads_no_size_bits() {
        // Luma tool_index selects RESTORE_PC_WIENER (1) -> usesLumaLr, but PC_WIENER does NOT
        // read frame_filters_on (only WIENER_NONSEP/SWITCHABLE do). usesLumaLr -> luma size
        // flags read. indexToTool for luma (both enabled) = [NONE, PC_WIENER, WIENER_NONSEP,
        // SWITCHABLE]; tool_index 1 -> PC_WIENER.
        let mut bits = Bits::default();
        bits.ns(1, 4); // plane 0 tool_index == 1 -> RESTORE_PC_WIENER
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        // usesLumaLr -> luma size: lr_luma_use_half_size == 1 -> shift 1, no more flags.
        bits.bit(1);
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(
            &mut r,
            false,
            3,
            &restoration_enabled(),
            geom_128_420(),
            100,
        )
        .unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert!(params.uses_lr);
                assert_eq!(
                    params.planes[0].restoration_type,
                    FrameRestorationType::PcWiener
                );
                assert!(!params.planes[0].frame_filters_on);
                // luma half size: 512 >> 1 == 256.
                assert_eq!(params.loop_restoration_size[0], 256);
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn lr_frame_filters_on_stops_before_wienerns() {
        // Luma tool_index selects RESTORE_WIENER_NONSEP (2); frame_filters_on == 1 ->
        // num_filter_classes_idx f(3) read, then the structure would enter
        // read_wienerns_filter -> honest stop. The size-signaling phase still runs first.
        let mut bits = Bits::default();
        bits.ns(2, 4); // plane 0 tool_index == 2 -> RESTORE_WIENER_NONSEP
        bits.bit(1); // frame_filters_on[0] == 1
        bits.f(4, 3); // num_filter_classes_idx == 4 -> Decode_Num_Filter_Classes[4] == 6
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        // usesLumaLr -> luma size flag (half size).
        bits.bit(1);
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(
            &mut r,
            false,
            3,
            &restoration_enabled(),
            geom_128_420(),
            100,
        )
        .unwrap();
        match outcome {
            LrParseOutcome::StoppedBeforeWienerNsFilter {
                feature_id,
                partial,
            } => {
                assert_eq!(feature_id, WIENERNS_FILTER_FEATURE);
                assert_eq!(
                    partial.planes[0].restoration_type,
                    FrameRestorationType::WienerNonsep
                );
                assert!(partial.planes[0].frame_filters_on);
                assert_eq!(partial.planes[0].num_filter_classes, Some(6));
                // UsesLr and the size-signaling flags are derived before the stop: luma
                // RESTORE_WIENER_NONSEP uses LR, and lr_luma_use_half_size -> 512 >> 1.
                assert!(partial.uses_lr);
                assert_eq!(partial.loop_restoration_size[0], 256);
            }
            other => panic!("expected StoppedBeforeWienerNsFilter, got {other:?}"),
        }
    }

    #[test]
    fn lr_eof_mid_tool_index_is_structured_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            parse_lr_params(
                &mut r,
                false,
                3,
                &restoration_enabled(),
                geom_128_420(),
                100
            ),
            Err(crate::error::Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn lr_256x256_luma_size_single_flag() {
        // SbSize == BLOCK_256X256: after lr_luma_use_half_size == 0 the size is fixed
        // (shift 0) with no further flag. Luma PC_WIENER -> usesLumaLr.
        let geom = LrGeometry::new(SuperblockSize::Block256x256, ChromaFormatIdc::Yuv420);
        let mut bits = Bits::default();
        bits.ns(1, 4); // plane 0 -> PC_WIENER
        bits.ns(0, 2); // plane 1 -> NONE
        bits.ns(0, 2); // plane 2 -> NONE
        bits.bit(0); // lr_luma_use_half_size == 0; BLOCK_256X256 -> shift 0, no more flags
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(&mut r, false, 3, &restoration_enabled(), geom, 100).unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                // shift 0 -> 512 >> 0 == 512.
                assert_eq!(params.loop_restoration_size[0], 512);
            }
            other => panic!("expected Parsed, got {other:?}"),
        }
    }

    #[test]
    fn lr_monochrome_skips_chroma_planes() {
        // NumPlanes == 1: only the luma plane is scanned.
        let mut bits = Bits::default();
        bits.ns(0, 4); // plane 0 -> NONE
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(
            &mut r,
            false,
            1,
            &restoration_enabled(),
            LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Monochrome),
            100,
        )
        .unwrap();
        assert!(matches!(outcome, LrParseOutcome::Parsed(p) if p.planes.len() == 1));
    }

    // ---- ccso_params ----

    fn ccso_enabled() -> CoreSeqCcsoView {
        CoreSeqCcsoView {
            enable_ccso: true,
            single_picture_header_flag: false,
        }
    }

    #[test]
    fn ccso_coded_lossless_reads_no_bits() {
        let mut r = reader(&[]);
        let params = parse_ccso_params(&mut r, true, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.ccso_frame_flag, None);
        assert!(params.planes.is_empty());
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn ccso_disabled_seq_flag_reads_no_bits() {
        let mut view = ccso_enabled();
        view.enable_ccso = false;
        let mut r = reader(&[]);
        let params = parse_ccso_params(&mut r, false, 3, &view).unwrap();
        assert_eq!(params.ccso_frame_flag, None);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn ccso_frame_flag_zero_returns_early() {
        let mut bits = Bits::default();
        bits.bit(0); // ccso_frame_flag == 0
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.ccso_frame_flag, Some(false));
        assert!(params.planes.is_empty());
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn ccso_single_picture_infers_frame_flag() {
        let mut view = ccso_enabled();
        view.single_picture_header_flag = true;
        // ccso_frame_flag inferred 1 (no bit). All planes ccso_planes == 0.
        let mut bits = Bits::default();
        bits.bit(0); // ccso_planes[0]
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &view).unwrap();
        assert_eq!(params.ccso_frame_flag, Some(true));
        assert_eq!(params.planes.len(), 3);
        for plane in &params.planes {
            assert!(!plane.ccso_planes);
        }
        // 3 ccso_planes bits, no frame-flag bit.
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn ccso_plane_bo_only_reads_offsets() {
        // ccso_frame_flag read == 1; plane 0 enabled, ccso_bo_only == 1 -> quant/ext/edge_clf
        // all 0, maxEdgeInterval = 1, n = 3 band bits. maxBand = 1 << ccso_max_band_log2.
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // ccso_bo_only[0]
        bits.f(0, 2); // ccso_scale_idx[0]
        // bo_only -> no quant/ext/edge_clf reads. n = 2 + 1 = 3.
        bits.f(0, 3); // ccso_max_band_log2[0] == 0 -> maxBand = 1
        // d0 in 0..1, d1 in 0..1, band in 0..1 -> one ccso_offset_idx tu(7).
        bits.tu(0, 7); // ccso_offset_idx == 0
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.planes.len(), 3);
        assert!(params.planes[0].ccso_planes);
        assert_eq!(params.planes[0].ccso_bo_only, Some(true));
        assert_eq!(params.planes[0].ccso_quant_idx, Some(0));
        assert_eq!(params.planes[0].ccso_ext_filter, Some(0));
        assert_eq!(params.planes[0].ccso_edge_clf, Some(false));
        assert_eq!(params.planes[0].ccso_max_band_log2, Some(0));
        assert!(!params.planes[1].ccso_planes);
    }

    #[test]
    fn ccso_plane_full_arm_reads_ext_filter_and_edge_clf() {
        // ccso_bo_only == 0 -> quant_idx f(2), ext_filter f(3) read. quantStep = CCSO_Quant_Sz
        // [scale_idx 1][quant_idx 0] == 56 != 0 -> ccso_edge_clf f(1). n = 2 band bits.
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only[0] == 0
        bits.f(1, 2); // ccso_scale_idx[0] == 1
        bits.f(0, 2); // ccso_quant_idx[0] == 0 -> CCSO_Quant_Sz[1][0] == 56 != 0
        bits.f(5, 3); // ccso_ext_filter[0] == 5
        bits.bit(1); // ccso_edge_clf[0] == 1 (quantStep != 0)
        bits.f(0, 2); // ccso_max_band_log2[0] == 0 (n = 2) -> maxBand 1
        // maxEdgeInterval = CCSO_INPUT_INTERVAL - edge_clf = 3 - 1 = 2. d0 0..2, d1 0..2,
        // band 0..1 -> 4 ccso_offset_idx tu(7).
        for _ in 0..4 {
            bits.tu(1, 7); // ccso_offset_idx == 1
        }
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.planes[0].ccso_bo_only, Some(false));
        assert_eq!(params.planes[0].ccso_scale_idx, Some(1));
        assert_eq!(params.planes[0].ccso_ext_filter, Some(5));
        assert_eq!(params.planes[0].ccso_edge_clf, Some(true));
    }

    #[test]
    fn ccso_quant_step_zero_suppresses_edge_clf() {
        // CCSO_Quant_Sz[0][3] == 0 -> ccso_edge_clf inferred 0 with no bit.
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only == 0
        bits.f(0, 2); // ccso_scale_idx == 0
        bits.f(3, 2); // ccso_quant_idx == 3 -> CCSO_Quant_Sz[0][3] == 0
        bits.f(0, 3); // ccso_ext_filter
        // no ccso_edge_clf bit (quantStep == 0). n = 2.
        bits.f(0, 2); // ccso_max_band_log2 == 0 -> maxBand 1
        // maxEdgeInterval = 3 - 0 = 3 (edge_clf == 0). 3*3*1 = 9 offsets.
        for _ in 0..9 {
            bits.tu(0, 7);
        }
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.planes[0].ccso_edge_clf, Some(false));
    }

    #[test]
    fn ccso_eof_mid_frame_flag_is_structured_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            parse_ccso_params(&mut r, false, 3, &ccso_enabled()),
            Err(crate::error::Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    fn arbitrary_sb_size() -> impl Strategy<Value = SuperblockSize> {
        prop_oneof![
            Just(SuperblockSize::Block64x64),
            Just(SuperblockSize::Block128x128),
            Just(SuperblockSize::Block256x256),
        ]
    }

    fn arbitrary_chroma() -> impl Strategy<Value = ChromaFormatIdc> {
        prop_oneof![
            Just(ChromaFormatIdc::Yuv420),
            Just(ChromaFormatIdc::Monochrome),
            Just(ChromaFormatIdc::Yuv444),
            Just(ChromaFormatIdc::Yuv422),
        ]
    }

    proptest! {
        /// `lr_params()` must never panic on arbitrary input and state. The size shifts and
        /// the maxBand shift are widened so no constructed view can overflow.
        #[test]
        fn parse_lr_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..48),
            coded_lossless in any::<bool>(),
            enable_restoration in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            lr_pc_wiener_disabled in any::<bool>(),
            lr_wiener_nonsep_disabled in any::<bool>(),
            lr_uv_wiener_nonsep_disabled in any::<bool>(),
            sb_size in arbitrary_sb_size(),
            chroma in arbitrary_chroma(),
            base_q_idx in any::<u32>(),
        ) {
            let view = CoreSeqRestorationView {
                enable_restoration,
                lr_pc_wiener_disabled,
                lr_wiener_nonsep_disabled,
                lr_uv_pc_wiener_disabled: enable_restoration,
                lr_uv_wiener_nonsep_disabled,
            };
            let geometry = LrGeometry::new(sb_size, chroma);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_lr_params(&mut reader, coded_lossless, num_planes, &view, geometry, base_q_idx);
        }

        /// `ccso_params()` must never panic on arbitrary input and state. The offset triple
        /// loop is bounded by f(2..=3) band bits, so it terminates without overflow.
        #[test]
        fn parse_ccso_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
            coded_lossless in any::<bool>(),
            enable_ccso in any::<bool>(),
            num_planes in prop_oneof![Just(1u8), Just(3u8)],
            single_picture_header_flag in any::<bool>(),
        ) {
            let view = CoreSeqCcsoView {
                enable_ccso,
                single_picture_header_flag,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_ccso_params(&mut reader, coded_lossless, num_planes, &view);
        }
    }
}
