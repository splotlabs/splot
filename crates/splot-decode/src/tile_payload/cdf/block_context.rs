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
pub(crate) const NON_DIRECTIONAL_MODES_COUNT: usize = 5;

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
    // TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): in-frame neighbours read
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
    /// bound (§ 5 `is_directional_mode`). `Mode_To_Angle[V_PRED] == 90`
    /// (§ 9.2), the cardinal vertical (`pAngle 90`) directional mode.
    const V_PRED: u8 = 1;
    /// AV2 § 9.2 `H_PRED` canonical luma mode value. `Mode_To_Angle[H_PRED] ==
    /// 180` (§ 9.2), the cardinal horizontal (`pAngle 180`) directional mode.
    const H_PRED: u8 = 2;
    /// AV2 § 9.2 `D45_PRED` canonical luma mode value (`Mode_To_Angle[D45_PRED]
    /// == 45`, § 9.2), the § 7.13.2.8 ZONE-1 one-sided directional mode
    /// (`pAngle < 90`, step 1) whose `dx == Dr_Intra_Derivative[45] == 64`
    /// projects up-and-right into the above-right (`base == i + 1 + j`).
    const D45_PRED: u8 = 3;
    /// Last directional mode value (`D67_PRED`), the upper `is_directional_mode`
    /// bound (§ 5 `is_directional_mode`).
    const D67_PRED: u8 = 8;

    /// AV2 § 9.2 `D203_PRED` canonical luma mode value (`Mode_To_Angle[D203_PRED]
    /// == 203`, § 9.2), the § 7.13.2.8 ZONE-3 one-sided directional mode
    /// (`pAngle > 180`, step 3) whose `dy == Dr_Intra_Derivative[270 - 203] ==
    /// Dr_Intra_Derivative[67] == 24` projects down-and-left into the below-left
    /// (`base == (((j + 1) * dy) >> 6) + i`), reading the real reconstructed left
    /// column. The symmetric mirror of `D45_PRED`.
    const D203_PRED: u8 = 7;

    /// AV2 § 9.2 `D113_PRED` canonical luma mode value (`Mode_To_Angle[D113_PRED]
    /// == 113`, § 9.2), a § 7.13.2.8 middle directional mode (`90 < pAngle < 180`)
    /// whose `dx == Dr_Intra_Derivative[180 - 113] == Dr_Intra_Derivative[67] ==
    /// 24` and `dy == Dr_Intra_Derivative[113 - 90] == Dr_Intra_Derivative[23] ==
    /// 170` (vertical-leaning) make most of its above-branch projections fall on a
    /// nonzero `shift`, genuinely exercising the luma § 7.13.2.8 IDIF 4-tap.
    const D113_PRED: u8 = 5;
    /// AV2 § 9.2 `D135_PRED` canonical luma mode value (a § 7.13.2.8 middle
    /// directional mode the general intra decode reconstructs bit-exact).
    const D135_PRED: u8 = 4;
    /// AV2 § 9.2 `D157_PRED` canonical luma mode value (`Mode_To_Angle[D157_PRED]
    /// == 157`, § 9.2), a § 7.13.2.8 middle directional mode whose nonzero-shift
    /// projections genuinely exercise the luma IDIF 4-tap.
    const D157_PRED: u8 = 6;
    /// AV2 § 9.2 `SMOOTH_PRED` canonical luma mode value (the plain 2-D
    /// § 7.13.2.13 smooth predictor that blends both the above row and left
    /// column).
    const SMOOTH_PRED: u8 = 9;
    /// AV2 § 9.2 `SMOOTH_V_PRED` canonical luma mode value.
    const SMOOTH_V_PRED: u8 = 10;
    /// AV2 § 9.2 `SMOOTH_H_PRED` canonical luma mode value.
    const SMOOTH_H_PRED: u8 = 11;
    /// AV2 § 9.2 `PAETH_PRED` canonical luma mode value.
    const PAETH_PRED: u8 = 12;

    /// AV2 § 5 `is_directional_mode(mode)`: true when `V_PRED <= mode <= D67_PRED`.
    pub(crate) const fn is_directional(self) -> bool {
        self.0 >= Self::V_PRED && self.0 <= Self::D67_PRED
    }

    /// AV2 § 9.2 `Mode_To_Angle[mode]` for a directional luma mode, or `None` for a
    /// non-directional mode (`DC_PRED` / `SMOOTH*` / `PAETH_PRED`, whose
    /// `Mode_To_Angle` entry is `0`). The §7.13.2.8 nominal angle the §5.20.5.3
    /// `pAngle = Mode_To_Angle[mode] + AngleDeltaY * ANGLE_STEP +
    /// Mrl_Index_To_Delta[MrlIndex]` derivation starts from. Values transcribed
    /// from the §9.2 `Mode_To_Angle[INTRA_MODES]` table:
    /// `{0, 90, 180, 45, 135, 113, 157, 203, 67, 0, 0, 0, 0}`.
    pub(crate) const fn mode_to_angle(self) -> Option<u16> {
        match self.0 {
            Self::V_PRED => Some(90),
            Self::H_PRED => Some(180),
            Self::D45_PRED => Some(45),
            Self::D135_PRED => Some(135),
            Self::D113_PRED => Some(113),
            Self::D157_PRED => Some(157),
            Self::D203_PRED => Some(203),
            Self::D67_PRED => Some(67),
            _ => None,
        }
    }

    /// Returns true when this mode is AV2 § 9.2 `PAETH_PRED`.
    pub(crate) const fn is_paeth(self) -> bool {
        self.0 == Self::PAETH_PRED
    }

    /// AV2 § 7.13.2.15/16 `is_smooth(mode)`: true when this luma mode is one of the
    /// § 7.13.2.13 smooth predictors (`SMOOTH_PRED`, `SMOOTH_V_PRED`,
    /// `SMOOTH_H_PRED`). Drives the § 7.13.2.7 `get_filter_type_above` /
    /// `get_filter_type_left` neighbour-smooth filter-type pick.
    pub(crate) const fn is_smooth(self) -> bool {
        self.0 == Self::SMOOTH_PRED
            || self.0 == Self::SMOOTH_V_PRED
            || self.0 == Self::SMOOTH_H_PRED
    }

    /// Canonical AV2 intra-mode value used by § 9.2 conversion tables.
    pub(crate) const fn value(self) -> usize {
        self.0 as usize
    }

    /// AV2 § 9.2 `H_PRED` luma mode (cardinal horizontal, pAngle 180). Test-only:
    /// the reconstruction-sink tests need a concrete non-DC directional mode value.
    #[cfg(test)]
    pub(crate) const H_PRED_FOR_TEST: Self = Self(Self::H_PRED);

    /// AV2 § 9.2 `V_PRED` luma mode (cardinal vertical, pAngle 90). Test-only.
    #[cfg(test)]
    pub(crate) const V_PRED_FOR_TEST: Self = Self(Self::V_PRED);

    /// AV2 § 9.2 `D135_PRED` luma mode (a § 7.13.2.8 middle angle). Test-only: the
    /// sink tests assert an angular directional mode is DEFERRED.
    #[cfg(test)]
    pub(crate) const D135_PRED_FOR_TEST: Self = Self(Self::D135_PRED);

    /// Maps this luma mode to the non-DC predictor the general intra decode
    /// currently reconstructs (§ 7.13.2.13 smooth prediction), or `None` for
    /// `DC_PRED`, the unsupported `PAETH_PRED`, and the directional modes (which
    /// `supported_directional` maps instead).
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
    /// `None` for every other mode. Supported:
    /// - `V_PRED` (pAngle 90) and `H_PRED` (pAngle 180): the two cardinal
    ///   § 7.13.2.8 directions whose prediction is a degenerate sample copy
    ///   (step 4 / step 5: `pred[i][j] = AboveRow[j]` / `LeftCol[i]`), reading
    ///   ONLY the above row (V) or the left column (H) — no corner, no IDIF, no
    ///   `useIBP` (which requires `pAngle < 90 || pAngle > 180`).
    /// - `D113_PRED` (pAngle 113), `D135_PRED` (pAngle 135) and `D157_PRED`
    ///   (pAngle 157), three § 7.13.2.8 "middle" angles. D135's projections all
    ///   have `shift == 0` (the IDIF 4-tap reduces to a copy); D113's (mostly
    ///   above-branch, vertical-leaning) and D157's (mostly left-branch,
    ///   horizontal-leaning) nonzero-shift projections exercise the real luma
    ///   IDIF 4-tap filter.
    ///
    /// `None` for every other directional mode and for any non-zero
    /// `AngleDeltaY` (the caller filters those out before this is reached).
    pub(crate) const fn supported_directional(self) -> Option<SupportedDirectionalLumaMode> {
        match self.0 {
            Self::V_PRED => Some(SupportedDirectionalLumaMode::Vertical),
            Self::H_PRED => Some(SupportedDirectionalLumaMode::Horizontal),
            Self::D45_PRED => Some(SupportedDirectionalLumaMode::D45),
            Self::D203_PRED => Some(SupportedDirectionalLumaMode::D203),
            Self::D113_PRED => Some(SupportedDirectionalLumaMode::D113),
            Self::D135_PRED => Some(SupportedDirectionalLumaMode::D135),
            Self::D157_PRED => Some(SupportedDirectionalLumaMode::D157),
            _ => None,
        }
    }
}

