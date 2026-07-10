// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 block-symbol CDF context derivation.

pub(crate) const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

const DC_PRED: usize = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeIndexContext {
    left_joint_mode: usize,
    above_joint_mode: usize,
}

impl YModeIndexContext {
    pub(crate) const fn tile_origin_block() -> Self {
        Self {
            left_joint_mode: DC_PRED,
            above_joint_mode: DC_PRED,
        }
    }

    pub(crate) const fn ctx(self) -> usize {
        (self.left_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
            + (self.above_joint_mode >= NON_DIRECTIONAL_MODES_COUNT) as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraYMode(u8);

impl IntraYMode {
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

    pub(crate) const fn is_directional(self) -> bool {
        self.0 >= Self::V_PRED && self.0 <= Self::D67_PRED
    }

    pub(crate) fn mode_to_angle(self) -> Option<u16> {
        MODE_TO_ANGLE.get(self.value()).copied().flatten()
    }

    pub(crate) const fn dpcm_vertical() -> Self {
        Self(Self::V_PRED)
    }

    pub(crate) const fn dpcm_horizontal() -> Self {
        Self(Self::H_PRED)
    }

    pub(crate) const fn is_paeth(self) -> bool {
        self.0 == Self::PAETH_PRED
    }

    pub(crate) const fn is_smooth(self) -> bool {
        matches!(
            self.0,
            Self::SMOOTH_PRED | Self::SMOOTH_V_PRED | Self::SMOOTH_H_PRED
        )
    }

    pub(crate) const fn value(self) -> usize {
        self.0 as usize
    }

    #[cfg(test)]
    pub(crate) const V_PRED_FOR_TEST: Self = Self(Self::V_PRED);

    #[cfg(test)]
    pub(crate) const H_PRED_FOR_TEST: Self = Self(Self::H_PRED);

    #[cfg(test)]
    pub(crate) const D45_PRED_FOR_TEST: Self = Self(Self::D45_PRED);

    #[cfg(test)]
    pub(crate) const D67_PRED_FOR_TEST: Self = Self(Self::D67_PRED);

    #[cfg(test)]
    pub(crate) const SMOOTH_PRED_FOR_TEST: Self = Self(Self::SMOOTH_PRED);

    #[cfg(test)]
    pub(crate) const D203_PRED_FOR_TEST: Self = Self(Self::D203_PRED);

    #[cfg(test)]
    pub(crate) const D135_PRED_FOR_TEST: Self = Self(Self::D135_PRED);

    #[cfg(test)]
    pub(crate) const D113_PRED_FOR_TEST: Self = Self(Self::D113_PRED);

    #[cfg(test)]
    pub(crate) const D157_PRED_FOR_TEST: Self = Self(Self::D157_PRED);

    pub(crate) fn supported_nondc(self) -> Option<SupportedNonDcLumaMode> {
        SUPPORTED_NONDC_LUMA_BY_MODE
            .get(self.value())
            .copied()
            .flatten()
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedNonDcLumaMode {
    Smooth,
    SmoothVertical,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedDirectionalLumaMode {
    Vertical,
    Horizontal,
    D113,
    D135,
    D157,
    D45,
    D203,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedChromaMode {
    Dc,
    Smooth,
    D135Follow,
    D113Follow,
    D157Follow,
    VerticalFollow,
    Vertical,
    HorizontalFollow,
    Horizontal,
    D45Follow,
    D67Follow,
    D45,
    D67,
    D135,
    D113,
    D203Follow,
    D203,
    D157,
    Paeth,
    SmoothVertical,
    SmoothHorizontal,
}

impl SupportedChromaMode {
    pub(crate) const fn directional_base_angle(self) -> Option<i32> {
        match self {
            Self::VerticalFollow | Self::Vertical => Some(90),
            Self::HorizontalFollow | Self::Horizontal => Some(180),
            Self::D45Follow | Self::D45 => Some(45),
            Self::D135Follow | Self::D135 => Some(135),
            Self::D113Follow | Self::D113 => Some(113),
            Self::D157Follow | Self::D157 => Some(157),
            Self::D203Follow | Self::D203 => Some(203),
            Self::D67Follow | Self::D67 => Some(67),
            _ => None,
        }
    }

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

pub(crate) fn resolved_chroma_uv_mode(y_mode: IntraYMode, uv_mode: u8) -> Option<u8> {
    get_intra_uv_mode_set(y_mode, uv_mode)
}

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

pub(crate) fn supported_chroma_mode_value(uv_mode_value: u8) -> Option<SupportedChromaMode> {
    CHROMA_EXPLICIT_BY_MODE
        .get(usize::from(uv_mode_value))
        .copied()
        .flatten()
}

pub(crate) fn reconstruct_minimal_y_mode(y_mode_set: u8, y_mode_index: u8) -> Option<IntraYMode> {
    let y_mode_index = usize::from(y_mode_index);
    if y_mode_set != 0 || y_mode_index >= NON_DIRECTIONAL_MODES_COUNT {
        return None;
    }
    REORDERED_Y_MODE.get(y_mode_index).copied()
}

pub(crate) const MODE_INDEX_COUNT: u8 = 8;

const MODE_OFFSET_COUNT: u8 = 6;

const FIRST_MODE_COUNT: usize = 13;

const SECOND_MODE_COUNT: u8 = 16;

const DIRECTIONAL_MODES_COUNT: usize = 56;

const TOTAL_ANGLE_DELTA_COUNT: usize = 7;

const MAX_ANGLE_DELTA: i8 = 3;

#[rustfmt::skip]
const DEFAULT_MODE_LIST_Y: [usize; DIRECTIONAL_MODES_COUNT] = [
    17, 45, 3, 10, 24, 31, 38, 52,
    15, 19, 43, 47, 1, 5, 8, 12, 22, 26, 29, 33, 36, 40, 50, 54,
    16, 18, 44, 46, 2, 4, 9, 11, 23, 25, 30, 32, 37, 39, 51, 53,
    14, 20, 42, 48, 0, 6, 7, 13, 21, 27, 28, 34, 35, 41, 49, 55,
];

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeEscapeResult {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
    pub(crate) intra_joint_mode: u8,
}

pub(crate) fn reconstruct_y_mode_offset_escape_top_left(
    y_mode_offset: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_offset >= MODE_OFFSET_COUNT {
        return None;
    }
    let mode_idx = usize::from(MODE_INDEX_COUNT - 1) + usize::from(y_mode_offset);
    resolve_y_mode_top_left(mode_idx)
}

pub(crate) fn reconstruct_y_mode_first_set_directional_top_left(
    y_mode_index: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_index < (NON_DIRECTIONAL_MODES_COUNT as u8) || y_mode_index >= MODE_INDEX_COUNT - 1 {
        return None;
    }
    resolve_y_mode_top_left(usize::from(y_mode_index))
}

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

pub(crate) const fn uv_mode_ctx(y_mode: IntraYMode) -> usize {
    y_mode.is_directional() as usize
}

const TXB_SKIP_CONTEXTS: usize = 10;

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
#[path = "block_context_tests.rs"]
mod tests;
