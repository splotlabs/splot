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
//! **Frame-level Wiener NS bank.** When a plane signals `frame_filters_on[plane] == 1`
//! (the `RESTORE_WIENER_NONSEP` / `RESTORE_SWITCHABLE` frame-level-filter arm),
//! `lr_params()` calls `read_wienerns_filter(plane, 0, 0, 1)` (§ 5.18.7.11, mirror :7377)
//! at its tail. The fixed-coded frame-level path of that sub-call is modeled in
//! [`wienerns`]: it preserves the parsed `FrameLrWienerNs` class bank on
//! [`LrPlaneParams::frame_filter_bank`]. Entropy-coded LR unit filters
//! (`readFrameFilters == 0`), temporal-copy Wiener state, and reconstruction remain out of
//! scope for this parser surface.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::size::ceil_log2;
use crate::headers::sequence::{ChromaFormatIdc, SuperblockSize};

mod wienerns;

use wienerns::parse_frame_wiener_ns_filter;
pub use wienerns::{WienerNsFrameFilterBank, WienerNsFrameFilterClass};

/// `RESTORATION_TILESIZE_MAX` (AV2 v1.0.0 § 3, `docs/spec/av2/1.0.0/03-symbols.md`):
/// maximum size of a loop-restoration tile. Exposed `pub(crate)` so the § 5.18.7.11 writer
/// ([`crate::write::frame_restoration`]) can reproduce the same size-signaling base.
pub(crate) const RESTORATION_TILESIZE_MAX: u32 = 512;

/// `RESTORE_SWITCHABLE_TYPES` (AV2 § 3): `RESTORE_SWITCHABLE == 3`, the number of
/// switchable loop-restoration types scanned by the per-plane `indexToTool` loop.
const RESTORE_SWITCHABLE_TYPES: usize = 3;

/// `Decode_Num_Filter_Classes[8]` (AV2 § 5.18.7.11, mirror :7410): maps the f(3)
/// `num_filter_classes_idx` to `NumFilterClasses`.
const DECODE_NUM_FILTER_CLASSES: [u8; 8] = [1, 2, 3, 4, 6, 8, 12, 16];

/// `CCSO_INPUT_INTERVAL` (AV2 § 3): number of CCSO edge classes (mirror :7572). Exposed
/// `pub(crate)` so the § 5.18.7.12 writer ([`crate::write::frame_restoration`]) re-derives the
/// same `maxEdgeInterval`.
pub(crate) const CCSO_INPUT_INTERVAL: u32 = 3;

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

