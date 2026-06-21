// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 block-symbol CDF context derivation.
//!
//! This module derives the per-symbol `ctx` index that selects a block-symbol
//! CDF row in the § 8.3.2 Cdf selection process
//! (`docs/spec/av2/1.0.0/08-parsing-process.md#s-8-3-2`), replacing the
//! hardcoded context literals in the minimal flat-intra block-symbol trace with
//! spec-grounded derivations.
//!
//! Feature tracking: `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
//!
//! Scope: the `y_mode_index` context (single-block tile-origin, out-of-frame
//! `get_joint_mode` neighbours), the `uv_mode` context (from the reconstructed
//! luma `YMode`, for the non-directional Y-mode subset), and the `all_zero`
//! (`txb_skip` / `v_txb_skip`) context formula (the luma and V-plane § 8.3.2
//! arithmetic over caller-supplied level context and transform-block geometry).
//! The in-frame `get_joint_mode` neighbour lookup, the directional / escape /
//! second-mode `YMode` reconstruction paths, the U-plane `txb_skip` branch, and
//! the actual level-context buffers and transform-block geometry that feed the
//! `all_zero` formula (which need the § 5.20 transform-block syntax) are derived
//! by future increments.

/// AV2 § 3 `NON_DIRECTIONAL_MODES_COUNT`: the number of non-directional intra
/// modes (intra modes `0..5`); a mode value at or above this is directional.
const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

/// AV2 `DC_PRED` intra mode value (intra mode `0`); also the value
/// `get_joint_mode` returns for an out-of-frame neighbour (§ 5 `get_joint_mode`).
const DC_PRED: usize = 0;

/// AV2 § 8.3.2 `y_mode_index` (and `y_mode_offset`) CDF context derivation.
///
/// The context is
/// `ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1)
/// >= NON_DIRECTIONAL_MODES_COUNT)` (§ 8.3.2), where `get_joint_mode(dir)` reads
/// the directional joint mode of the left (`dir == 0`) or above (`dir == 1`)
/// neighbour, or returns `DC_PRED` when that neighbour is out of frame (§ 5
/// `get_joint_mode`). The resulting context is in `0..=2`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeIndexContext {
    /// `get_joint_mode(0)` — the left neighbour's joint intra mode.
    left_joint_mode: usize,
    /// `get_joint_mode(1)` — the above neighbour's joint intra mode.
    above_joint_mode: usize,
}

impl YModeIndexContext {
    /// The single-block tile-origin case: the block at `MiRow == 0`,
    /// `MiCol == 0`, whose left (`MiCol - 1`) and above (`MiRow - 1`) joint-mode
    /// neighbours are both out of frame, so `get_joint_mode` returns `DC_PRED`
    /// for each (§ 5 `get_joint_mode` / § 8.3.2).
    //
    // TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): in-frame neighbours read
    // `IntraJointModes[mvRow][mvCol]`; the minimal flat-intra frontier tracks no
    // neighbour mode state yet, so only the out-of-frame `DC_PRED` branch is
    // modelled here.
    pub(crate) const fn tile_origin_block() -> Self {
        Self {
            left_joint_mode: DC_PRED,
            above_joint_mode: DC_PRED,
        }
    }