/// The non-DC non-directional luma intra modes the general intra decode can
/// reconstruct today — a strict subset of the § 9.2 modes, gated to those proven
/// bit-exact against the AVM/dav2d oracle. `PAETH_PRED` is not yet supported;
/// plain `SMOOTH_PRED` is admitted only for the top-left no-neighbour block (see
/// the general intra mode gate).
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
/// those proven bit-exact against the AVM/dav2d oracle, all with
/// `AngleDeltaY == 0`. The other directional modes, the IDIF edge synthesis for
/// non-cardinal angles, and non-zero angle deltas need their own verified oracle
/// fixtures and remain deferred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportedDirectionalLumaMode {
    /// AV2 `V_PRED` (§ 7.13.2.8 step 4, pAngle 90): a pure VERTICAL copy
    /// (`pred[i][j] = AboveRow[j]`, each row equals the § 7.13.2.1 above row).
    /// Reads only the above row — no corner, no left, no IDIF/upsample.
    Vertical,
    /// AV2 `H_PRED` (§ 7.13.2.8 step 5, pAngle 180): a pure HORIZONTAL copy
    /// (`pred[i][j] = LeftCol[i]`, each column equals the § 7.13.2.1 left
    /// column). Reads only the left column — no corner, no above, no IDIF.
    Horizontal,
    /// AV2 `D113_PRED` (§ 7.13.2.8 directional prediction at pAngle 113,
    /// `dx == Dr_Intra_Derivative[180 - 113] == Dr_Intra_Derivative[67] == 24`,
    /// `dy == Dr_Intra_Derivative[113 - 90] == Dr_Intra_Derivative[23] == 170`).
    /// Vertical-leaning (`pAngle` near 90): most projections take the above
    /// branch (`base >= -(1 + mrlIndex)`) and land on a nonzero `shift`, so the
    /// § 7.13.2.8 luma IDIF 4-tap `Dr_Interp_Filter` genuinely interpolates over
    /// the real reconstructed above row + corner.
    D113,
    /// AV2 `D135_PRED` (§ 7.13.2.8 directional prediction at pAngle 135). All
    /// projections have `shift == 0`, so the luma IDIF 4-tap reduces to a sample
    /// copy (bit-identical to the bilinear branch).
    D135,
    /// AV2 `D157_PRED` (§ 7.13.2.8 directional prediction at pAngle 157,
    /// `dx == Dr_Intra_Derivative[23] == 170`, `dy == Dr_Intra_Derivative[67] ==
    /// 24`). Its nonzero-shift projections genuinely interpolate via the
    /// § 7.13.2.8 luma IDIF 4-tap `Dr_Interp_Filter`.
    D157,
    /// AV2 `D45_PRED` (§ 7.13.2.8 directional prediction at pAngle 45, the
    /// ZONE-1 one-sided angle `pAngle < 90`, step 1, `dx ==
    /// Dr_Intra_Derivative[45] == 64`). Unlike the "middle" angles (which read
    /// `AboveRow[0..w)` / `LeftCol[0..h)`), the zone-1 projection reads the above
    /// row AND projects up-and-right into the ABOVE-RIGHT (`base = (i + 1 + j)`,
    /// up to `maxBaseX == w + h - 1`), reading the real reconstructed above-right
    /// samples. Every D45 projection has `shift == 0` (`(i + 1) * 64 >> 1 & 0x1F
    /// == 0`), so the § 7.13.2.8 luma IDIF 4-tap reduces to the sample copy
    /// `AboveRow[base]` (bit-identical to the bilinear branch) — but it still
    /// reads far into the real reconstructed above-right, the one-sided zone the
    /// middle angles never touch.
    D45,
    /// AV2 `D203_PRED` (§ 7.13.2.8 directional prediction at pAngle 203, the
    /// ZONE-3 one-sided angle `pAngle > 180`, step 3, `dy ==
    /// Dr_Intra_Derivative[270 - 203] == Dr_Intra_Derivative[67] == 24`). The
    /// symmetric mirror of D45: the projection reads the left column AND projects
    /// down-and-left into the BELOW-LEFT (`idx = (j + 1) * dy`,
    /// `base = (idx >> 6) + i`, up to `maxBaseY == w + h - 1`), reading the real
    /// reconstructed left column (and the clamped below-left in raster order).
    /// Unlike D45, D203's `dy == 24` makes most projections land on a nonzero
    /// `shift`, so the § 7.13.2.8 luma IDIF 4-tap `Dr_Interp_Filter` genuinely
    /// interpolates over the real reconstructed left column.
    D203,
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
    /// AV2 `D113_PRED` directional-follow chroma prediction (§ 7.13.2.8,
    /// pAngle 113, `AngleDeltaUV == 0`). Resolved when the decoded
    /// `uv_mode == 0` over a `D113_PRED` luma mode makes § 5.20.5.3
    /// `get_intra_uv_mode_set` return `YMode == D113_PRED` (the directional-follow
    /// branch, `AngleDeltaUV = AngleDeltaY == 0`). Chroma uses the
    /// `enableIdif == 0` bilinear branch (`enableIdif = plane == 0`, `0` for
    /// U/V), reading the real reconstructed § 7.13.2.1 above row / left column /
    /// corner; the spec-mandated chroma branch IS bilinear, so it is bit-exact.
    D113Follow,
    /// AV2 `D157_PRED` directional-follow chroma prediction (§ 7.13.2.8,
    /// pAngle 157, `AngleDeltaUV == 0`). Resolved when the decoded
    /// `uv_mode == 0` over a `D157_PRED` luma mode makes § 5.20.5.3
    /// `get_intra_uv_mode_set` return `YMode == D157_PRED` (the directional-follow
    /// branch, `AngleDeltaUV = AngleDeltaY == 0`). Chroma uses the
    /// `enableIdif == 0` bilinear branch (`enableIdif = plane == 0`, `0` for
    /// U/V), reading the real reconstructed § 7.13.2.1 left chroma column; over a
    /// flat real chroma edge the D157 bilinear projection is bit-exact.
    D157Follow,
    /// AV2 `V_PRED` directional-follow chroma (§ 7.13.2.8 step 4, pAngle 90,
    /// `AngleDeltaUV == 0`). Resolved when the decoded `uv_mode == 0` over a
    /// `V_PRED` luma mode makes § 5.20.5.3 `get_intra_uv_mode_set` return `YMode
    /// == V_PRED`; the spec sets `AngleDeltaUV = AngleDeltaY` (`0`). The cardinal
    /// copy of the § 7.13.2.1 above row needs no IDIF (chroma `enableIdif == 0`
    /// anyway), so it is bit-exact over a real reconstructed above row.
    VerticalFollow,
    /// AV2 `H_PRED` directional-follow chroma (§ 7.13.2.8 step 5, pAngle 180,
    /// `AngleDeltaUV == 0`): the cardinal copy of the § 7.13.2.1 left column.
    HorizontalFollow,
    /// AV2 `H_PRED` chroma (§ 7.13.2.8 step 5, pAngle 180) decoded NON-follow: the
    /// explicit `uv_mode` index (not the `uv_mode == 0` follow branch) resolves to
    /// `H_PRED` over a non-directional (e.g. DC) luma. Supported only at the
    /// no-neighbour top-left block, where § 7.13.2.1 makes `LeftCol[i]` the flat
    /// no-left fallback (`129` for 8-bit); the § 7.13.2.8 horizontal copy
    /// `pred[i][j] = LeftCol[i]` is then a flat plane, bit-exact against the
    /// AVM/dav2d oracle. Neighbour-having non-follow H_PRED chroma (a real
    /// reconstructed left column) is the `HorizontalFollow` path or remains
    /// deferred.
    Horizontal,
    /// AV2 `D45_PRED` directional-follow chroma (§ 7.13.2.8 ZONE-1 step 1,
    /// pAngle 45, `AngleDeltaUV == 0`). Resolved when the decoded `uv_mode == 0`
    /// over a `D45_PRED` luma mode makes § 5.20.5.3 `get_intra_uv_mode_set`
    /// return `YMode == D45_PRED`; the spec sets `AngleDeltaUV = AngleDeltaY`
    /// (`0`). Chroma uses the `enableIdif == 0` bilinear one-sided predictor
    /// (chroma `enableIdif = plane == 0` is `0`); for D45 every projection has
    /// `shift == 0`, so the bilinear branch is the sample copy `AboveRow[base]`,
    /// reading the real reconstructed chroma above row + above-right.
    D45Follow,
    /// AV2 `D203_PRED` directional-follow chroma (§ 7.13.2.8 ZONE-3 step 3,
    /// pAngle 203, `AngleDeltaUV == 0`). Resolved when the decoded `uv_mode == 0`
    /// over a `D203_PRED` luma mode makes § 5.20.5.3 `get_intra_uv_mode_set`
    /// return `YMode == D203_PRED`; the spec sets `AngleDeltaUV = AngleDeltaY`
    /// (`0`). Chroma uses the `enableIdif == 0` bilinear one-sided predictor
    /// (chroma `enableIdif = plane == 0` is `0`), the spec-mandated chroma branch,
    /// reading the real reconstructed chroma left column + below-left; over a flat
    /// real chroma left column the D203 bilinear projection is bit-exact.
    D203Follow,
}