/// `quantStep = CCSO_Quant_Sz[scale_idx][quant_idx]` (AV2 § 5.18.7.12, mirror :7552), the
/// CCSO quantization step. A step of `0` suppresses the `ccso_edge_clf` read. Out-of-range
/// indices (which the f(2) reads can never produce in-band) map to `0` rather than panicking.
/// Shared with the § 5.18.7.12 writer ([`crate::write::frame_restoration`]) so the
/// edge-clf-suppression derivation never drifts between parser and writer.
pub fn ccso_quant_step(scale_idx: u8, quant_idx: u8) -> u16 {
    CCSO_QUANT_SZ
        .get(usize::from(scale_idx))
        .and_then(|row| row.get(usize::from(quant_idx)))
        .copied()
        .unwrap_or(0)
}

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

    /// Maps the enum to its `RESTORE_*` tool id (the inverse of [`Self::from_tool`]):
    /// `None -> 0`, `PcWiener -> 1`, `WienerNonsep -> 2`, `Switchable -> 3`. Used by the
    /// § 5.18.7.11 writer ([`crate::write::frame_restoration`]) to find a plane's position in
    /// the `indexToTool` table.
    pub(crate) const fn to_tool(self) -> u8 {
        match self {
            Self::None => 0,
            Self::PcWiener => 1,
            Self::WienerNonsep => 2,
            Self::Switchable => 3,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrPlaneParams {
    /// `FrameRestorationType[plane]` (selected via `tool_index ns(n)`).
    pub restoration_type: FrameRestorationType,
    /// `frame_filters_on[plane]`: whether the plane signals a frame-level Wiener filter.
    pub frame_filters_on: bool,
    /// `NumFilterClasses` derived from `num_filter_classes_idx` when
    /// `frame_filters_on[plane]` is set and not temporal; `None` when not signalled.
    pub num_filter_classes: Option<u8>,
    /// Parsed frame-level `FrameLrWienerNs[plane]` bank from
    /// `read_wienerns_filter(plane, 0, 0, 1)` (§ 5.20.10.6), present only when
    /// [`Self::frame_filters_on`] is `true` and the fixed-coded frame-level bank parsed.
    pub frame_filter_bank: Option<WienerNsFrameFilterBank>,
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

/// The partially-parsed `lr_params()` facts committed before a reserved
/// [`LrParseOutcome::StoppedBeforeWienerNsFilter`] stop (AV2 v1.0.0 § 5.18.7.11).
///
/// The fixed-coded frame-level `read_wienerns_filter()` bank is now modeled as part of a
/// complete [`LrParams`] value. This type remains separate for out-of-tree compatibility
/// and for future unsupported Wiener branches that may be detected only after the
/// `lr_params()` prefix has been consumed.
///
/// This is deliberately a *distinct* type from [`LrParams`]: a complete parse yields
/// `LrParams`, a stopped parse yields `LrPartialParams`. The two are never interchangeable,
/// so no consumer can mistake a partial parse for a complete one. The fields mirror the
/// completed `LrParams` fields up to the stop point.
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
    /// `lr_params()` parsed to completion.
    Parsed(LrParams),
    /// Reserved for a future unsupported `read_wienerns_filter()` branch whose presence is
    /// known before its bits can be safely modeled. The fixed-coded frame-level path
    /// (`readFrameFilters == 1`) is parsed to completion and stored on
    /// [`LrPlaneParams::frame_filter_bank`].
    StoppedBeforeWienerNsFilter {
        /// Implementation-matrix Feature ID for the blocking `read_wienerns_filter()` branch.
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
/// When a plane signals a frame-level Wiener filter, this consumes the fixed-coded
/// `read_wienerns_filter(plane, 0, 0, 1)` path and stores the resulting class bank on the
/// corresponding [`LrPlaneParams`].
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
    parse_lr_params_with_references(
        reader,
        coded_lossless,
        num_planes,
        *view,
        geometry,
        base_q_idx,
        0,
        [0; 3],
    )
}

/// Parses `lr_params()` for a non-switch inter path with `NumTotalRefs` already derived.
///
/// The inter grammar differs from the intra wrapper only when a Wiener-NS-capable plane has
/// `frame_filters_on[plane] == 1`: § 5.18.7.11 reads `temporal_pred_flag[plane]` when
/// `NumTotalRefs > 0`, and reads `rst_ref_pic_idx` when that flag is set and more than one
/// reference is available. Temporal-copy filter banks are represented by
/// `frame_filters_on == true` with no local `frame_filter_bank`; runtime consumers already
/// treat that as an unsupported reconstruction input until reference-filter state is modeled.
#[allow(clippy::too_many_arguments)]
pub fn parse_lr_params_for_inter(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: CoreSeqRestorationView,
    geometry: LrGeometry,
    base_q_idx: u32,
    num_ref_frames: u32,
    reference_filter_counts: [usize; 3],
) -> Result<LrParseOutcome> {
    parse_lr_params_with_references(
        reader,
        coded_lossless,
        num_planes,
        view,
        geometry,
        base_q_idx,
        num_ref_frames,
        reference_filter_counts,
    )
}

#[allow(clippy::too_many_arguments)]
fn parse_lr_params_with_references(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: CoreSeqRestorationView,
    geometry: LrGeometry,
    base_q_idx: u32,
    num_ref_frames: u32,
    reference_filter_counts: [usize; 3],
) -> Result<LrParseOutcome> {
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
    let mut temporal_pred_flags: Vec<bool> = Vec::with_capacity(usize::from(num_planes));

    for plane in 0..usize::from(num_planes) {
        let is_chroma = plane > 0;
        let (index_to_tool, _tools_count, n) = lr_plane_tool_table(view, is_chroma);
        let tool_index = reader.read_ns(n)?;
        let tool = index_to_tool.get(tool_index as usize).copied().unwrap_or(0);
        let restoration_type = FrameRestorationType::from_tool(tool);

        if restoration_type != FrameRestorationType::None {
            if plane == 0 {
                uses_luma_lr = true;
            } else {
                uses_chroma_lr = true;
            }
        }

        let mut frame_filters_on = false;
        let mut num_filter_classes: Option<u8> = None;
        let mut temporal_pred_flag = false;

        if matches!(
            restoration_type,
            FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
        ) {
            frame_filters_on = reader.read_flag()?;
            if frame_filters_on && plane == 0 {
                if num_ref_frames > 0 {
                    temporal_pred_flag = reader.read_flag()?;
                }
                if temporal_pred_flag && num_ref_frames > 1 {
                    let n = ceil_log2(num_ref_frames);
                    let _rst_ref_pic_idx = reader.read_f(n)?;
                }
                if !temporal_pred_flag && max_num_filter_classes(plane) > 1 {
                    let idx = reader.read_bits_u8(3)?;
                    let classes = DECODE_NUM_FILTER_CLASSES
                        .get(usize::from(idx))
                        .copied()
                        .unwrap_or(1);
                    num_filter_classes = Some(classes);
                }
            } else if frame_filters_on && num_ref_frames > 0 {
                temporal_pred_flag = reader.read_flag()?;
                if temporal_pred_flag && num_ref_frames > 1 {
                    let n = ceil_log2(num_ref_frames);
                    let _rst_ref_pic_idx = reader.read_f(n)?;
                }
            }
        }

        temporal_pred_flags.push(temporal_pred_flag);
        planes.push(LrPlaneParams {
            restoration_type,
            frame_filters_on,
            num_filter_classes,
            frame_filter_bank: None,
        });
    }

    let uses_lr = uses_luma_lr || uses_chroma_lr;
    let max_subsampling = u32::from(geometry.subsampling_x.max(geometry.subsampling_y));
    let mut loop_restoration_size = [
        RESTORATION_TILESIZE_MAX >> 3,
        RESTORATION_TILESIZE_MAX >> (3 + max_subsampling),
        0,
    ];

    if uses_luma_lr {
        let shift = read_lr_size_shift(reader, geometry.sb_size)?;
        loop_restoration_size[0] = RESTORATION_TILESIZE_MAX >> shift;
    }

    if uses_chroma_lr {
        let base = RESTORATION_TILESIZE_MAX >> max_subsampling;
        let shift = read_lr_size_shift(reader, geometry.sb_size)?;
        loop_restoration_size[1] = base >> shift;
    }

    loop_restoration_size[2] = loop_restoration_size[1];

    for (plane, plane_params) in planes.iter_mut().enumerate() {
        if plane_params.frame_filters_on
            && !temporal_pred_flags.get(plane).copied().unwrap_or(false)
        {
            let classes = plane_params.num_filter_classes.unwrap_or(1);
            let num_ref_filters = reference_filter_counts.get(plane).copied().unwrap_or(0);
            plane_params.frame_filter_bank = Some(parse_frame_wiener_ns_filter(
                reader,
                plane,
                classes,
                num_ref_filters,
                view,
            )?);
        }
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
    if reader.read_flag()? {
        return Ok(1);
    }
    if sb_size == SuperblockSize::Block256x256 {
        return Ok(0);
    }
    if reader.read_flag()? {
        return Ok(0);
    }
    if sb_size == SuperblockSize::Block128x128 {
        return Ok(2);
    }
    if reader.read_flag()? { Ok(2) } else { Ok(3) }
}

/// Reconstructs the per-plane `indexToTool` selection table (AV2 § 5.18.7.11, mirror
/// :7295-7312): index `0` is `RESTORE_NONE`; for `i in 1..RESTORE_SWITCHABLE_TYPES` the tool
/// `i` is appended when `lr_tools_disable[isChroma][i]` is `0` (incrementing `toolsCount`);
/// then `indexToTool[toolsCount] = RESTORE_SWITCHABLE`. Returns the filled
/// `[RESTORE_SWITCHABLE_TYPES + 1]` table, `toolsCount`, and `n = toolsCount + allowSwitchable`
/// (the `tool_index ns(n)` range, where `allowSwitchable = toolsCount > 2`).
///
/// Shared by [`parse_lr_params`] (the reader) and the § 5.18.7.11 writer
/// ([`crate::write::frame_restoration`]) so the table never drifts between the two.
pub(crate) fn lr_plane_tool_table(
    view: CoreSeqRestorationView,
    is_chroma: bool,
) -> ([u8; RESTORE_SWITCHABLE_TYPES + 1], usize, u32) {
    let mut index_to_tool = [0u8; RESTORE_SWITCHABLE_TYPES + 1];
    let mut tools_count = 1usize; // indexToTool[0] = RESTORE_NONE.
    for i in 1..RESTORE_SWITCHABLE_TYPES {
        if !view.lr_tool_disabled(is_chroma, i) {
            index_to_tool[tools_count] = i as u8;
            tools_count += 1;
        }
    }
    index_to_tool[tools_count] = RESTORE_SWITCHABLE_TYPES as u8;
    let allow_switchable = tools_count > 2;
    let n = tools_count as u32 + u32::from(allow_switchable);
    (index_to_tool, tools_count, n)
}

const fn max_num_filter_classes(_plane: usize) -> u8 {
    DECODE_NUM_FILTER_CLASSES[DECODE_NUM_FILTER_CLASSES.len() - 1]
}

/// `LoopRestorationSize` when restoration is disabled or before any signalling
/// (AV2 § 5.18.7.11, mirror :7281-7285): the default unit sizes. Exposed `pub(crate)` so the
/// § 5.18.7.11 writer ([`crate::write::frame_restoration`]) validates the disabled-arm size.
pub(crate) const fn default_restoration_size(geometry: LrGeometry) -> [u32; 3] {
    let max_subsampling = if geometry.subsampling_x > geometry.subsampling_y {
        geometry.subsampling_x as u32
    } else {
        geometry.subsampling_y as u32
    };
    let chroma = RESTORATION_TILESIZE_MAX >> (3 + max_subsampling);
    [RESTORATION_TILESIZE_MAX >> 3, chroma, chroma]
}

/// One plane's parsed `ccso_params()` state (AV2 § 5.18.7.12).
///
/// Not `Copy`: [`Self::ccso_offset_idx`] owns the per-plane `ccso_offset_idx` array.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The per-plane `ccso_offset_idx` values (§ 5.18.7.12,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-12`), each a `tu(7)` value in
    /// `0..=7`, read in `(d0, d1, band)` iteration order over
    /// `0 <= d0, d1 < maxEdgeInterval` and `0 <= band < maxBand` (so the length is
    /// `maxEdgeInterval * maxEdgeInterval * maxBand`). Empty when `ccso_planes[plane] == 0`
    /// (no offsets are coded). These were previously read and discarded; they are surfaced so
    /// the § 5.18.7.12 writer can reproduce them byte-exactly.
    pub ccso_offset_idx: Vec<u8>,
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
    parse_ccso_params_with_references(reader, coded_lossless, num_planes, *view, None)
}

/// Parses `ccso_params()` for a non-switch inter path with `NumTotalRefs` already derived.
///
/// When a plane enables CCSO, § 5.18.7.12 reads the inter-only `reuse_ccso` and
/// `sb_reuse_ccso` flags. If `reuse_ccso` is set, the direct per-plane coefficients are not
/// present; the returned plane keeps those direct fields as `None`, matching the existing
/// "not locally available" representation used by runtime/frontier checks.
pub fn parse_ccso_params_for_inter(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: CoreSeqCcsoView,
    num_ref_frames: u32,
) -> Result<CcsoParams> {
    parse_ccso_params_with_references(
        reader,
        coded_lossless,
        num_planes,
        view,
        Some(num_ref_frames),
    )
}

fn parse_ccso_params_with_references(
    reader: &mut BitReader<'_>,
    coded_lossless: bool,
    num_planes: u8,
    view: CoreSeqCcsoView,
    inter_num_ref_frames: Option<u32>,
) -> Result<CcsoParams> {
    if coded_lossless || !view.enable_ccso {
        return Ok(CcsoParams {
            ccso_frame_flag: None,
            planes: Vec::new(),
        });
    }

    let ccso_frame_flag = if view.single_picture_header_flag {
        true
    } else {
        reader.read_flag()?
    };
    if !ccso_frame_flag {
        return Ok(CcsoParams {
            ccso_frame_flag: Some(false),
            planes: Vec::new(),
        });
    }

    let mut planes: Vec<CcsoPlaneParams> = Vec::with_capacity(usize::from(num_planes));
    for _plane in 0..usize::from(num_planes) {
        let ccso_planes = reader.read_flag()?;
        let mut plane_params = CcsoPlaneParams {
            ccso_planes,
            ccso_bo_only: None,
            ccso_scale_idx: None,
            ccso_quant_idx: None,
            ccso_ext_filter: None,
            ccso_edge_clf: None,
            ccso_max_band_log2: None,
            ccso_offset_idx: Vec::new(),
        };

        if ccso_planes {
            let mut reuse_ccso = false;
            if let Some(num_ref_frames) = inter_num_ref_frames {
                reuse_ccso = reader.read_flag()?;
                let sb_reuse_ccso = reader.read_flag()?;
                if (reuse_ccso || sb_reuse_ccso) && num_ref_frames > 1 {
                    let n = ceil_log2(num_ref_frames);
                    let _ccso_ref_idx = reader.read_f(n)?;
                }
            }
            if reuse_ccso {
                planes.push(plane_params);
                continue;
            }

            let ccso_bo_only = reader.read_flag()?;
            let ccso_scale_idx = reader.read_bits_u8(2)?;
            let (ccso_quant_idx, ccso_ext_filter, ccso_edge_clf) = if ccso_bo_only {
                (0u8, 0u8, false)
            } else {
                let ccso_quant_idx = reader.read_bits_u8(2)?;
                let ccso_ext_filter = reader.read_bits_u8(3)?;
                let quant_step = ccso_quant_step(ccso_scale_idx, ccso_quant_idx);
                let ccso_edge_clf = if quant_step == 0 {
                    false
                } else {
                    reader.read_flag()?
                };
                (ccso_quant_idx, ccso_ext_filter, ccso_edge_clf)
            };

            let band_bits = 2 + u32::from(ccso_bo_only);
            let ccso_max_band_log2 = reader.read_bits_u8(band_bits)?;

            let max_edge_interval = if ccso_bo_only {
                1u32
            } else {
                CCSO_INPUT_INTERVAL - u32::from(ccso_edge_clf)
            };
            let max_band = 1u32 << u32::from(ccso_max_band_log2);

            let offset_count = (max_edge_interval * max_edge_interval * max_band) as usize;
            let mut ccso_offset_idx = Vec::with_capacity(offset_count);
            for _d0 in 0..max_edge_interval {
                for _d1 in 0..max_edge_interval {
                    for _band in 0..max_band {
                        ccso_offset_idx.push(read_tu(reader, 7)? as u8);
                    }
                }
            }

            plane_params.ccso_bo_only = Some(ccso_bo_only);
            plane_params.ccso_scale_idx = Some(ccso_scale_idx);
            plane_params.ccso_quant_idx = Some(ccso_quant_idx);
            plane_params.ccso_ext_filter = Some(ccso_ext_filter);
            plane_params.ccso_edge_clf = Some(ccso_edge_clf);
            plane_params.ccso_max_band_log2 = Some(ccso_max_band_log2);
            plane_params.ccso_offset_idx = ccso_offset_idx;
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

    use crate::test_bits::Bits;

    fn reader(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data, ByteOffset::new(0))
    }

    fn restoration_enabled() -> CoreSeqRestorationView {
        CoreSeqRestorationView {
            enable_restoration: true,
            lr_pc_wiener_disabled: false,
            lr_wiener_nonsep_disabled: false,
            lr_uv_pc_wiener_disabled: true,
            lr_uv_wiener_nonsep_disabled: false,
        }
    }

    fn restoration_enabled_without_luma_pc() -> CoreSeqRestorationView {
        let mut view = restoration_enabled();
        view.lr_pc_wiener_disabled = true;
        view
    }

    fn geom_128_420() -> LrGeometry {
        LrGeometry::new(SuperblockSize::Block128x128, ChromaFormatIdc::Yuv420)
    }

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
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
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
                assert_eq!(params.loop_restoration_size, [64, 32, 32]);
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        }
    }

    #[test]
    fn lr_luma_pc_wiener_no_frame_filters_reads_no_size_bits() {
        let mut bits = Bits::default();
        bits.ns(1, 4); // plane 0 tool_index == 1 -> RESTORE_PC_WIENER
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
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
                assert_eq!(params.loop_restoration_size[0], 256);
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        }
    }

    #[test]
    fn lr_frame_filters_on_parses_wienerns_bank() {
        let mut bits = Bits::default();
        bits.ns(1, 2); // plane 0 tool_index == 1 -> RESTORE_WIENER_NONSEP
        bits.bit(1); // frame_filters_on[0] == 1
        bits.f(1, 3); // num_filter_classes_idx == 1 -> Decode_Num_Filter_Classes[1] == 2
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        bits.bit(1);
        bits.bit(0); // class 1 match_index == 1
        bits.bit(1); // merged[0]
        bits.bit(1); // merged[1]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params(
            &mut r,
            false,
            3,
            &restoration_enabled_without_luma_pc(),
            geom_128_420(),
            100,
        )
        .unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert_eq!(
                    params.planes[0].restoration_type,
                    FrameRestorationType::WienerNonsep
                );
                assert!(params.planes[0].frame_filters_on);
                assert_eq!(params.planes[0].num_filter_classes, Some(2));
                assert!(params.uses_lr);
                assert_eq!(params.loop_restoration_size[0], 256);
                let bank = params.planes[0]
                    .frame_filter_bank
                    .as_ref()
                    .expect("frame_filters_on carries the parsed bank");
                assert_eq!(bank.classes.len(), 2);
                assert_eq!(bank.classes[0].match_index, 0);
                assert_eq!(bank.classes[1].match_index, 1);
                assert!(bank.classes.iter().all(|class| class.merged));
                assert!(bank.classes.iter().all(|class| class.coeffs == vec![0; 16]));
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        }
    }

    #[test]
    fn lr_inter_temporal_flag_zero_still_parses_local_wienerns_bank() {
        let mut bits = Bits::default();
        bits.ns(1, 2); // plane 0 -> RESTORE_WIENER_NONSEP
        bits.bit(1); // frame_filters_on[0]
        bits.bit(0); // temporal_pred_flag[0]
        bits.f(1, 3); // num_filter_classes_idx -> 2 classes
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        bits.bit(1); // lr_luma_use_half_size
        bits.bit(0); // class 1 match_index == 1
        bits.bit(1); // merged[0]
        bits.bit(1); // merged[1]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params_for_inter(
            &mut r,
            false,
            3,
            restoration_enabled_without_luma_pc(),
            geom_128_420(),
            100,
            1,
            [0; 3],
        )
        .unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert!(params.planes[0].frame_filters_on);
                assert_eq!(params.planes[0].num_filter_classes, Some(2));
                assert!(params.planes[0].frame_filter_bank.is_some());
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        }
    }

    #[test]
    fn lr_inter_temporal_flag_one_skips_local_wienerns_bank() {
        let mut bits = Bits::default();
        bits.ns(1, 2); // plane 0 -> RESTORE_WIENER_NONSEP
        bits.bit(1); // frame_filters_on[0]
        bits.bit(1); // temporal_pred_flag[0], rst_ref_pic_idx inferred 0 for one ref
        bits.ns(0, 2); // plane 1 -> RESTORE_NONE
        bits.ns(0, 2); // plane 2 -> RESTORE_NONE
        bits.bit(1); // lr_luma_use_half_size
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let outcome = parse_lr_params_for_inter(
            &mut r,
            false,
            3,
            restoration_enabled_without_luma_pc(),
            geom_128_420(),
            100,
            1,
            [0; 3],
        )
        .unwrap();
        match outcome {
            LrParseOutcome::Parsed(params) => {
                assert!(params.planes[0].frame_filters_on);
                assert_eq!(params.planes[0].num_filter_classes, None);
                assert!(params.planes[0].frame_filter_bank.is_none());
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
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
                assert_eq!(params.loop_restoration_size[0], 512);
            }
            other @ LrParseOutcome::StoppedBeforeWienerNsFilter { .. } => {
                panic!("expected Parsed, got {other:?}")
            }
        }
    }

    #[test]
    fn lr_monochrome_skips_chroma_planes() {
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
        assert_eq!(r.consumed_bits(), 3);
    }

    #[test]
    fn ccso_plane_bo_only_reads_offsets() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // ccso_bo_only[0]
        bits.f(0, 2); // ccso_scale_idx[0]
        bits.f(0, 3); // ccso_max_band_log2[0] == 0 -> maxBand = 1
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
        assert_eq!(params.planes[0].ccso_offset_idx, vec![0]);
        assert!(!params.planes[1].ccso_planes);
        assert!(params.planes[1].ccso_offset_idx.is_empty());
    }

    #[test]
    fn ccso_inter_reuse_flags_zero_parse_direct_plane_fields() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // reuse_ccso[0]
        bits.bit(0); // sb_reuse_ccso[0]
        bits.bit(1); // ccso_bo_only[0]
        bits.f(0, 2); // ccso_scale_idx[0]
        bits.f(0, 3); // ccso_max_band_log2[0]
        bits.tu(0, 7); // ccso_offset_idx
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params_for_inter(&mut r, false, 3, ccso_enabled(), 1).unwrap();
        assert_eq!(params.planes.len(), 3);
        assert_eq!(params.planes[0].ccso_bo_only, Some(true));
        assert_eq!(params.planes[0].ccso_offset_idx, vec![0]);
    }

    #[test]
    fn ccso_inter_reuse_flag_one_skips_direct_plane_fields() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // reuse_ccso[0]
        bits.bit(0); // sb_reuse_ccso[0], ccso_ref_idx inferred 0 for one ref
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params_for_inter(&mut r, false, 3, ccso_enabled(), 1).unwrap();
        assert_eq!(params.planes.len(), 3);
        assert!(params.planes[0].ccso_planes);
        assert_eq!(params.planes[0].ccso_bo_only, None);
        assert!(params.planes[0].ccso_offset_idx.is_empty());
    }

    #[test]
    fn ccso_plane_full_arm_reads_ext_filter_and_edge_clf() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only[0] == 0
        bits.f(1, 2); // ccso_scale_idx[0] == 1
        bits.f(0, 2); // ccso_quant_idx[0] == 0 -> CCSO_Quant_Sz[1][0] == 56 != 0
        bits.f(5, 3); // ccso_ext_filter[0] == 5
        bits.bit(1); // ccso_edge_clf[0] == 1 (quantStep != 0)
        bits.f(0, 2); // ccso_max_band_log2[0] == 0 (n = 2) -> maxBand 1
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
        assert_eq!(params.planes[0].ccso_offset_idx, vec![1, 1, 1, 1]);
    }

    #[test]
    fn ccso_quant_step_zero_suppresses_edge_clf() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(0); // ccso_bo_only == 0
        bits.f(0, 2); // ccso_scale_idx == 0
        bits.f(3, 2); // ccso_quant_idx == 3 -> CCSO_Quant_Sz[0][3] == 0
        bits.f(0, 3); // ccso_ext_filter
        bits.f(0, 2); // ccso_max_band_log2 == 0 -> maxBand 1
        for _ in 0..9 {
            bits.tu(0, 7);
        }
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.planes[0].ccso_edge_clf, Some(false));
        assert_eq!(params.planes[0].ccso_offset_idx, vec![0u8; 9]);
    }

    #[test]
    fn ccso_offset_idx_values_surface_in_iteration_order() {
        let mut bits = Bits::default();
        bits.bit(1); // ccso_frame_flag
        bits.bit(1); // ccso_planes[0]
        bits.bit(1); // ccso_bo_only[0]
        bits.f(0, 2); // ccso_scale_idx[0]
        bits.f(2, 3); // ccso_max_band_log2[0] == 2 (n = 3) -> maxBand 4
        bits.tu(0, 7); // band 0
        bits.tu(1, 7); // band 1
        bits.tu(2, 7); // band 2
        bits.tu(7, 7); // band 3 (the tu(7) all-ones terminal value)
        bits.bit(0); // ccso_planes[1]
        bits.bit(0); // ccso_planes[2]
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let params = parse_ccso_params(&mut r, false, 3, &ccso_enabled()).unwrap();
        assert_eq!(params.planes[0].ccso_offset_idx, vec![0, 1, 2, 7]);
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