    /// The § 8.3.2 `y_mode_index` context, in `0..=2`.
    pub(crate) const fn ctx(self) -> usize {
        (self.left_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (self.above_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }
}

/// AV2 intra luma prediction mode value, in the canonical `Mode_To_Txfm`
/// ordering (§ 9.2): `DC_PRED == 0`, the directional modes `V_PRED..=D67_PRED`
/// are `1..=8`, and the remaining non-directional modes (`SMOOTH_PRED`,
/// `SMOOTH_V_PRED`, `SMOOTH_H_PRED`, `PAETH_PRED`) are `9..=12`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraYMode(u8);

impl IntraYMode {
    /// `DC_PRED` (intra mode 0).
    pub(crate) const DC_PRED: Self = Self(0);

    /// First directional mode value (`V_PRED`), the lower `is_directional_mode`
    /// bound (§ 5 `is_directional_mode`).
    const V_PRED: u8 = 1;
    /// Last directional mode value (`D67_PRED`), the upper `is_directional_mode`
    /// bound (§ 5 `is_directional_mode`).
    const D67_PRED: u8 = 8;

    /// AV2 § 9.2 `D135_PRED` canonical luma mode value (the single directional
    /// luma mode the general intra decode currently reconstructs bit-exact).
    const D135_PRED: u8 = 4;
    /// AV2 § 9.2 `SMOOTH_PRED` canonical luma mode value (the plain 2-D
    /// § 7.13.2.13 smooth predictor that blends both the above row and left
    /// column).
    const SMOOTH_PRED: u8 = 9;
    /// AV2 § 9.2 `SMOOTH_V_PRED` canonical luma mode value.
    const SMOOTH_V_PRED: u8 = 10;
    /// AV2 § 9.2 `SMOOTH_H_PRED` canonical luma mode value.
    const SMOOTH_H_PRED: u8 = 11;

    /// AV2 § 5 `is_directional_mode(mode)`: true when `V_PRED <= mode <= D67_PRED`.
    pub(crate) const fn is_directional(self) -> bool {
        self.0 >= Self::V_PRED && self.0 <= Self::D67_PRED
    }

    /// Maps this luma mode to the non-DC predictor the general intra decode
    /// currently reconstructs (§ 7.13.2.13 smooth prediction), or `None` for
    /// `DC_PRED` and the not-yet-supported non-DC modes (`PAETH_PRED` and the
    /// directional modes, which lack oracle fixtures).
    pub(crate) const fn supported_nondc(self) -> Option<SupportedNonDcLumaMode> {
        match self.0 {
            Self::SMOOTH_PRED => Some(SupportedNonDcLumaMode::Smooth),
            Self::SMOOTH_V_PRED => Some(SupportedNonDcLumaMode::SmoothVertical),
            Self::SMOOTH_H_PRED => Some(SupportedNonDcLumaMode::SmoothHorizontal),
            _ => None,
        }
    }

    /// Maps this luma mode to the directional-angle predictor the general intra
    /// decode currently reconstructs bit-exact against the AVM/dav2d oracle, or
    /// `None` for every other mode. Only `D135_PRED` (pAngle 135, a § 7.13.2.8
    /// "middle" angle) is supported, and only over the § 7.13.2.1 no-neighbour
    /// fallback edges; the remaining directional modes and the
    /// `enable_intra_edge_filter` / IDIF / upsample edge synthesis are deferred.
    pub(crate) const fn supported_directional(self) -> Option<SupportedDirectionalLumaMode> {
        match self.0 {
            Self::D135_PRED => Some(SupportedDirectionalLumaMode::D135),
            _ => None,
        }
    }
}

/// The non-DC non-directional luma intra modes the general intra decode can
/// reconstruct today — a strict subset of the § 9.2 modes, gated to those proven
/// bit-exact against the AVM/dav2d oracle. `PAETH_PRED` remains deferred until it
/// has a single-block oracle fixture; plain `SMOOTH_PRED` is admitted only for
/// the top-left no-neighbour block (see the general intra mode gate).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedNonDcLumaMode {
    /// AV2 `SMOOTH_PRED` (§ 7.13.2.13 plain 2-D smooth prediction blending both
    /// the above row + top-right and the left column + bottom-left).
    Smooth,
    /// AV2 `SMOOTH_V_PRED` (§ 7.13.2.13 vertical smooth prediction).
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED` (§ 7.13.2.13 horizontal smooth prediction).
    SmoothHorizontal,
}

/// The directional-angle luma intra modes the general intra decode can
/// reconstruct today — a strict subset of the § 9.2 directional modes, gated to
/// those proven bit-exact against the AVM/dav2d oracle. Only `D135_PRED`
/// (pAngle 135, `AngleDeltaY == 0`) is supported, and only for the top-left
/// no-neighbour block where the § 7.13.2.8 prediction edges reduce to the
/// § 7.13.2.1 flat fallbacks and the `enable_intra_edge_filter` / IDIF / upsample
/// edge synthesis are no-ops. The other directional modes and angle deltas need
/// their own verified oracle fixtures and remain deferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedDirectionalLumaMode {
    /// AV2 `D135_PRED` (§ 7.13.2.8 directional prediction at pAngle 135).
    D135,
}

/// The chroma intra modes the general intra decode can reconstruct today — the
/// subset of § 5.20.5.3 `get_intra_uv_mode_set` outputs proven bit-exact against
/// the AVM/dav2d oracle. Other chroma modes (`SMOOTH_V/H_PRED`, `PAETH_PRED`,
/// other directional angles) need their own § 7.13 chroma predictors and remain
/// deferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedChromaMode {
    /// AV2 `DC_PRED` chroma prediction (§ 7.13.2.4).
    Dc,
    /// AV2 `SMOOTH_PRED` chroma prediction (§ 7.13.2.13).
    Smooth,
    /// AV2 `D135_PRED` directional-follow chroma prediction (§ 7.13.2.8,
    /// pAngle 135, `AngleDeltaUV == 0`). Resolved when the decoded
    /// `uv_mode == 0` makes § 5.20.5.3 `get_intra_uv_mode_set` return `YMode`
    /// (the directional-follow branch) and `YMode == D135_PRED`; the spec then
    /// sets `AngleDeltaUV = AngleDeltaY` (here `0`). Over the § 7.13.2.1
    /// no-neighbour fallback edges this reduces to the same `enableIdif == 0`
    /// bilinear middle-angle prediction the luma D135 path uses (shift `0`, IDIF
    /// is a sample copy), so it is verified bit-exact only for the top-left
    /// no-neighbour block; neighbour-having directional chroma is deferred.
    D135Follow,
}

/// AV2 § 9.2 canonical chroma mode values from `Default_Mode_List_Uv`:
/// `DC_PRED == 0`, `D135_PRED == 4`, `SMOOTH_PRED == 9`.
const DC_PRED_VALUE: u8 = 0;
const D135_PRED_VALUE: u8 = 4;
const SMOOTH_PRED_VALUE: u8 = 9;

/// AV2 § 5.20.5.3 `Default_Mode_List_Uv[UV_INTRA_MODES_CFL_NOT_ALLOWED]`: the
/// chroma intra mode list (CfL not allowed), in selection order. The first five
/// entries (`DC_PRED`, `SMOOTH_PRED`, `SMOOTH_V_PRED`, `SMOOTH_H_PRED`,
/// `PAETH_PRED`) are the non-directional modes; the remainder are directional.
/// The label order is the § 5.20.5.3 table; each entry is the canonical intra
/// mode value from § 6 (`06-syntax-structures-semantics.md` lines 6790-6815):
/// `DC=0, V=1, H=2, D45=3, D135=4, D113=5, D157=6, D203=7, D67=8, SMOOTH=9,
/// SMOOTH_V=10, SMOOTH_H=11, PAETH=12`.
const DEFAULT_MODE_LIST_UV: [u8; 13] = [
    0, // DC_PRED
    9, // SMOOTH_PRED
    10, 11, 12, // SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED
    1, 2, // V_PRED, H_PRED
    3, 4, // D45_PRED, D135_PRED
    8, 5, 6, 7, // D67_PRED, D113_PRED, D157_PRED, D203_PRED
];

/// Resolves the typed chroma `UVMode` value from the decoded `uv_mode` index via
/// the AV2 § 5.20.5.3 `get_intra_uv_mode_set(modeIdx)` process, faithful to both
/// the non-directional and directional luma branches.
///
/// When `is_directional_mode(YMode)`, `modeIdx == 0` returns `YMode`; otherwise
/// `modeIdx -= 1` and the `Default_Mode_List_Uv` scan skips the entry equal to
/// `YMode` (the `mode != YMode || !is_directional_mode(YMode)` filter). When
/// `YMode` is non-directional, no entry is skipped and the result is
/// `Default_Mode_List_Uv[uv_mode]`. Returns `None` if `uv_mode` exhausts the
/// list (malformed syntax).
fn get_intra_uv_mode_set(y_mode: IntraYMode, uv_mode: u8) -> Option<u8> {
    let y_directional = y_mode.is_directional();
    let mut mode_idx = usize::from(uv_mode);
    if y_directional {
        if mode_idx == 0 {
            return Some(y_mode.0);
        }
        mode_idx -= 1;
    }
    for &mode in DEFAULT_MODE_LIST_UV.iter() {
        if mode != y_mode.0 || !y_directional {
            if mode_idx == 0 {
                return Some(mode);
            }
            mode_idx -= 1;
        }
    }
    None
}

/// Resolves the supported chroma predictor from the decoded `uv_mode` index for
/// any luma mode the general intra decode produces (non-directional or the
/// supported directional subset), via AV2 § 5.20.5.3 `get_intra_uv_mode_set`.
/// Returns the supported chroma predictor (`DC_PRED`, `SMOOTH_PRED`, or the
/// `D135_PRED` directional follow) or `None` for any other resolved chroma mode.
///
/// `D135_PRED` is admitted only when it is the directional **follow** of a
/// directional luma mode (`uv_mode == 0` so § 5.20.5.3 returns `YMode`, with
/// `YMode == D135_PRED`): the spec then sets `AngleDeltaUV = AngleDeltaY`, which
/// is `0` for the supported luma D135 (so the chroma is pAngle 135 with no angle
/// delta). The `D135_PRED` value can also appear as a non-follow entry from the
/// `Default_Mode_List_Uv` scan paired with a non-directional luma mode; that
/// pairing (`AngleDeltaUV = 0` independently) is also pAngle 135 with no delta,
/// so it maps to the same predictor, but no oracle fixture exercises it yet, so
/// it is left to a future increment by requiring the directional-follow path.
pub(crate) fn supported_chroma_mode(
    y_mode: IntraYMode,
    uv_mode: u8,
) -> Option<SupportedChromaMode> {
    let uv_mode_value = get_intra_uv_mode_set(y_mode, uv_mode)?;
    match uv_mode_value {
        DC_PRED_VALUE => Some(SupportedChromaMode::Dc),
        SMOOTH_PRED_VALUE => Some(SupportedChromaMode::Smooth),
        // Directional-follow D135 chroma: `uv_mode == 0` over a directional luma
        // makes § 5.20.5.3 return `YMode` (`AngleDeltaUV = AngleDeltaY`). Only the
        // luma D135 (`AngleDeltaY == 0`) follow is verified, so require both the
        // follow branch (`uv_mode == 0`, directional luma) and `YMode == D135`.
        D135_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D135Follow)
        }
        _ => None,
    }
}