/// AV2 § 9.2 canonical chroma mode values from `Default_Mode_List_Uv`:
/// `DC_PRED == 0`, `V_PRED == 1`, `H_PRED == 2`, `D135_PRED == 4`,
/// `D157_PRED == 6`, `SMOOTH_PRED == 9`.
const DC_PRED_VALUE: u8 = 0;
const V_PRED_VALUE: u8 = 1;
const H_PRED_VALUE: u8 = 2;
const D45_PRED_VALUE: u8 = 3;
const D203_PRED_VALUE: u8 = 7;
const D113_PRED_VALUE: u8 = 5;
const D135_PRED_VALUE: u8 = 4;
const D157_PRED_VALUE: u8 = 6;
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
/// so it maps to the same predictor, but is not yet AVM-verified, so it is left
/// to a future increment by requiring the directional-follow path.
pub(crate) fn supported_chroma_mode(
    y_mode: IntraYMode,
    uv_mode: u8,
) -> Option<SupportedChromaMode> {
    let uv_mode_value = get_intra_uv_mode_set(y_mode, uv_mode)?;
    match uv_mode_value {
        DC_PRED_VALUE => Some(SupportedChromaMode::Dc),
        SMOOTH_PRED_VALUE => Some(SupportedChromaMode::Smooth),
        D135_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D135Follow)
        }
        D113_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D113Follow)
        }
        D157_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D157Follow)
        }
        V_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::VerticalFollow)
        }
        H_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::HorizontalFollow)
        }
        H_PRED_VALUE => Some(SupportedChromaMode::Horizontal),
        D45_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D45Follow)
        }
        D203_PRED_VALUE if uv_mode == 0 && y_mode.is_directional() => {
            Some(SupportedChromaMode::D203Follow)
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
/// `y_mode_index` for the supported minimal subset (§ 5.20.5.5
/// `read_intra_y_mode`, `get_intra_y_mode_set`, and `Reordered_Y_Mode`).
///
/// Supported subset: `y_mode_set == 0` with a non-directional `y_mode_index`
/// (`0..NON_DIRECTIONAL_MODES_COUNT`). Then `modeIdx == y_mode_index` (the
/// `MODE_INDEX_COUNT - 1 == 7` escape never applies for these indices),
/// `get_intra_y_mode_set` passes `modeIdx` through unchanged (it is below
/// `NON_DIRECTIONAL_MODES_COUNT`), and `YMode == Reordered_Y_Mode[y_mode_index]`.
/// Returns `None` for inputs outside this subset.
// TODO(spec: DECODE-TILE-CDF-SELECTION-BOUNDARY): the directional-neighbour
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
// TODO(spec: DECODE-GENERAL-INTRA-ANGLE): the in-frame directional-neighbour
pub(crate) fn reconstruct_y_mode_offset_escape_top_left(
    y_mode_offset: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_offset >= MODE_OFFSET_COUNT {
        return None;
    }
    let mode_idx = usize::from(MODE_INDEX_COUNT - 1) + usize::from(y_mode_offset);
    resolve_y_mode_top_left(mode_idx)
}

/// Reconstructs the typed luma `YMode` and `AngleDeltaY` for a **direct
/// first-mode-set** `y_mode_index` that selects a directional mode, at the
/// top-left-equivalent block with **no in-frame directional joint-mode
/// neighbours** (`ctx == 0`, § 8.3.2).
///
/// This is the AV2 § 5.20.5.3 `read_intra_y_mode` `y_mode_set == 0` branch with
/// `NON_DIRECTIONAL_MODES_COUNT <= y_mode_index < MODE_INDEX_COUNT - 1` (the
/// `y_mode_offset` escape only fires at `y_mode_index == MODE_INDEX_COUNT - 1`).
/// Then `modeIdx == y_mode_index` directly (no `y_mode_offset` is read), and
/// `get_intra_y_mode_set(modeIdx)` reduces to the same `Default_Mode_List_Y`
/// scan as the escape because `ctx == 0` means no directional neighbour is
/// pre-selected (the § 5.20.5.3 selection loop has `count == 0`). The cardinal
/// `V_PRED` (`modeIdx == 5`, `Default_Mode_List_Y[0] == 17 == V_PRED + 4 * 4`,
/// `modeDelta == 22`, `AngleDeltaY == 0`) and `H_PRED` (`modeIdx == 6`,
/// `Default_Mode_List_Y[1] == 45`, `modeDelta == 50`, `AngleDeltaY == 0`) reach
/// this path; their `Mode_To_Angle` is 90 / 180 (§ 9.2).
///
/// Returns `None` for a `y_mode_index` below `NON_DIRECTIONAL_MODES_COUNT` (a
/// non-directional first-set index, handled by [`reconstruct_minimal_y_mode`]),
/// at or above `MODE_INDEX_COUNT - 1` (the escape, handled by
/// [`reconstruct_y_mode_offset_escape_top_left`]), or for any arithmetic that
/// escapes the table bounds.
// TODO(spec: DECODE-GENERAL-INTRA-ANGLE): only the `ctx == 0` (no
pub(crate) fn reconstruct_y_mode_first_set_directional_top_left(
    y_mode_index: u8,
) -> Option<YModeEscapeResult> {
    if y_mode_index < (NON_DIRECTIONAL_MODES_COUNT as u8) || y_mode_index >= MODE_INDEX_COUNT - 1 {
        return None;
    }
    resolve_y_mode_top_left(usize::from(y_mode_index))
}

/// Reconstructs the typed luma `YMode` and `AngleDeltaY` for the AV2
/// § 5.20.5.5 `y_mode_set != 0` branch, restricted to blocks with no in-frame
/// directional joint-mode neighbours (`ctx == 0`, § 8.3.2).
///
/// The syntax reads `y_second_mode L(4)` and computes
/// `modeIdx = FIRST_MODE_COUNT + (y_mode_set - 1) * SECOND_MODE_COUNT +
/// y_second_mode`. With no directional neighbours selected by
/// `get_intra_y_mode_set`, the same top-left-equivalent `Default_Mode_List_Y`
/// scan used by the direct first-set/offset paths resolves `modeDelta`, typed
/// `YMode`, and `AngleDeltaY`.
///
/// Returns `None` for `y_mode_set == 0`, a `y_second_mode` outside
/// `0..SECOND_MODE_COUNT`, or arithmetic/table overflow.
// TODO(spec: DECODE-GENERAL-INTRA-ANGLE): only the `ctx == 0` (no
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

/// Resolves a § 5.20.5.5 `modeIdx` through the full directional-neighbour
/// `get_intra_y_mode_set` reorder for a block with already decoded left/above
/// `IntraJointMode` neighbours.
///
/// `neighbour_joint_modes[0]` is `get_joint_mode(0)` (left) and
/// `neighbour_joint_modes[1]` is `get_joint_mode(1)` (above). `block_n4w` /
/// `block_n4h` are `Num_4x4_Blocks_Wide/High[MiSize]`; they select the
/// `MiSize >= BLOCK_8X8` neighbour branch and the
/// `Block_Width * Block_Height > 64` ±1..4 expansion branch.
pub(crate) fn reconstruct_y_mode_with_neighbours(
    mode_idx: usize,
    neighbour_joint_modes: [u8; 2],
    block_n4w: usize,
    block_n4h: usize,
) -> Option<YModeEscapeResult> {
    let mode_delta = get_intra_y_mode_set(mode_idx, neighbour_joint_modes, block_n4w, block_n4h)?;
    resolve_y_mode_delta(mode_delta)
}

/// Resolves a § 5.20.5.3 `modeIdx` to the typed `YMode`, `AngleDeltaY`, and
/// stored `IntraJointMode` (`modeDelta`) for the top-left no-directional-neighbour
/// (`ctx == 0`) case, shared by the `y_mode_offset` escape and the direct
/// first-set directional path. Returns `None` for any arithmetic that escapes the
/// `Default_Mode_List_Y` / `Reordered_Y_Mode` table bounds.
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

/// AV2 § 5.20.5.3 `get_intra_y_mode_set(modeIdx)` for the top-left block with no
/// in-frame directional joint-mode neighbours. Returns `modeDelta`.
///
/// When `modeIdx < NON_DIRECTIONAL_MODES_COUNT` the spec returns `modeIdx`
/// directly. Otherwise, with both neighbours out of frame, no directional mode is
/// pre-selected, so the result is the `(modeIdx - NON_DIRECTIONAL_MODES_COUNT)`-th
/// entry of `Default_Mode_List_Y`, biased by `NON_DIRECTIONAL_MODES_COUNT`.
/// Returns `None` if `modeIdx - NON_DIRECTIONAL_MODES_COUNT` exceeds the table.
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

/// AV2 § 5.20.5.3 `get_intra_y_mode_set` neighbour-reorder gate
/// (`05-syntax-structures.md` line 11128: `if ( MiSize >= BLOCK_8X8 )`).
///
/// In the § 5.20.6 `BLOCK_SIZE` enum, `BLOCK_8X8` is the fourth value, so
/// `MiSize >= BLOCK_8X8` excludes exactly `{BLOCK_4X4, BLOCK_4X8, BLOCK_8X4}` —
/// the three sub-8x8 leaves with `Num_4x4_Blocks_Wide * Num_4x4_Blocks_High`
/// (`block_n4w * block_n4h`) of `1`, `2`, and `2`. Every other `MiSize`
/// (including `BLOCK_4X16` / `BLOCK_16X4`, which have a single-MI dimension but
/// are NOT `< BLOCK_8X8`) has area `> 2` and runs the directional-neighbour
/// reorder.
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
    fn y_mode_offset_escape_reconstructs_d135_for_the_hedge_fixture() {
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

    #[test]
    fn supported_directional_admits_middle_d45_and_d203_but_rejects_d67() {
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
        assert_eq!(IntraYMode(8).supported_directional(), None);
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d113_for_uv_mode_zero() {
        let d113 = IntraYMode(IntraYMode::D113_PRED);
        assert_eq!(get_intra_uv_mode_set(d113, 0), Some(IntraYMode::D113_PRED));
        assert_eq!(
            supported_chroma_mode(d113, 0),
            Some(SupportedChromaMode::D113Follow)
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
        let d157 = IntraYMode(IntraYMode::D157_PRED);
        assert_eq!(get_intra_uv_mode_set(d157, 0), Some(IntraYMode::D157_PRED));
        assert_eq!(
            supported_chroma_mode(d157, 0),
            Some(SupportedChromaMode::D157Follow)
        );
    }

    #[test]
    fn supported_chroma_mode_directional_follow_resolves_d203_for_uv_mode_zero() {
        let d203 = IntraYMode(IntraYMode::D203_PRED);
        assert_eq!(get_intra_uv_mode_set(d203, 0), Some(IntraYMode::D203_PRED));
        assert_eq!(
            supported_chroma_mode(d203, 0),
            Some(SupportedChromaMode::D203Follow)
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
        let d135 = IntraYMode(IntraYMode::D135_PRED);
        assert_eq!(get_intra_uv_mode_set(d135, 0), Some(IntraYMode::D135_PRED));
        assert_eq!(
            supported_chroma_mode(d135, 0),
            Some(SupportedChromaMode::D135Follow)
        );
    }

    #[test]
    fn supported_chroma_mode_non_follow_d135_is_deferred() {
        let dc = IntraYMode::DC_PRED;
        assert_eq!(get_intra_uv_mode_set(dc, 8), Some(IntraYMode::D135_PRED));
        assert_eq!(supported_chroma_mode(dc, 8), None);
    }

    #[test]
    fn supported_chroma_mode_non_directional_luma_passes_list_through() {
        let dc = IntraYMode::DC_PRED;
        assert_eq!(supported_chroma_mode(dc, 0), Some(SupportedChromaMode::Dc));
        assert_eq!(
            supported_chroma_mode(dc, 1),
            Some(SupportedChromaMode::Smooth)
        );
        assert_eq!(supported_chroma_mode(dc, 5), None);
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
        assert_eq!(get_intra_uv_mode_set(dc, 6), Some(H_PRED_VALUE));
        assert_eq!(
            supported_chroma_mode(dc, 6),
            Some(SupportedChromaMode::Horizontal)
        );
    }
}
