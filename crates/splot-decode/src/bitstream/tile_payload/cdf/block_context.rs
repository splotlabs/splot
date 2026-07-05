// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 block-symbol CDF context derivation.

/// AV2 § 3 `NON_DIRECTIONAL_MODES_COUNT`.
pub(crate) const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

const DC_PRED: usize = 0;

/// AV2 § 8.3.2 `y_mode_index` and `y_mode_offset` context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeIndexContext {
    left_joint_mode: usize,
    above_joint_mode: usize,
}

impl YModeIndexContext {
    /// Tile-origin context: both joint-mode neighbours are out of frame.
    pub(crate) const fn tile_origin_block() -> Self {
        Self {
            left_joint_mode: DC_PRED,
            above_joint_mode: DC_PRED,
        }
    }

    /// The § 8.3.2 context, in `0..=2`.
    pub(crate) const fn ctx(self) -> usize {
        (self.left_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (self.above_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }
}

/// AV2 intra luma prediction mode value in § 9.2 order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraYMode(u8);

impl IntraYMode {
    /// `DC_PRED` (intra mode 0).
    pub(crate) const DC_PRED: Self = Self(0);

    const V_PRED: u8 = 1;
    const H_PRED: u8 = 2;
    const D45_PRED: u8 = 3;
    const D67_PRED: u8 = 8;
    const D203_PRED: u8 = 7;
    const D113_PRED: u8 = 5;
    const D135_PRED: u8 = 4;
    const D157_PRED: u8 = 6;
    const SMOOTH_PRED: u8 = 9;
    const SMOOTH_V_PRED: u8 = 10;
    const SMOOTH_H_PRED: u8 = 11;
    const PAETH_PRED: u8 = 12;

    /// AV2 § 5 `is_directional_mode(mode)`: true when `V_PRED <= mode <= D67_PRED`.
    pub(crate) const fn is_directional(self) -> bool {
        self.0 >= Self::V_PRED && self.0 <= Self::D67_PRED
    }

    /// AV2 § 9.2 `Mode_To_Angle[mode]` for directional luma modes.
    pub(crate) fn mode_to_angle(self) -> Option<u16> {
        MODE_TO_ANGLE.get(self.value()).copied().flatten()
    }

    /// Returns true for AV2 § 9.2 `PAETH_PRED`.
    pub(crate) const fn is_paeth(self) -> bool {
        self.0 == Self::PAETH_PRED
    }

    /// AV2 § 7.13.2.15/16 `is_smooth(mode)`.
    pub(crate) const fn is_smooth(self) -> bool {
        matches!(
            self.0,
            Self::SMOOTH_PRED | Self::SMOOTH_V_PRED | Self::SMOOTH_H_PRED
        )
    }

    /// Canonical AV2 intra-mode value.
    pub(crate) const fn value(self) -> usize {
        self.0 as usize
    }

    /// AV2 § 9.2 `V_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const V_PRED_FOR_TEST: Self = Self(Self::V_PRED);

    /// AV2 § 9.2 `D45_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const D45_PRED_FOR_TEST: Self = Self(Self::D45_PRED);

    /// AV2 § 9.2 `D67_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const D67_PRED_FOR_TEST: Self = Self(Self::D67_PRED);

    /// AV2 § 9.2 `D135_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const D135_PRED_FOR_TEST: Self = Self(Self::D135_PRED);

    /// AV2 § 9.2 `D113_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const D113_PRED_FOR_TEST: Self = Self(Self::D113_PRED);

    /// AV2 § 9.2 `D157_PRED` luma mode, for tests.
    #[cfg(test)]
    pub(crate) const D157_PRED_FOR_TEST: Self = Self(Self::D157_PRED);

    /// Maps this mode to a supported non-directional luma predictor.
    pub(crate) fn supported_nondc(self) -> Option<SupportedNonDcLumaMode> {
        SUPPORTED_NONDC_LUMA_BY_MODE
            .get(self.value())
            .copied()
            .flatten()
    }

    /// Maps this mode to a supported directional luma predictor.
    pub(crate) fn supported_directional(self) -> Option<SupportedDirectionalLumaMode> {
        SUPPORTED_DIRECTIONAL_LUMA_BY_MODE
            .get(self.value())
            .copied()
            .flatten()
    }
}

#[rustfmt::skip]
const MODE_TO_ANGLE: [Option<u16>; 13] = [
    None, Some(90), Some(180), Some(45), Some(135), Some(113), Some(157),
    Some(203), Some(67), None, None, None, None,
];

/// Supported non-DC, non-directional luma intra modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedNonDcLumaMode {
    /// AV2 `SMOOTH_PRED`.
    Smooth,
    /// AV2 `SMOOTH_V_PRED`.
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED`.
    SmoothHorizontal,
}

#[rustfmt::skip]
const SUPPORTED_NONDC_LUMA_BY_MODE: [Option<SupportedNonDcLumaMode>; 13] = [
    None, None, None, None, None, None, None, None, None,
    Some(SupportedNonDcLumaMode::Smooth),
    Some(SupportedNonDcLumaMode::SmoothVertical),
    Some(SupportedNonDcLumaMode::SmoothHorizontal),
    None,
];

/// Supported directional-angle luma intra modes with `AngleDeltaY == 0`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedDirectionalLumaMode {
    /// AV2 `V_PRED`.
    Vertical,
    /// AV2 `H_PRED`.
    Horizontal,
    /// AV2 `D113_PRED`.
    D113,
    /// AV2 `D135_PRED`.
    D135,
    /// AV2 `D157_PRED`.
    D157,
    /// AV2 `D45_PRED`.
    D45,
    /// AV2 `D203_PRED`.
    D203,
    /// AV2 `D67_PRED`.
    D67,
}

#[rustfmt::skip]
const SUPPORTED_DIRECTIONAL_LUMA_BY_MODE: [Option<SupportedDirectionalLumaMode>; 13] = [
    None,
    Some(SupportedDirectionalLumaMode::Vertical),
    Some(SupportedDirectionalLumaMode::Horizontal),
    Some(SupportedDirectionalLumaMode::D45),
    Some(SupportedDirectionalLumaMode::D135),
    Some(SupportedDirectionalLumaMode::D113),
    Some(SupportedDirectionalLumaMode::D157),
    Some(SupportedDirectionalLumaMode::D203),
    Some(SupportedDirectionalLumaMode::D67),
    None, None, None, None,
];

/// Supported chroma intra modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedChromaMode {
    /// AV2 `DC_PRED`.
    Dc,
    /// AV2 `SMOOTH_PRED`.
    Smooth,
    /// AV2 `D135_PRED` from the directional-follow branch.
    D135Follow,
    /// AV2 `D113_PRED` from the directional-follow branch.
    D113Follow,
    /// AV2 `D157_PRED` from the directional-follow branch.
    D157Follow,
    /// AV2 `V_PRED` from the directional-follow branch.
    VerticalFollow,
    /// AV2 `V_PRED` from an explicit non-follow `uv_mode`.
    Vertical,
    /// AV2 `H_PRED` from the directional-follow branch.
    HorizontalFollow,
    /// AV2 `H_PRED` from an explicit non-follow `uv_mode`.
    Horizontal,
    /// AV2 `D45_PRED` from the directional-follow branch.
    D45Follow,
    /// AV2 `D67_PRED` from the directional-follow branch.
    D67Follow,
    /// AV2 `D45_PRED` from an explicit non-follow `uv_mode`.
    D45,
    /// AV2 `D67_PRED` from an explicit non-follow `uv_mode`.
    D67,
    /// AV2 `D135_PRED` from an explicit non-follow `uv_mode`.
    D135,
    /// AV2 `D113_PRED` from an explicit non-follow `uv_mode`.
    D113,
    /// AV2 `D203_PRED` from the directional-follow branch.
    D203Follow,
    /// AV2 `D203_PRED` from an explicit non-follow `uv_mode`.
    D203,
    /// AV2 `D157_PRED` from an explicit non-follow `uv_mode`.
    D157,
    /// AV2 `PAETH_PRED` from an explicit non-follow `uv_mode`.
    Paeth,
    /// AV2 `SMOOTH_V_PRED`.
    SmoothVertical,
    /// AV2 `SMOOTH_H_PRED`.
    SmoothHorizontal,
}

impl SupportedChromaMode {
    /// The § 9.2 `Mode_To_Angle` base angle for a directional `UVMode` and whether
    /// it inherits the luma § 5.20.5.3 angle delta (`*Follow` modes derive their
    /// `AngleDeltaUV` from luma; explicit modes carry `0`). Returns `None` for a
    /// non-directional mode (DC / SMOOTH* / PAETH). The § 7.13.2.8 pAngle is
    /// `base + (inherit ? AngleDeltaY : 0) * ANGLE_STEP`, then wide-angle remapped.
    pub(crate) const fn directional_base_angle(self) -> Option<(i32, bool)> {
        match self {
            Self::VerticalFollow => Some((90, true)),
            Self::Vertical => Some((90, false)),
            Self::HorizontalFollow => Some((180, true)),
            Self::D45Follow => Some((45, true)),
            Self::D135Follow => Some((135, true)),
            Self::D113Follow => Some((113, true)),
            Self::D157Follow => Some((157, true)),
            Self::D203Follow => Some((203, true)),
            Self::D67Follow => Some((67, true)),
            Self::D45 => Some((45, false)),
            Self::D67 => Some((67, false)),
            Self::D135 => Some((135, false)),
            Self::D113 => Some((113, false)),
            Self::D203 => Some((203, false)),
            Self::D157 => Some((157, false)),
            _ => None,
        }
    }

    /// § 7.13.2.15/16 `is_smooth`: `true` for the SMOOTH / SMOOTH_V / SMOOTH_H
    /// chroma modes, which seed the § 7.13.2.17 edge-filter `filterType`.
    pub(crate) const fn is_smooth(self) -> bool {
        matches!(
            self,
            Self::Smooth | Self::SmoothVertical | Self::SmoothHorizontal
        )
    }
}

#[rustfmt::skip]
const CHROMA_FOLLOW_BY_Y_MODE: [Option<SupportedChromaMode>; 13] = [
    None,
    Some(SupportedChromaMode::VerticalFollow),
    Some(SupportedChromaMode::HorizontalFollow),
    Some(SupportedChromaMode::D45Follow),
    Some(SupportedChromaMode::D135Follow),
    Some(SupportedChromaMode::D113Follow),
    Some(SupportedChromaMode::D157Follow),
    Some(SupportedChromaMode::D203Follow),
    Some(SupportedChromaMode::D67Follow),
    None, None, None, None,
];

#[rustfmt::skip]
const CHROMA_EXPLICIT_BY_MODE: [Option<SupportedChromaMode>; 13] = [
    Some(SupportedChromaMode::Dc),
    Some(SupportedChromaMode::Vertical),
    Some(SupportedChromaMode::Horizontal),
    Some(SupportedChromaMode::D45), Some(SupportedChromaMode::D135),
    Some(SupportedChromaMode::D113), Some(SupportedChromaMode::D157),
    Some(SupportedChromaMode::D203), Some(SupportedChromaMode::D67),
    Some(SupportedChromaMode::Smooth),
    Some(SupportedChromaMode::SmoothVertical),
    Some(SupportedChromaMode::SmoothHorizontal),
    Some(SupportedChromaMode::Paeth),
];

/// AV2 § 5.20.5.3 `Default_Mode_List_Uv`.
const DEFAULT_MODE_LIST_UV: [u8; 13] = [0, 9, 10, 11, 12, 1, 2, 3, 4, 8, 5, 6, 7];

fn get_intra_uv_mode_set(y_mode: IntraYMode, uv_mode: u8) -> Option<u8> {
    let y_directional = y_mode.is_directional();
    let mut mode_idx = usize::from(uv_mode);
    if y_directional {
        if mode_idx == 0 {
            return Some(y_mode.0);
        }
        mode_idx -= 1;
    }
    for &mode in &DEFAULT_MODE_LIST_UV {
        if mode != y_mode.0 || !y_directional {
            if mode_idx == 0 {
                return Some(mode);
            }
            mode_idx -= 1;
        }
    }
    None
}

/// Resolves AV2 § 5.20.5.3 coded `uv_mode` through
/// `Default_Mode_List_Uv` / directional-follow rules to the actual `UVMode`.
pub(crate) fn resolved_chroma_uv_mode(y_mode: IntraYMode, uv_mode: u8) -> Option<u8> {
    get_intra_uv_mode_set(y_mode, uv_mode)
}

/// Resolves the supported chroma predictor from AV2 § 5.20.5.3 `uv_mode`.
pub(crate) fn supported_chroma_mode(
    y_mode: IntraYMode,
    uv_mode: u8,
) -> Option<SupportedChromaMode> {
    let uv_mode_value = get_intra_uv_mode_set(y_mode, uv_mode)?;
    if uv_mode == 0 && y_mode.is_directional() {
        return CHROMA_FOLLOW_BY_Y_MODE
            .get(usize::from(uv_mode_value))
            .copied()
            .flatten();
    }

    CHROMA_EXPLICIT_BY_MODE
        .get(usize::from(uv_mode_value))
        .copied()
        .flatten()
}

/// Reconstructs non-directional AV2 § 5.20.5.5 luma modes for `y_mode_set == 0`.
pub(crate) fn reconstruct_minimal_y_mode(y_mode_set: u8, y_mode_index: u8) -> Option<IntraYMode> {
    let y_mode_index = usize::from(y_mode_index);
    if y_mode_set != 0 || y_mode_index >= NON_DIRECTIONAL_MODES_COUNT {
        return None;
    }
    REORDERED_Y_MODE.get(y_mode_index).copied()
}

/// AV2 § 3 `MODE_INDEX_COUNT`: the number of values for `y_mode_index`
/// (`03-symbols.md`); `y_mode_index == MODE_INDEX_COUNT - 1` triggers the
/// § 5.20.5.3 `y_mode_offset` escape.
pub(crate) const MODE_INDEX_COUNT: u8 = 8;

/// AV2 § 3 `MODE_OFFSET_COUNT`: the number of values for `y_mode_offset`
/// (`03-symbols.md`); the decoded `y_mode_offset` is in `0..MODE_OFFSET_COUNT`.
const MODE_OFFSET_COUNT: u8 = 6;

/// AV2 § 3 `FIRST_MODE_COUNT` (`03-symbols.md`): number of values coded through
/// the first intra luma mode set before `y_second_mode` numbering starts.
const FIRST_MODE_COUNT: usize = 13;

/// AV2 § 3 `SECOND_MODE_COUNT` (`03-symbols.md`): number of legal
/// `y_second_mode` values. The syntax reads it as `L(4)`.
const SECOND_MODE_COUNT: u8 = 16;

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

/// Reconstructed AV2 § 5.20.5.3 luma mode facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeEscapeResult {
    /// Reconstructed typed luma `YMode`.
    pub(crate) y_mode: IntraYMode,
    /// Reconstructed `AngleDeltaY`.
    pub(crate) angle_delta_y: i8,
    /// AV2 § 5.20.5.3 `IntraJointMode`.
    pub(crate) intra_joint_mode: u8,
}

/// Reconstructs the tile-origin AV2 § 5.20.5.3 `y_mode_offset` escape.
pub(crate) fn reconstruct_y_mode_offset_escape_top_left(
    y_mode_offset: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_offset >= MODE_OFFSET_COUNT {
        return None;
    }
    let mode_idx = usize::from(MODE_INDEX_COUNT - 1) + usize::from(y_mode_offset);
    resolve_y_mode_top_left(mode_idx)
}

/// Reconstructs a tile-origin directional first-mode-set `y_mode_index`.
pub(crate) fn reconstruct_y_mode_first_set_directional_top_left(
    y_mode_index: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_index < (NON_DIRECTIONAL_MODES_COUNT as u8) || y_mode_index >= MODE_INDEX_COUNT - 1 {
        return None;
    }
    resolve_y_mode_top_left(usize::from(y_mode_index))
}

/// Reconstructs a tile-origin AV2 § 5.20.5.5 second-mode-set luma mode.
pub(crate) fn reconstruct_y_mode_second_set_top_left(
    y_mode_set: u8,
    y_second_mode: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_set == 0 || y_second_mode >= SECOND_MODE_COUNT {
        return None;
    }
    let set_offset =
        usize::from(y_mode_set.checked_sub(1)?).checked_mul(usize::from(SECOND_MODE_COUNT))?;
    let mode_idx = FIRST_MODE_COUNT
        .checked_add(set_offset)?
        .checked_add(usize::from(y_second_mode))?;
    resolve_y_mode_top_left(mode_idx)
}

/// Resolves AV2 § 5.20.5.5 `modeIdx` with decoded left/above joint modes.
pub(crate) fn reconstruct_y_mode_with_neighbours(
    mode_idx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Option<YModeEscapeResult> {
    let mode_delta = get_intra_y_mode_set(mode_idx, neighbour_joint_modes, block_n4w, block_n4h)?;
    resolve_y_mode_delta(mode_delta)
}

fn resolve_y_mode_top_left(mode_idx: usize) -> Option<YModeEscapeResult> {
    let mode_delta = get_intra_y_mode_set_top_left(mode_idx)?;
    resolve_y_mode_delta(mode_delta)
}

fn resolve_y_mode_delta(mode_delta: usize) -> Option<YModeEscapeResult> {
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
    let angle_delta_y = (mode_delta % TOTAL_ANGLE_DELTA_COUNT) as i8 - MAX_ANGLE_DELTA;
    Some(YModeEscapeResult {
        y_mode,
        angle_delta_y,
        intra_joint_mode,
    })
}

fn get_intra_y_mode_set_top_left(mode_idx: usize) -> Option<usize> {
    get_intra_y_mode_set(mode_idx, [DC_PRED as u8, DC_PRED as u8], 0, 0)
}

fn get_intra_y_mode_set(
    mode_idx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Option<usize> {
    if mode_idx < NON_DIRECTIONAL_MODES_COUNT {
        return Some(mode_idx);
    }
    let mut mode_idx = mode_idx - NON_DIRECTIONAL_MODES_COUNT;
    let mut is_dir_selected = [false; DIRECTIONAL_MODES_COUNT];
    let mut dir_modes = [0usize; 2];
    let mut count = 0usize;

    if mi_size_at_least_block_8x8(block_n4w, block_n4h) {
        for joint_mode in neighbour_joint_modes {
            if usize::from(joint_mode) >= NON_DIRECTIONAL_MODES_COUNT {
                let mode = usize::from(joint_mode) - NON_DIRECTIONAL_MODES_COUNT;
                if mode >= DIRECTIONAL_MODES_COUNT {
                    return None;
                }
                if count == 0 || mode != dir_modes[0] {
                    if mode_idx == 0 {
                        return Some(mode + NON_DIRECTIONAL_MODES_COUNT);
                    }
                    mode_idx -= 1;
                    is_dir_selected[mode] = true;
                    dir_modes[count] = mode;
                    count += 1;
                }
            }
        }

        if block_area_exceeds_64_samples(block_n4w, block_n4h) {
            for i in 1..=4usize {
                for &base_mode in dir_modes.iter().take(count) {
                    for sign in [-1isize, 1] {
                        let mode = wrap_directional_mode(base_mode, i, sign);
                        if !is_dir_selected[mode] {
                            if mode_idx == 0 {
                                return Some(mode + NON_DIRECTIONAL_MODES_COUNT);
                            }
                            mode_idx -= 1;
                            is_dir_selected[mode] = true;
                        }
                    }
                }
            }
        }
    }

    for &mode in &DEFAULT_MODE_LIST_Y {
        if !is_dir_selected[mode] {
            if mode_idx == 0 {
                return Some(mode + NON_DIRECTIONAL_MODES_COUNT);
            }
            mode_idx -= 1;
        }
    }
    None
}

fn mi_size_at_least_block_8x8(block_n4w: usize, block_n4h: usize) -> bool {
    block_n4w.checked_mul(block_n4h).is_none_or(|area| area > 2)
}

fn block_area_exceeds_64_samples(block_n4w: usize, block_n4h: usize) -> bool {
    match block_n4w.checked_mul(block_n4h) {
        Some(area_in_4x4_units) => area_in_4x4_units > 4,
        None => true,
    }
}

fn wrap_directional_mode(base_mode: usize, distance: usize, sign: isize) -> usize {
    if sign < 0 {
        (base_mode + DIRECTIONAL_MODES_COUNT - (distance % DIRECTIONAL_MODES_COUNT))
            % DIRECTIONAL_MODES_COUNT
    } else {
        (base_mode + distance) % DIRECTIONAL_MODES_COUNT
    }
}

/// AV2 § 8.3.2 `uv_mode` context.
pub(crate) const fn uv_mode_ctx(y_mode: IntraYMode) -> usize {
    y_mode.is_directional() as usize
}

const TXB_SKIP_CONTEXTS: usize = 10;

/// AV2 § 8.3.2 luma `all_zero` context.
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

/// AV2 § 8.3.2 V-plane `all_zero` context.
#[allow(clippy::fn_params_excessive_bools)]
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
        assert_eq!(YModeIndexContext::tile_origin_block().ctx(), 0);
    }

    #[test]
    fn directional_neighbours_raise_the_context() {
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
        let ctx = YModeIndexContext {
            left_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
            above_joint_mode: NON_DIRECTIONAL_MODES_COUNT - 1,
        };
        assert_eq!(ctx.ctx(), 0);
    }

    #[test]
    fn minimal_y_mode_reconstruction_maps_set0_index0_to_dc_pred() {
        assert_eq!(reconstruct_minimal_y_mode(0, 0), Some(IntraYMode::DC_PRED));
    }

    #[test]
    fn minimal_y_mode_reconstruction_covers_the_non_directional_subset() {
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
        assert_eq!(reconstruct_minimal_y_mode(1, 0), None);
        assert_eq!(
            reconstruct_minimal_y_mode(0, NON_DIRECTIONAL_MODES_COUNT as u8),
            None
        );
    }

    #[test]
    fn uv_mode_ctx_is_zero_for_dc_pred_and_one_for_directional() {
        assert_eq!(uv_mode_ctx(IntraYMode::DC_PRED), 0);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::V_PRED)), 1);
        assert_eq!(uv_mode_ctx(IntraYMode(IntraYMode::D67_PRED)), 1);
        assert_eq!(uv_mode_ctx(IntraYMode(12)), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_first_block_filling_transform_is_zero() {
        assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);
    }

    #[test]
    fn luma_txb_skip_ctx_uses_min_clamped_level_sum_when_not_filling() {
        assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
        assert_eq!(txb_skip_ctx_luma(9, 9, false, false), 5);
        assert_eq!(txb_skip_ctx_luma(1, 2, false, false), 3);
    }

    #[test]
    fn luma_txb_skip_ctx_fsc_selects_last_context() {
        assert_eq!(txb_skip_ctx_luma(0, 0, true, true), TXB_SKIP_CONTEXTS - 1);
        assert_eq!(txb_skip_ctx_luma(3, 3, false, true), TXB_SKIP_CONTEXTS - 1);
    }

    #[test]
    fn v_txb_skip_ctx_first_block_larger_chroma_is_three() {
        assert_eq!(v_txb_skip_ctx(false, false, true, false), 3);
    }

    #[test]
    fn v_txb_skip_ctx_adds_neighbour_chroma_and_eob_contributions() {
        assert_eq!(v_txb_skip_ctx(false, false, false, false), 0);
        assert_eq!(v_txb_skip_ctx(true, false, false, false), 1);
        assert_eq!(v_txb_skip_ctx(true, true, false, false), 2);
        assert_eq!(v_txb_skip_ctx(true, true, true, false), 5);
        assert_eq!(v_txb_skip_ctx(true, true, true, true), 11);
    }

    #[test]
    fn y_mode_offset_escape_reconstructs_d135() {
        let escape = reconstruct_y_mode_offset_escape_top_left(3)
            .expect("y_mode_offset 3 reconstructs a mode");
        assert_eq!(
            (
                escape.y_mode,
                escape.angle_delta_y,
                escape.y_mode.supported_directional(),
                escape.y_mode.is_directional()
            ),
            (
                IntraYMode(IntraYMode::D135_PRED),
                0,
                Some(SupportedDirectionalLumaMode::D135),
                true
            )
        );
    }

    #[test]
    fn y_mode_offset_escape_rejects_out_of_range_offset() {
        assert!(reconstruct_y_mode_offset_escape_top_left(MODE_OFFSET_COUNT).is_none());
        assert!(reconstruct_y_mode_offset_escape_top_left(u8::MAX).is_none());
    }

    #[test]
    fn y_mode_offset_escape_is_total_over_the_legal_offset_range() {
        for offset in 0..MODE_OFFSET_COUNT {
            let escape = reconstruct_y_mode_offset_escape_top_left(offset)
                .expect("legal offset reconstructs");
            assert!(escape.angle_delta_y >= -MAX_ANGLE_DELTA);
            assert!(escape.angle_delta_y <= MAX_ANGLE_DELTA);
        }
    }

    #[test]
    fn get_intra_uv_mode_set_directional_luma_returns_y_mode_for_index_zero() {
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(get_intra_uv_mode_set(d135, 0), Some(IntraYMode::D135_PRED));
    }

    fn assert_supported_chroma_mode(
        y_mode: IntraYMode,
        uv_mode: u8,
        expected_uv_mode: u8,
        expected_supported: SupportedChromaMode,
    ) {
        assert_eq!(
            get_intra_uv_mode_set(y_mode, uv_mode),
            Some(expected_uv_mode)
        );
        assert_eq!(
            supported_chroma_mode(y_mode, uv_mode),
            Some(expected_supported)
        );
    }

    #[test]
    fn supported_directional_admits_all_av2_directional_luma_modes() {
        assert_eq!(
            IntraYMode(IntraYMode::D45_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D45)
        );
        assert_eq!(
            IntraYMode(IntraYMode::D203_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D203)
        );
        assert_eq!(
            IntraYMode(IntraYMode::D113_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D113)
        );
        assert_eq!(
            IntraYMode(IntraYMode::D135_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D135)
        );
        assert_eq!(
            IntraYMode(IntraYMode::D157_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D157)
        );
        assert_eq!(
            IntraYMode(IntraYMode::D67_PRED).supported_directional(),
            Some(SupportedDirectionalLumaMode::D67)
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d113_for_uv_mode_zero() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D113_PRED),
            0,
            IntraYMode::D113_PRED,
            SupportedChromaMode::D113Follow,
        );
    }

    #[test]
    fn y_mode_offset_escape_reconstructs_d113() {
        let escape = reconstruct_y_mode_offset_escape_top_left(2)
            .expect("y_mode_offset 2 reconstructs a mode");
        assert_eq!(escape.y_mode, IntraYMode(IntraYMode::D113_PRED));
        assert_eq!(escape.angle_delta_y, 0);
        assert_eq!(escape.intra_joint_mode, 29);
        assert_eq!(
            escape.y_mode.supported_directional(),
            Some(SupportedDirectionalLumaMode::D113)
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d157_for_uv_mode_zero() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D157_PRED),
            0,
            IntraYMode::D157_PRED,
            SupportedChromaMode::D157Follow,
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d203_for_uv_mode_zero() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D203_PRED),
            0,
            IntraYMode::D203_PRED,
            SupportedChromaMode::D203Follow,
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d67_for_uv_mode_zero() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D67_PRED),
            0,
            IntraYMode::D67_PRED,
            SupportedChromaMode::D67Follow,
        );
    }

    #[test]
    fn supported_chroma_mode_directional_luma_resolves_dc_for_uv_mode_one() {
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(
            supported_chroma_mode(d135, 1),
            Some(SupportedChromaMode::Dc)
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d135_for_uv_mode_zero() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D135_PRED),
            0,
            IntraYMode::D135_PRED,
            SupportedChromaMode::D135Follow,
        );
    }

    #[test]
    fn supported_chroma_mode_explicit_d135_uses_non_follow_mode() {
        assert_supported_chroma_mode(
            IntraYMode::DC_PRED,
            8,
            IntraYMode::D135_PRED,
            SupportedChromaMode::D135,
        );
    }

    #[test]
    fn supported_chroma_mode_explicit_d203_uses_non_follow_mode() {
        assert_supported_chroma_mode(
            IntraYMode::DC_PRED,
            12,
            IntraYMode::D203_PRED,
            SupportedChromaMode::D203,
        );
    }

    #[test]
    fn supported_chroma_mode_d67_luma_can_select_explicit_d203() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D67_PRED),
            12,
            IntraYMode::D203_PRED,
            SupportedChromaMode::D203,
        );
    }

    #[test]
    fn supported_chroma_mode_explicit_d157_uses_non_follow_mode() {
        assert_supported_chroma_mode(
            IntraYMode::DC_PRED,
            11,
            IntraYMode::D157_PRED,
            SupportedChromaMode::D157,
        );
    }

    #[test]
    fn supported_chroma_mode_explicit_paeth_uses_non_follow_mode() {
        assert_supported_chroma_mode(
            IntraYMode::DC_PRED,
            4,
            IntraYMode::PAETH_PRED,
            SupportedChromaMode::Paeth,
        );
    }

    #[test]
    fn supported_chroma_mode_directional_luma_can_select_explicit_paeth() {
        assert_supported_chroma_mode(
            IntraYMode(IntraYMode::D45_PRED),
            5,
            IntraYMode::PAETH_PRED,
            SupportedChromaMode::Paeth,
        );
    }

    #[test]
    fn supported_chroma_mode_non_directional_luma_passes_list_through() {
        let dc = IntraYMode::DC_PRED;
        assert_eq!(supported_chroma_mode(dc, 0), Some(SupportedChromaMode::Dc));
        assert_eq!(
            supported_chroma_mode(dc, 1),
            Some(SupportedChromaMode::Smooth)
        );
        assert_eq!(
            supported_chroma_mode(dc, 5),
            Some(SupportedChromaMode::Vertical)
        );
    }

    #[test]
    fn first_set_directional_reconstructs_v_pred_for_index_five() {
        let result = reconstruct_y_mode_first_set_directional_top_left(5)
            .expect("y_mode_index 5 reconstructs V_PRED");
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
        assert_eq!(result.angle_delta_y, 0);
        assert_eq!(result.intra_joint_mode, 22);
        assert_eq!(
            result.y_mode.supported_directional(),
            Some(SupportedDirectionalLumaMode::Vertical)
        );
    }

    #[test]
    fn first_set_directional_reconstructs_h_pred_for_index_six() {
        let result = reconstruct_y_mode_first_set_directional_top_left(6)
            .expect("y_mode_index 6 reconstructs H_PRED");
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::H_PRED));
        assert_eq!(result.angle_delta_y, 0);
        assert_eq!(result.intra_joint_mode, 50);
        assert_eq!(
            result.y_mode.supported_directional(),
            Some(SupportedDirectionalLumaMode::Horizontal)
        );
    }

    #[test]
    fn first_set_directional_rejects_non_directional_or_escape_indices() {
        for index in 0..(NON_DIRECTIONAL_MODES_COUNT as u8) {
            assert!(reconstruct_y_mode_first_set_directional_top_left(index).is_none());
        }
        assert!(reconstruct_y_mode_first_set_directional_top_left(MODE_INDEX_COUNT - 1).is_none());
        assert!(reconstruct_y_mode_first_set_directional_top_left(u8::MAX).is_none());
    }

    #[test]
    fn second_set_reconstructs_y_mode_from_y_second_mode() {
        let result = reconstruct_y_mode_second_set_top_left(1, 0)
            .expect("legal second-mode branch reconstructs");
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
        assert_eq!(result.angle_delta_y, -2);
        assert_eq!(result.intra_joint_mode, 20);
    }

    #[test]
    fn second_set_reconstructs_later_mode_sets() {
        let result = reconstruct_y_mode_second_set_top_left(2, 15)
            .expect("later legal second-mode branch reconstructs");
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::D203_PRED));
        assert_eq!(result.angle_delta_y, 1);
        assert_eq!(result.intra_joint_mode, 58);
    }

    #[test]
    fn second_set_rejects_first_set_and_out_of_range_literals() {
        assert!(reconstruct_y_mode_second_set_top_left(0, 0).is_none());
        assert!(reconstruct_y_mode_second_set_top_left(1, SECOND_MODE_COUNT).is_none());
        assert!(reconstruct_y_mode_second_set_top_left(1, u8::MAX).is_none());
    }

    #[test]
    fn neighbour_reorder_selects_directional_joint_mode_before_default_list() {
        let result = reconstruct_y_mode_with_neighbours(5, [36, 0], 16, 16)
            .expect("directional neighbour reconstructs");
        assert_eq!(result.intra_joint_mode, 36);
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::D135_PRED));
        assert_eq!(result.angle_delta_y, 0);
    }

    #[test]
    fn neighbour_reorder_skips_duplicate_directional_neighbours() {
        let result = reconstruct_y_mode_with_neighbours(6, [36, 36], 16, 16)
            .expect("duplicate directional neighbours reconstruct");
        assert_eq!(result.intra_joint_mode, 35);
    }

    #[test]
    fn neighbour_reorder_uses_default_list_after_small_block_neighbour() {
        let result = reconstruct_y_mode_with_neighbours(6, [36, 0], 2, 2)
            .expect("small block directional neighbour reconstructs");
        assert_eq!(result.intra_joint_mode, 22);
        assert_eq!(result.y_mode, IntraYMode(IntraYMode::V_PRED));
    }

    #[test]
    fn neighbour_reorder_runs_for_wide_tall_sub_8x8_blocks() {
        for (n4w, n4h) in [(1usize, 4usize), (4, 1)] {
            let result = reconstruct_y_mode_with_neighbours(5, [18, 19], n4w, n4h)
                .expect("wide/tall sub-8x8 directional neighbour reconstructs");
            assert_eq!(result.intra_joint_mode, 18, "{n4w}x{n4h} stored joint mode");
            assert_eq!(
                result.y_mode,
                IntraYMode(IntraYMode::D67_PRED),
                "{n4w}x{n4h} reconstructed YMode"
            );
        }
        for (n4w, n4h) in [(1usize, 1usize), (1, 2), (2, 1)] {
            let result = reconstruct_y_mode_with_neighbours(5, [18, 19], n4w, n4h)
                .expect("sub-8x8 small block reconstructs via the default list");
            assert_eq!(
                result.intra_joint_mode, 22,
                "{n4w}x{n4h} stays on default list"
            );
        }
    }

    #[test]
    fn supported_chroma_mode_cardinal_follow_resolves_v_and_h_for_uv_mode_zero() {
        let v = IntraYMode(IntraYMode::V_PRED);
        let h = IntraYMode(IntraYMode::H_PRED);
        assert_eq!(get_intra_uv_mode_set(v, 0), Some(IntraYMode::V_PRED));
        assert_eq!(get_intra_uv_mode_set(h, 0), Some(IntraYMode::H_PRED));
        assert_eq!(
            supported_chroma_mode(v, 0),
            Some(SupportedChromaMode::VerticalFollow)
        );
        assert_eq!(
            supported_chroma_mode(h, 0),
            Some(SupportedChromaMode::HorizontalFollow)
        );
    }

    #[test]
    fn supported_chroma_mode_cardinal_luma_with_dc_chroma_resolves_dc() {
        let v = IntraYMode(IntraYMode::V_PRED);
        assert_eq!(supported_chroma_mode(v, 1), Some(SupportedChromaMode::Dc));
    }

    #[test]
    fn supported_chroma_mode_non_follow_h_pred_over_dc_luma_resolves_horizontal() {
        let dc = IntraYMode(DC_PRED as u8);
        assert!(!dc.is_directional());
        assert_eq!(get_intra_uv_mode_set(dc, 6), Some(IntraYMode::H_PRED));
        assert_eq!(
            supported_chroma_mode(dc, 6),
            Some(SupportedChromaMode::Horizontal)
        );
    }
}