/// AV2 § 5 `Reordered_Y_Mode[0..NON_DIRECTIONAL_MODES_COUNT]`: the five
/// non-directional modes in reorder order — `DC_PRED`, `SMOOTH_PRED`,
/// `SMOOTH_V_PRED`, `SMOOTH_H_PRED`, `PAETH_PRED` (canonical values 0, 9, 10, 11,
/// 12).
const REORDERED_Y_MODE_NON_DIRECTIONAL: [IntraYMode; NON_DIRECTIONAL_MODES_COUNT] = [
    IntraYMode(0),
    IntraYMode(9),
    IntraYMode(10),
    IntraYMode(11),
    IntraYMode(12),
];

/// Reconstructs the typed luma `YMode` from the decoded `y_mode_set` and
/// `y_mode_index` for the supported minimal subset (§ 5 `intra_y_mode_info`,
/// `get_intra_y_mode_set`, and `Reordered_Y_Mode`).
///
/// Supported subset: `y_mode_set == 0` with a non-directional `y_mode_index`
/// (`0..NON_DIRECTIONAL_MODES_COUNT`). Then `modeIdx == y_mode_index` (the
/// `MODE_INDEX_COUNT - 1 == 7` escape never applies for these indices),
/// `get_intra_y_mode_set` passes `modeIdx` through unchanged (it is below
/// `NON_DIRECTIONAL_MODES_COUNT`), and `YMode == Reordered_Y_Mode[y_mode_index]`.
/// Returns `None` for inputs outside this subset.
//
// TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): the directional reordering
// (`modeDelta >= NON_DIRECTIONAL_MODES_COUNT`), the `y_mode_offset` escape at
// `MODE_INDEX_COUNT - 1`, and the `y_mode_set != 0` / `y_second_mode` path are
// not yet modelled.
pub(crate) fn reconstruct_minimal_y_mode(y_mode_set: u8, y_mode_index: u8) -> Option<IntraYMode> {
    if y_mode_set != 0 {
        return None;
    }
    REORDERED_Y_MODE_NON_DIRECTIONAL
        .get(usize::from(y_mode_index))
        .copied()
}

