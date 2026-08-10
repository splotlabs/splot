// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 8.3.2 block-symbol CDF context derivation.

pub(crate) const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum IntraYMode {
    Dc = 0,
    Vertical = 1,
    Horizontal = 2,
    D45 = 3,
    D135 = 4,
    D113 = 5,
    D157 = 6,
    D203 = 7,
    D67 = 8,
    Smooth = 9,
    SmoothVertical = 10,
    SmoothHorizontal = 11,
    Paeth = 12,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntraYModeClass {
    Dc,
    Directional { base_angle: u16 },
    Smooth(SupportedNonDcLumaMode),
    Paeth,
}

impl IntraYMode {
    pub(crate) const fn is_directional(self) -> bool {
        matches!(self.class(), IntraYModeClass::Directional { .. })
    }

    pub(crate) const fn mode_to_angle(self) -> Option<u16> {
        match self.class() {
            IntraYModeClass::Directional { base_angle } => Some(base_angle),
            IntraYModeClass::Dc | IntraYModeClass::Smooth(_) | IntraYModeClass::Paeth => None,
        }
    }

    pub(crate) const fn is_paeth(self) -> bool {
        matches!(self, Self::Paeth)
    }

    pub(crate) const fn is_smooth(self) -> bool {
        matches!(self.class(), IntraYModeClass::Smooth(_))
    }

    pub(crate) const fn value(self) -> usize {
        self as usize
    }

    pub(crate) const fn class(self) -> IntraYModeClass {
        match self {
            Self::Dc => IntraYModeClass::Dc,
            Self::Vertical => IntraYModeClass::Directional { base_angle: 90 },
            Self::Horizontal => IntraYModeClass::Directional { base_angle: 180 },
            Self::D45 => IntraYModeClass::Directional { base_angle: 45 },
            Self::D135 => IntraYModeClass::Directional { base_angle: 135 },
            Self::D113 => IntraYModeClass::Directional { base_angle: 113 },
            Self::D157 => IntraYModeClass::Directional { base_angle: 157 },
            Self::D203 => IntraYModeClass::Directional { base_angle: 203 },
            Self::D67 => IntraYModeClass::Directional { base_angle: 67 },
            Self::Smooth => IntraYModeClass::Smooth(SupportedNonDcLumaMode::Smooth),
            Self::SmoothVertical => IntraYModeClass::Smooth(SupportedNonDcLumaMode::SmoothVertical),
            Self::SmoothHorizontal => {
                IntraYModeClass::Smooth(SupportedNonDcLumaMode::SmoothHorizontal)
            }
            Self::Paeth => IntraYModeClass::Paeth,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedNonDcLumaMode {
    Smooth,
    SmoothVertical,
    SmoothHorizontal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedDirectionalLumaMode {
    D113,
    D135,
    D157,
}

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

pub(crate) fn get_intra_uv_mode_set(y_mode: IntraYMode, uv_mode: u8) -> Option<u8> {
    let y_directional = y_mode.is_directional();
    let mut mode_idx = usize::from(uv_mode);
    if y_directional {
        if mode_idx == 0 {
            return u8::try_from(y_mode.value()).ok();
        }
        mode_idx -= 1;
    }
    for &mode in &DEFAULT_MODE_LIST_UV {
        if usize::from(mode) != y_mode.value() || !y_directional {
            if mode_idx == 0 {
                return Some(mode);
            }
            mode_idx -= 1;
        }
    }
    None
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

pub(crate) const MODE_INDEX_COUNT: u8 = 8;

#[cfg(test)]
const MODE_OFFSET_COUNT: u8 = 6;

const DIRECTIONAL_MODES_COUNT: usize = 56;

const TOTAL_ANGLE_DELTA_COUNT: usize = 7;

const MAX_ANGLE_DELTA: i8 = 3;
const INTRA_JOINT_MODE_COUNT: usize = NON_DIRECTIONAL_MODES_COUNT + DIRECTIONAL_MODES_COUNT;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum IntraModeStateError {
    #[error("general intra mode index {value} exceeds the 0..=60 domain")]
    InvalidModeIndex { value: usize },
    #[error("general intra joint mode {value} exceeds the 0..=60 domain")]
    InvalidJointMode { value: usize },
    #[error("general intra directional mode list did not contain index {mode_index}")]
    DirectionalModeListExhausted { mode_index: usize },
    #[error("general intra MRL index {value} exceeds the 0..=3 domain")]
    InvalidMrlIndex { value: u8 },
    #[error("general intra secondary MRL selector {value} exceeds the 0..=1 domain")]
    InvalidMrlSecondary { value: u8 },
    #[error("general intra MRL state is missing its secondary selector")]
    MissingMrlSecondary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeIndex(u8);

impl ModeIndex {
    pub(crate) fn try_new(value: usize) -> Result<Self, IntraModeStateError> {
        if value < INTRA_JOINT_MODE_COUNT {
            Ok(Self(value as u8))
        } else {
            Err(IntraModeStateError::InvalidModeIndex { value })
        }
    }

    pub(crate) const fn value(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraJointMode(u8);

impl IntraJointMode {
    pub(crate) const DC: Self = Self(0);

    pub(crate) fn try_new(value: usize) -> Result<Self, IntraModeStateError> {
        if value < INTRA_JOINT_MODE_COUNT {
            Ok(Self(value as u8))
        } else {
            Err(IntraModeStateError::InvalidJointMode { value })
        }
    }

    const fn from_bounded(value: usize) -> Self {
        Self(value as u8)
    }

    pub(crate) const fn value(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn is_directional(self) -> bool {
        self.value() >= NON_DIRECTIONAL_MODES_COUNT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum MrlIndex {
    One = 1,
    Two = 2,
    Three = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MrlSelection {
    Disabled,
    Primary(MrlIndex),
    Secondary(MrlIndex),
}

impl MrlSelection {
    pub(crate) fn from_symbols(
        index: u8,
        secondary: Option<u8>,
    ) -> Result<Self, IntraModeStateError> {
        if index == 0 {
            return Ok(Self::Disabled);
        }
        let index = match index {
            1 => MrlIndex::One,
            2 => MrlIndex::Two,
            3 => MrlIndex::Three,
            value => return Err(IntraModeStateError::InvalidMrlIndex { value }),
        };
        match secondary.ok_or(IntraModeStateError::MissingMrlSecondary)? {
            0 => Ok(Self::Primary(index)),
            1 => Ok(Self::Secondary(index)),
            value => Err(IntraModeStateError::InvalidMrlSecondary { value }),
        }
    }

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Disabled => 0,
            Self::Primary(index) | Self::Secondary(index) => index as usize,
        }
    }

    pub(crate) const fn is_active(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub(crate) const fn is_secondary(self) -> bool {
        matches!(self, Self::Secondary(_))
    }

    pub(crate) const fn secondary_symbol(self) -> Option<u8> {
        match self {
            Self::Disabled => None,
            Self::Primary(_) => Some(0),
            Self::Secondary(_) => Some(1),
        }
    }
}

#[rustfmt::skip]
const DEFAULT_MODE_LIST_Y: [usize; DIRECTIONAL_MODES_COUNT] = [
    17, 45, 3, 10, 24, 31, 38, 52,
    15, 19, 43, 47, 1, 5, 8, 12, 22, 26, 29, 33, 36, 40, 50, 54,
    16, 18, 44, 46, 2, 4, 9, 11, 23, 25, 30, 32, 37, 39, 51, 53,
    14, 20, 42, 48, 0, 6, 7, 13, 21, 27, 28, 34, 35, 41, 49, 55,
];

#[rustfmt::skip]
const REORDERED_Y_MODE: [IntraYMode; 13] = [
    IntraYMode::Dc,
    IntraYMode::Smooth,
    IntraYMode::SmoothVertical,
    IntraYMode::SmoothHorizontal,
    IntraYMode::Paeth,
    IntraYMode::D45,
    IntraYMode::D67,
    IntraYMode::Vertical,
    IntraYMode::D113,
    IntraYMode::D135,
    IntraYMode::D157,
    IntraYMode::Horizontal,
    IntraYMode::D203,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct YModeEscapeResult {
    pub(crate) y_mode: IntraYMode,
    pub(crate) angle_delta_y: i8,
    pub(crate) intra_joint_mode: IntraJointMode,
}

pub(crate) fn reconstruct_y_mode_with_neighbours(
    mode_idx: ModeIndex,
    neighbour_joint_modes: [IntraJointMode; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Result<YModeEscapeResult, IntraModeStateError> {
    let mode_delta = get_intra_y_mode_set(mode_idx, neighbour_joint_modes, block_n4w, block_n4h)?;
    resolve_y_mode_delta(mode_delta)
}

pub(crate) fn reconstruct_y_mode_top_left(
    mode_idx: ModeIndex,
) -> Result<YModeEscapeResult, IntraModeStateError> {
    let mode_delta = get_intra_y_mode_set_top_left(mode_idx)?;
    resolve_y_mode_delta(mode_delta)
}

fn resolve_y_mode_delta(
    intra_joint_mode: IntraJointMode,
) -> Result<YModeEscapeResult, IntraModeStateError> {
    let mode_delta = intra_joint_mode.value();
    if mode_delta < NON_DIRECTIONAL_MODES_COUNT {
        return Ok(YModeEscapeResult {
            y_mode: REORDERED_Y_MODE[mode_delta],
            angle_delta_y: 0,
            intra_joint_mode,
        });
    }
    let directional_delta = mode_delta - NON_DIRECTIONAL_MODES_COUNT;
    let reorder_index = directional_delta / TOTAL_ANGLE_DELTA_COUNT + NON_DIRECTIONAL_MODES_COUNT;
    let Some(&y_mode) = REORDERED_Y_MODE.get(reorder_index) else {
        return Err(IntraModeStateError::InvalidJointMode { value: mode_delta });
    };
    let angle_delta_y = (directional_delta % TOTAL_ANGLE_DELTA_COUNT) as i8 - MAX_ANGLE_DELTA;
    Ok(YModeEscapeResult {
        y_mode,
        angle_delta_y,
        intra_joint_mode,
    })
}

fn get_intra_y_mode_set_top_left(
    mode_idx: ModeIndex,
) -> Result<IntraJointMode, IntraModeStateError> {
    get_intra_y_mode_set(mode_idx, [IntraJointMode::DC; 2], 0, 0)
}

fn get_intra_y_mode_set(
    mode_idx: ModeIndex,
    neighbour_joint_modes: [IntraJointMode; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Result<IntraJointMode, IntraModeStateError> {
    let original_mode_idx = mode_idx.value();
    let mut mode_idx = original_mode_idx;
    if mode_idx < NON_DIRECTIONAL_MODES_COUNT {
        return Ok(IntraJointMode::from_bounded(mode_idx));
    }
    mode_idx -= NON_DIRECTIONAL_MODES_COUNT;
    let mut is_dir_selected = [false; DIRECTIONAL_MODES_COUNT];
    let mut dir_modes = [0usize; 2];
    let mut count = 0usize;

    if mi_size_at_least_block_8x8(block_n4w, block_n4h) {
        for joint_mode in neighbour_joint_modes {
            if joint_mode.is_directional() {
                let mode = joint_mode.value() - NON_DIRECTIONAL_MODES_COUNT;
                if count == 0 || mode != dir_modes[0] {
                    if mode_idx == 0 {
                        return Ok(IntraJointMode::from_bounded(
                            mode + NON_DIRECTIONAL_MODES_COUNT,
                        ));
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
                                return Ok(IntraJointMode::from_bounded(
                                    mode + NON_DIRECTIONAL_MODES_COUNT,
                                ));
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
                return Ok(IntraJointMode::from_bounded(
                    mode + NON_DIRECTIONAL_MODES_COUNT,
                ));
            }
            mode_idx -= 1;
        }
    }
    Err(IntraModeStateError::DirectionalModeListExhausted {
        mode_index: original_mode_idx,
    })
}

fn mi_size_at_least_block_8x8(block_n4w: usize, block_n4h: usize) -> bool {
    block_n4w.checked_mul(block_n4h).is_none_or(|area| area > 2)
}

fn block_area_exceeds_64_samples(block_n4w: usize, block_n4h: usize) -> bool {
    block_n4w
        .checked_mul(block_n4h)
        .is_none_or(|area_in_4x4_units| area_in_4x4_units > 4)
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

pub(crate) fn txb_skip_ctx_luma(
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
        let top = above_level_or.min(4);
        let left = left_level_or.min(4);
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