/// AV2 § 3 `MODE_INDEX_COUNT`: the number of values for `y_mode_index`
/// (`03-symbols.md`); `y_mode_index == MODE_INDEX_COUNT - 1` triggers the
/// § 5.20.5.3 `y_mode_offset` escape.
pub(crate) const MODE_INDEX_COUNT: u8 = 8;

/// AV2 § 3 `MODE_OFFSET_COUNT`: the number of values for `y_mode_offset`
/// (`03-symbols.md`); the decoded `y_mode_offset` is in `0..MODE_OFFSET_COUNT`.
const MODE_OFFSET_COUNT: u8 = 6;

/// AV2 § 3 `DIRECTIONAL_MODES_COUNT` (`03-symbols.md`): the length of
/// `Default_Mode_List_Y` and the directional-mode index modulus.
const DIRECTIONAL_MODES_COUNT: usize = 56;

/// AV2 § 3 `TOTAL_ANGLE_DELTA_COUNT` (`03-symbols.md`): the number of distinct
/// angle deltas (`-MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA`).
const TOTAL_ANGLE_DELTA_COUNT: usize = 7;

/// AV2 § 3 `MAX_ANGLE_DELTA` (`03-symbols.md`): the maximum magnitude of
/// `AngleDeltaY`, the bias subtracted from `modeDelta % TOTAL_ANGLE_DELTA_COUNT`.
const MAX_ANGLE_DELTA: i8 = 3;

/// AV2 § 5.20.5.3 `Default_Mode_List_Y[DIRECTIONAL_MODES_COUNT]`
/// (`05-syntax-structures.md` lines 11094-11099): the directional-mode selection
/// order used by `get_intra_y_mode_set` after the joint-mode neighbours are
/// exhausted. Verbatim from the spec mirror.
#[rustfmt::skip]
const DEFAULT_MODE_LIST_Y: [usize; DIRECTIONAL_MODES_COUNT] = [
    17, 45, 3, 10, 24, 31, 38, 52,
    15, 19, 43, 47, 1, 5, 8, 12, 22, 26, 29, 33, 36, 40, 50, 54,
    16, 18, 44, 46, 2, 4, 9, 11, 23, 25, 30, 32, 37, 39, 51, 53,
    14, 20, 42, 48, 0, 6, 7, 13, 21, 27, 28, 34, 35, 41, 49, 55,
];

/// AV2 § 5.20.5.3 `Reordered_Y_Mode[INTRA_MODES]` (`05-syntax-structures.md`
/// lines 11088-11092): the canonical § 9.2 intra mode value for each reorder
/// index. Index `0..NON_DIRECTIONAL_MODES_COUNT` are the non-directional modes;
/// `NON_DIRECTIONAL_MODES_COUNT..` are the directional modes
/// (`D45, D67, V, D113, D135, D157, H, D203` -> canonical 3, 8, 1, 5, 4, 6, 2, 7).
#[rustfmt::skip]
const REORDERED_Y_MODE: [IntraYMode; 13] = [
    IntraYMode(0),  // DC_PRED
    IntraYMode(9),  // SMOOTH_PRED
    IntraYMode(10), // SMOOTH_V_PRED
    IntraYMode(11), // SMOOTH_H_PRED
    IntraYMode(12), // PAETH_PRED
    IntraYMode(3),  // D45_PRED
    IntraYMode(8),  // D67_PRED
    IntraYMode(1),  // V_PRED
    IntraYMode(5),  // D113_PRED
    IntraYMode(4),  // D135_PRED
    IntraYMode(6),  // D157_PRED
    IntraYMode(2),  // H_PRED
    IntraYMode(7),  // D203_PRED
];

/// The typed luma mode and `AngleDeltaY` reconstructed from the § 5.20.5.3
/// `y_mode_offset` escape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeEscapeResult {
    /// The reconstructed typed luma `YMode`.
    pub(crate) y_mode: IntraYMode,
    /// The reconstructed `AngleDeltaY` (in `-MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA`),
    /// `0` for a non-directional reconstructed mode.
    pub(crate) angle_delta_y: i8,
    /// The AV2 § 5.20.5.3 `IntraJointMode` (`= modeDelta`,
    /// `get_intra_y_mode_set(modeIdx)`), the reorder index stored into
    /// `IntraJointModes` for the § 8.3.2 neighbour `y_mode_index` context. A
    /// directional reconstructed mode has `modeDelta >= NON_DIRECTIONAL_MODES_COUNT`.
    pub(crate) intra_joint_mode: u8,
}

/// Reconstructs the typed luma `YMode` and `AngleDeltaY` for the AV2 § 5.20.5.3
/// `y_mode_set == 0`, `y_mode_index == MODE_INDEX_COUNT - 1` `y_mode_offset`
/// escape, restricted to the **top-left block with no in-frame directional
/// joint-mode neighbours** (the single-block tile-origin case).
///
/// In that case `modeIdx = (MODE_INDEX_COUNT - 1) + y_mode_offset` and
/// `get_intra_y_mode_set(modeIdx)` simplifies: both `get_joint_mode` neighbours
/// are out of frame (`DC_PRED < NON_DIRECTIONAL_MODES_COUNT`), so neither the
/// joint-mode selection loop nor the `Block_Width * Block_Height > 64` expansion
/// selects any directional mode (`count == 0`). `modeDelta` therefore reduces to
/// the `(modeIdx - NON_DIRECTIONAL_MODES_COUNT)`-th unselected entry of
/// `Default_Mode_List_Y`, i.e. `Default_Mode_List_Y[modeIdx - NON_DIRECTIONAL_MODES_COUNT]
/// + NON_DIRECTIONAL_MODES_COUNT`. The typed `YMode` and `AngleDeltaY` then
/// follow the § 5.20.5.3 directional reorder (`Reordered_Y_Mode`,
/// `TOTAL_ANGLE_DELTA_COUNT`, `MAX_ANGLE_DELTA`).
///
/// Returns `None` for a `y_mode_offset` outside `0..MODE_OFFSET_COUNT` or any
/// arithmetic that escapes the table bounds.
//
// TODO(spec: DECODE-GENERAL-INTRA-ANGLE): the in-frame directional-neighbour
// reorder branches of `get_intra_y_mode_set` (which depend on the per-block
// `IntraJointModes` neighbour state and `Block_Width * Block_Height > 64`
// expansion) are not modelled; only the top-left no-neighbour case is supported.
pub(crate) fn reconstruct_y_mode_offset_escape_top_left(
    y_mode_offset: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_offset >= MODE_OFFSET_COUNT {
        return None;
    }
    // modeIdx = y_mode_index + y_mode_offset, with y_mode_index == MODE_INDEX_COUNT - 1.
    let mode_idx = usize::from(MODE_INDEX_COUNT - 1) + usize::from(y_mode_offset);
    // get_intra_y_mode_set, top-left no-directional-neighbour case. This is the
    // AV2 § 5.20.5.3 `IntraJointMode` (`modeDelta`) stored for the § 8.3.2
    // neighbour context; it is also `>= NON_DIRECTIONAL_MODES_COUNT` for the
    // directional reconstruction branch below.
    let mode_delta = get_intra_y_mode_set_top_left(mode_idx)?;
    // Preserve the stored `IntraJointMode` before the directional rebase, which
    // mutates `mode_delta` only for the `YMode` / `AngleDeltaY` reorder math.
    let intra_joint_mode = u8::try_from(mode_delta).ok()?;

    if mode_delta < NON_DIRECTIONAL_MODES_COUNT {
        let y_mode = *REORDERED_Y_MODE.get(mode_delta)?;
        return Some(YModeEscapeResult {
            y_mode,
            angle_delta_y: 0,
            intra_joint_mode,
        });
    }
    let mode_delta = mode_delta - NON_DIRECTIONAL_MODES_COUNT;
    let reorder_index = mode_delta / TOTAL_ANGLE_DELTA_COUNT + NON_DIRECTIONAL_MODES_COUNT;
    let y_mode = *REORDERED_Y_MODE.get(reorder_index)?;
    // AngleDeltaY = (modeDelta % TOTAL_ANGLE_DELTA_COUNT) - MAX_ANGLE_DELTA.
    let angle_delta_y = (mode_delta % TOTAL_ANGLE_DELTA_COUNT) as i8 - MAX_ANGLE_DELTA;
    Some(YModeEscapeResult {
        y_mode,
        angle_delta_y,
        intra_joint_mode,
    })
}

/// AV2 § 5.20.5.3 `get_intra_y_mode_set(modeIdx)` for the top-left block with no
/// in-frame directional joint-mode neighbours. Returns `modeDelta`.
///
/// When `modeIdx < NON_DIRECTIONAL_MODES_COUNT` the spec returns `modeIdx`
/// directly. Otherwise, with both neighbours out of frame, no directional mode is
/// pre-selected, so the result is the `(modeIdx - NON_DIRECTIONAL_MODES_COUNT)`-th
/// entry of `Default_Mode_List_Y`, biased by `NON_DIRECTIONAL_MODES_COUNT`.
/// Returns `None` if `modeIdx - NON_DIRECTIONAL_MODES_COUNT` exceeds the table.
fn get_intra_y_mode_set_top_left(mode_idx: usize) -> Option<usize> {
    if mode_idx < NON_DIRECTIONAL_MODES_COUNT {
        return Some(mode_idx);
    }
    let directional_index = mode_idx - NON_DIRECTIONAL_MODES_COUNT;
    let mode = *DEFAULT_MODE_LIST_Y.get(directional_index)?;
    Some(mode + NON_DIRECTIONAL_MODES_COUNT)
}

/// AV2 § 8.3.2 `uv_mode` (`TileUVModeCflNotAllowedCdf[ctx]`) context: `ctx`
/// equals `is_directional_mode(YMode)`, i.e. 1 when the reconstructed luma mode
/// is directional and 0 otherwise.
pub(crate) const fn uv_mode_ctx(y_mode: IntraYMode) -> usize {
    y_mode.is_directional() as usize
}

/// AV2 § 3 `TXB_SKIP_CONTEXTS`: the number of luma/U `all_zero` (txb_skip)
/// contexts; the `fsc_mode` luma branch selects the last one.
const TXB_SKIP_CONTEXTS: usize = 10;

/// AV2 § 8.3.2 luma `all_zero` (txb_skip) CDF context (`plane == 0`), selecting
/// `TileTxbSkipCdf[is_inter || fsc_mode][txSzCtx][ctx]`.
///
/// `above_level_or` / `left_level_or` are the OR-reductions of
/// `AboveLevelContext[0]` / `LeftLevelContext[0]` over the transform block's
/// in-frame 4x4 columns / rows (the caller's bounded reduction); this function
/// applies the § 8.3.2 `Min(·, 4)`. `tx_fills_block` is `bw == w && bh == h`
/// (the transform fills its plane residual block), and `fsc_active` is
/// `fsc_mode && enable_fsc`.
pub(crate) const fn txb_skip_ctx_luma(
    above_level_or: u32,
    left_level_or: u32,
    tx_fills_block: bool,
    fsc_active: bool,
) -> usize {
    if fsc_active {
        TXB_SKIP_CONTEXTS - 1
    } else if tx_fills_block {
        0
    } else {
        let top = if above_level_or < 4 {
            above_level_or
        } else {
            4
        };
        let left = if left_level_or < 4 { left_level_or } else { 4 };
        ((top + left + 3) >> 1) as usize
    }
}

/// AV2 § 8.3.2 V-plane `all_zero` (v_txb_skip) CDF context (`plane == 2`),
/// selecting `TileVTxbSkipCdf[ctx]`.
///
/// `above_nonzero` / `left_nonzero` are whether the OR of `AboveLevelContext[2]`
/// with `AboveDcContext[2]` (resp. the left arrays) over the in-frame 4x4
/// columns / rows is non-zero. `chroma_block_larger_than_tx` is `bw * bh > w * h`
/// and `eob_u_nonzero` is `EobU != 0`. (The `plane == 1` U-plane `+6` branch is
/// not modelled — the minimal trace has no U `all_zero` symbol.)
pub(crate) const fn v_txb_skip_ctx(
    above_nonzero: bool,
    left_nonzero: bool,
    chroma_block_larger_than_tx: bool,
    eob_u_nonzero: bool,
) -> usize {
    let mut ctx = above_nonzero as usize + left_nonzero as usize;
    if chroma_block_larger_than_tx {
        ctx += 3;
    }
    if eob_u_nonzero {
        ctx += 6;
    }
    ctx
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn tile_origin_block_is_dc_pred_context_zero() {
        // Both neighbours out of frame -> DC_PRED (0), which is non-directional
        // (0 < NON_DIRECTIONAL_MODES_COUNT), so each term is 0 and ctx == 0. This
        // matches the literal the minimal flat-intra trace previously hardcoded.
        assert_eq!(YModeIndexContext::tile_origin_block().ctx(), 0);
    }

    #[test]
    fn directional_neighbours_raise_the_context() {
        // A directional joint mode (>= NON_DIRECTIONAL_MODES_COUNT) on one side
        // gives ctx 1; on both sides ctx 2 (the § 8.3.2 sum of two indicators).
        let one = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT,
            above_joint_mode: DC_PRED,
        };
        assert_eq!(one.ctx(), 1);
        let both = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT,
            above_joint_mode: NON_DIRECTIONAL_MODES_COUNT + 7,
        };
        assert_eq!(both.ctx(), 2);
    }

    #[test]
    fn last_non_directional_mode_does_not_raise_the_context() {
        // The boundary: mode NON_DIRECTIONAL_MODES_COUNT - 1 is still
        // non-directional, so it contributes 0.
        let ctx = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
            above_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
        };
        assert_eq!(ctx.ctx(), 0);
    }

    #[test]
    fn minimal_y_mode_reconstruction_maps_set0_index0_to_dc_pred() {
        // The minimal flat-intra trace decodes y_mode_set == 0, y_mode_index == 0,
        // so YMode == Reordered_Y_Mode[0] == DC_PRED.
        assert_eq!(reconstruct_minimal_y_mode(0, 0), Some(IntraYMode::DC_PRED));
    }

    #[test]
    fn minimal_y_mode_reconstruction_covers_the_non_directional_subset() {
        // y_mode_set == 0 with a non-directional index maps to the reorder prefix;
        // every reconstructed mode is non-directional.
        for index in 0..NON_DIRECTIONAL_MODES_COUNT {
            let mode = reconstruct_minimal_y_mode(0, index as u8)
                .expect("non-directional index is supported");
            assert!(
                !mode.is_directional(),
                "index {index} must be non-directional"
            );
        }
    }

    #[test]
    fn minimal_y_mode_reconstruction_rejects_unsupported_inputs() {
        // A non-zero set and a directional/escape index are outside the supported
        // subset and return None (deferred to a future increment).
        assert_eq!(reconstruct_minimal_y_mode(1, 0), None);
        assert_eq!(
            reconstruct_minimal_y_mode(0, NON_DIRECTIONAL_MODES_COUNT as u8),
            None
        );
    }

    #[test]
    fn uv_mode_ctx_is_zero_for_dc_pred_and_one_for_directional() {
        // is_directional_mode(DC_PRED) == false -> ctx 0 (matches the literal the
        // trace previously hardcoded); a directional mode -> ctx 1.
        assert_eq!(uv_mode_ctx(IntraYMode::DC_PRED), 0);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::V_PRED)), 1);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::D67_PRED)), 1);
        // A non-directional mode above the directional range (PAETH_PRED) -> 0.
        assert_eq!(uv_mode_ctx(IntraYMode(12)), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_first_block_filling_transform_is_zero() {
        // The minimal trace's first luma transform block: zero level context
        // (first block, out-of-frame neighbours) and a transform that fills its
        // residual block -> the `bw == w && bh == h` branch -> ctx 0, matching
        // the value the conformant fixture forces.
        assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_uses_min_clamped_level_sum_when_not_filling() {
        // When the transform does not fill the block, ctx = (Min(top,4) +
        // Min(left,4) + 3) >> 1. Zero context -> (0+0+3)>>1 = 1; saturated
        // context -> (4+4+3)>>1 = 5 (the Min(.,4) clamp caps each term).
        assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
        assert_eq!(txb_skip_ctx_luma(9, 9, false, false), 5);
        assert_eq!(txb_skip_ctx_luma(1, 2, false, false), 3);
    }

    #[test]
    fn luma_txb_skip_ctx_fsc_selects_last_context() {
        // fsc_mode && enable_fsc -> ctx = TXB_SKIP_CONTEXTS - 1, overriding the
        // fill/level branches.
        assert_eq!(txb_skip_ctx_luma(0, 0, true, true), TXB_SKIP_CONTEXTS - 1);
        assert_eq!(txb_skip_ctx_luma(3, 3, false, true), TXB_SKIP_CONTEXTS - 1);
    }

    #[test]
    fn v_txb_skip_ctx_first_block_larger_chroma_is_three() {
        // The minimal trace's V transform block: zero level/DC context (first
        // block), a chroma residual block larger than the transform (+3), and
        // EobU == 0 (the U plane decoded all-zero) -> ctx 3, matching the value
        // the conformant fixture forces.
        assert_eq!(v_txb_skip_ctx(false, false, true, false), 3);
    }

    #[test]
    fn v_txb_skip_ctx_adds_neighbour_chroma_and_eob_contributions() {
        // ctx = (above != 0) + (left != 0), then +3 if the chroma block exceeds
        // the transform and +6 if EobU != 0.
        assert_eq!(v_txb_skip_ctx(false, false, false, false), 0);
        assert_eq!(v_txb_skip_ctx(true, false, false, false), 1);
        assert_eq!(v_txb_skip_ctx(true, true, false, false), 2);
        assert_eq!(v_txb_skip_ctx(true, true, true, false), 5);
        assert_eq!(v_txb_skip_ctx(true, true, true, true), 11);
    }

    #[test]
    fn y_mode_offset_escape_reconstructs_d135_for_the_hedge_fixture() {
        // The hedge fixture's escape: y_mode_set == 0,
        // y_mode_index == MODE_INDEX_COUNT - 1, y_mode_offset == 3 ->
        // modeIdx = 7 + 3 = 10; get_intra_y_mode_set(10) (top-left, no directional
        // neighbour) = Default_Mode_List_Y[10 - 5] + 5 = 31 + 5 = 36; directional:
        // 36 - 5 = 31; Reordered_Y_Mode[31 / 7 + 5] = Reordered_Y_Mode[9] = D135;
        // AngleDeltaY = 31 % 7 - 3 = 0.
        let escape = reconstruct_y_mode_offset_escape_top_left(3)
            .expect("y_mode_offset 3 reconstructs a mode");
        assert_eq!(escape.y_mode, IntraYMode(IntraYMode::D135_PRED));
        assert_eq!(escape.angle_delta_y, 0);
        assert_eq!(
            escape.y_mode.supported_directional(),
            Some(SupportedDirectionalLumaMode::D135)
        );
        assert!(escape.y_mode.is_directional());
    }

    #[test]
    fn y_mode_offset_escape_rejects_out_of_range_offset() {
        // y_mode_offset must be in 0..MODE_OFFSET_COUNT (0..6).
        assert!(reconstruct_y_mode_offset_escape_top_left(MODE_OFFSET_COUNT).is_none());
        assert!(reconstruct_y_mode_offset_escape_top_left(u8::MAX).is_none());
    }

    #[test]
    fn y_mode_offset_escape_is_total_over_the_legal_offset_range() {
        // Every legal y_mode_offset reconstructs a mode without panicking, and the
        // reconstructed AngleDeltaY stays in -MAX_ANGLE_DELTA..=MAX_ANGLE_DELTA.
        for offset in 0..MODE_OFFSET_COUNT {
            let escape = reconstruct_y_mode_offset_escape_top_left(offset)
                .expect("legal offset reconstructs");
            assert!(escape.angle_delta_y >= -MAX_ANGLE_DELTA);
            assert!(escape.angle_delta_y <= MAX_ANGLE_DELTA);
        }
    }

    #[test]
    fn get_intra_uv_mode_set_directional_luma_returns_y_mode_for_index_zero() {
        // is_directional_mode(D135) -> uv_mode 0 returns YMode (D135).
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(get_intra_uv_mode_set(d135, 0), Some(IntraYMode::D135_PRED));
    }

    #[test]
    fn supported_chroma_mode_directional_luma_resolves_dc_for_uv_mode_one() {
        // The original hedge fixture: directional luma (D135) with DC chroma. With
        // is_directional_mode(YMode), uv_mode 1 -> after modeIdx -= 1, the first
        // Default_Mode_List_Uv entry (DC_PRED) -> SupportedChromaMode::Dc.
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(
            supported_chroma_mode(d135, 1),
            Some(SupportedChromaMode::Dc)
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d135_for_uv_mode_zero() {
        // §5.20.5.3: with a directional luma, uv_mode 0 returns YMode itself
        // (`AngleDeltaUV = AngleDeltaY`). For YMode == D135_PRED that is the
        // directional-follow D135 chroma the decode now reconstructs.
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(get_intra_uv_mode_set(d135, 0), Some(IntraYMode::D135_PRED));
        assert_eq!(
            supported_chroma_mode(d135, 0),
            Some(SupportedChromaMode::D135Follow)
        );
    }

    #[test]
    fn supported_chroma_mode_non_follow_d135_is_deferred() {
        // The §5.20.5.3 scan can also yield D135_PRED paired with a non-directional
        // luma (`Default_Mode_List_Uv[8] == D135`, reached at uv_mode 8 for DC luma).
        // That non-follow D135 chroma pairing is not yet oracle-verified, so it is
        // deferred (only the uv_mode 0 directional-follow branch is admitted).
        let dc = IntraYMode::DC_PRED;
        assert_eq!(get_intra_uv_mode_set(dc, 8), Some(IntraYMode::D135_PRED));
        assert_eq!(supported_chroma_mode(dc, 8), None);
    }

    #[test]
    fn supported_chroma_mode_non_directional_luma_passes_list_through() {
        // For a non-directional luma mode, get_intra_uv_mode_set skips no entry, so
        // uv_mode indexes Default_Mode_List_Uv directly: 0 -> DC, 1 -> SMOOTH.
        let dc = IntraYMode::DC_PRED;
        assert_eq!(supported_chroma_mode(dc, 0), Some(SupportedChromaMode::Dc));
        assert_eq!(
            supported_chroma_mode(dc, 1),
            Some(SupportedChromaMode::Smooth)
        );
        // A directional Default_Mode_List_Uv entry (e.g. V_PRED at index 5) is not
        // a supported chroma predictor.
        assert_eq!(supported_chroma_mode(dc, 5), None);
    }
}
