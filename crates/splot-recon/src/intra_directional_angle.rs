// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Directional-angle intra prediction primitives.
//!
//! Feature tracking: `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`,
//! `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`.

use crate::intra_dc_math::{clip1, round2, validate_output_shape, validate_sample_type};
use crate::{BitDepth, IntraRectBlockSize, ReconError, ReconSample, Result};

const ANGLE_D45: u16 = 45;
const ANGLE_D67: u16 = 67;
const ANGLE_D113: u16 = 113;
const ANGLE_D135: u16 = 135;
const ANGLE_D157: u16 = 157;
const ANGLE_D203: u16 = 203;
const INTERP_SCALE: u16 = 32;

/// AV2 v1.0.0 §9.2 `Dr_Intra_Derivative[90]` conversion table, transcribed
/// VERBATIM from the committed spec mirror
/// `docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md` (the
/// `Dr_Intra_Derivative[ 90 ]` array). `Dr_Intra_Derivative[a]` is the projection
/// derivative for a directional angle whose §9.2 index is `a` (the angle in
/// roughly `0.9°` steps): the §7.13.2.8 zone-1 step reads
/// `dx = Dr_Intra_Derivative[pAngle]` (`pAngle < 90`), the zone-3 step reads
/// `dy = Dr_Intra_Derivative[270 - pAngle]` (`pAngle > 180`), and the zone-2
/// middle step reads `Dr_Intra_Derivative[180 - pAngle]` /
/// `Dr_Intra_Derivative[pAngle - 90]`. Several entries (indices 0, 11, 12, 34,
/// 56, 78, 79 — the spec's starred values) are unused by any reachable angle.
#[rustfmt::skip]
pub(super) const DR_INTRA_DERIVATIVE: [u16; 90] = [
    0,    4096, 2048,
    1365, 1024, 819,
    682,  585,  512,
    455,  409,  409,  409, 372,
    341,  292,  273,
    256,  227,  215,
    204,  186,  178,
    170,  157,  151,
    146,  136,  132,
    128,  117,  110,
    107,  99,   97,   97,
    93,   87,   83,
    81,   77,   74,
    73,   69,   66,
    64,   62,   59,
    56,   55,   53,
    50,   49,   47,
    44,   42,   42,   41,
    38,   37,   35,
    32,   31,   30,
    28,   27,   26,
    24,   23,   22,
    20,   19,   18,
    16,   15,   14,
    12,   11,   10,   10,  10,
    9,    8,    7,
    6,    5,    4,
    3,    2,    1,
];

/// AV2 §7.13.2.8 zone boundary: a `pAngle` strictly below `ZONE_1_MAX` reads the
/// above row (step 1, zone-1), strictly above `ZONE_3_MIN` reads the left column
/// (step 3, zone-3). `90`/`180` are the cardinals (steps 4/5) and `90 < pAngle <
/// 180` is the zone-2 middle band (step 2).
pub(super) const ZONE_1_MAX: u16 = 90;
pub(super) const ZONE_3_MIN: u16 = 180;
/// AV2 §7.13.2.8 zone-3 derivative index base: `dy = Dr_Intra_Derivative[270 -
/// pAngle]`.
pub(super) const ZONE_3_INDEX_BASE: u16 = 270;

/// Number of `shift` rows in the §7.13.2.8 IDIF filter table.
const DR_INTERP_FILTER_SHIFTS: usize = 32;
/// Number of taps per §7.13.2.8 IDIF filter row.
const DR_INTERP_FILTER_TAPS: usize = 4;
/// `Round2` shift applied to the §7.13.2.8 IDIF 4-tap sum (`pred = Clip1(Round2(s, 7))`).
const DR_INTERP_FILTER_ROUND: u8 = 7;

/// AV2 v1.0.0 §7.13.2.8 `Dr_Interp_Filter[32][4]` interpolation filter taps,
/// used when `enableIdif == 1` (luma): `s = Σ(t=0..3) Dr_Interp_Filter[shift][t]
/// * Edge[base + t - 1]; pred = Clip1(Round2(s, 7))`. Copied verbatim from the
/// committed spec mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8`. Row 0
/// (`shift == 0`) is `{0, 128, 0, 0}`, so the 4-tap reduces to
/// `Clip1(Round2(128 * Edge[base], 7)) == Edge[base]` — a sample copy
/// identical to the `enableIdif == 0` bilinear branch at `shift == 0`.
#[rustfmt::skip]
const DR_INTERP_FILTER: [[i32; DR_INTERP_FILTER_TAPS]; DR_INTERP_FILTER_SHIFTS] = [
    [0, 128, 0, 0],     [-2, 127, 4, -1],   [-3, 125, 8, -2],
    [-5, 123, 13, -3],  [-6, 121, 17, -4],  [-7, 118, 22, -5],
    [-9, 116, 27, -6],  [-9, 112, 32, -7],  [-10, 109, 37, -8],
    [-11, 106, 41, -8], [-11, 102, 46, -9], [-12, 98, 52, -10],
    [-12, 94, 56, -10], [-12, 90, 61, -11], [-12, 85, 66, -11],
    [-12, 81, 71, -12], [-12, 76, 76, -12], [-12, 71, 81, -12],
    [-11, 66, 85, -12], [-11, 61, 90, -12], [-10, 56, 94, -12],
    [-10, 52, 98, -12], [-9, 46, 102, -11], [-8, 41, 106, -11],
    [-8, 37, 109, -10], [-7, 32, 112, -9],  [-6, 27, 116, -9],
    [-5, 22, 118, -7],  [-4, 17, 121, -6],  [-3, 13, 123, -5],
    [-2, 8, 125, -3],   [-1, 4, 127, -2],
];

/// Supported one-sided directional-angle pAngle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraDirectionalAngle {
    p_angle: u16,
}

impl IntraDirectionalAngle {
    /// AV2 `D45_PRED`, `Mode_To_Angle == 45`.
    pub const D45: Self = Self { p_angle: ANGLE_D45 };

    /// AV2 `D67_PRED`, `Mode_To_Angle == 67`.
    pub const D67: Self = Self { p_angle: ANGLE_D67 };

    /// AV2 `D203_PRED`, `Mode_To_Angle == 203`.
    pub const D203: Self = Self {
        p_angle: ANGLE_D203,
    };

    /// Creates a supported one-sided directional angle from an AV2 pAngle.
    ///
    /// Admits ANY pAngle in the §7.13.2.8 one-sided ranges that has a §9.2
    /// derivative entry: zone-1 (`0 < pAngle < 90`, reads `AboveRow` with
    /// `dx = Dr_Intra_Derivative[pAngle]`) and zone-3 (`180 < pAngle < 270`,
    /// reads `LeftCol` with `dy = Dr_Intra_Derivative[270 - pAngle]`). This is a
    /// RANGE check over the §9.2 table, not a per-angle whitelist; the
    /// `shift`/`base`/`maxBase` projection in the predictor is angle-agnostic.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedIntraDirectionalAngle`] for a `pAngle`
    /// outside the one-sided ranges (the cardinals `90`/`180` and the zone-2
    /// middle band `90 < pAngle < 180` use dedicated primitives), or for a
    /// `pAngle` whose §9.2 derivative index falls outside the table.
    pub const fn try_from_p_angle(p_angle: u16) -> Result<Self> {
        if Self::derivative_for(p_angle).is_some() {
            Ok(Self { p_angle })
        } else {
            Err(ReconError::UnsupportedIntraDirectionalAngle { p_angle })
        }
    }

    /// Returns the §7.13.2.8 one-sided projection derivative for `p_angle`, or
    /// `None` when `p_angle` is not a one-sided angle with a §9.2 entry. Zone-1
    /// (`0 < pAngle < 90`) uses `Dr_Intra_Derivative[pAngle]`; zone-3
    /// (`180 < pAngle < 270`) uses `Dr_Intra_Derivative[270 - pAngle]`.
    const fn derivative_for(p_angle: u16) -> Option<u16> {
        if p_angle > 0 && p_angle < ZONE_1_MAX {
            return Some(DR_INTRA_DERIVATIVE[p_angle as usize]);
        }
        if p_angle > ZONE_3_MIN && p_angle < ZONE_3_INDEX_BASE {
            let index = (ZONE_3_INDEX_BASE - p_angle) as usize;
            return Some(DR_INTRA_DERIVATIVE[index]);
        }
        None
    }

    /// Returns the AV2 pAngle value.
    pub const fn p_angle(self) -> u16 {
        self.p_angle
    }

    /// Returns the required prepared edge for this pAngle: zone-1 (`pAngle < 90`)
    /// reads the above row; zone-3 (`pAngle > 180`) reads the left column.
    pub const fn required_edge(self) -> IntraDirectionalAngleEdge {
        if self.p_angle < ZONE_1_MAX {
            IntraDirectionalAngleEdge::Above
        } else {
            IntraDirectionalAngleEdge::Left
        }
    }

    /// Returns the furthest logical edge index this one-sided §7.13.2.8 IDIF
    /// projection reads for a `size` block at `mrl_index`, i.e. the largest
    /// `base + 2` the 4-tap reads while `base <= maxBase`, capped at `maxBase`
    /// (`= w + h - 1 + (mrlIndex << 1)`). Beyond the block's own in-edge span
    /// (`side - 1`) this is how far into the above-right (zone-1) / below-left
    /// (zone-3) the prediction reaches. A caller verifies those neighbour samples
    /// are reconstructed before admitting the block. `mrl_index == 0` is the
    /// immediate reference line.
    ///
    /// # Errors
    /// Returns [`ReconError::ArithmeticOverflow`] when the block dimensions
    /// overflow the index arithmetic.
    pub fn max_one_sided_edge_read_index(
        self,
        size: IntraRectBlockSize,
        mrl_index: usize,
    ) -> Result<usize> {
        let derivative = one_sided_idif_derivative(self);
        let branch = self.branch();
        let max_base = one_sided_max_base(size, mrl_index)?;
        let mut furthest: i64 = 0;
        for row in 0..size.height() {
            for column in 0..size.width() {
                let reference =
                    one_sided_idif_reference(branch, row, column, derivative, mrl_index)?;
                let read = if reference.base <= max_base {
                    reference
                        .base
                        .checked_add(2)
                        .filter(|&v| v <= max_base)
                        .unwrap_or(max_base)
                } else {
                    max_base
                };
                if read > furthest {
                    furthest = read;
                }
            }
        }
        usize::try_from(furthest).map_err(|_| ReconError::ArithmeticOverflow {
            context: "one-sided directional angle furthest edge read index",
        })
    }

    const fn branch(self) -> DirectionalAngleBranch {
        let derivative = match Self::derivative_for(self.p_angle) {
            Some(derivative) => derivative,
            None => 0,
        };
        if self.p_angle < ZONE_1_MAX {
            DirectionalAngleBranch::Above { derivative }
        } else {
            DirectionalAngleBranch::Left { derivative }
        }
    }
}

/// Edge identifier for one-sided directional-angle prediction inputs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntraDirectionalAngleEdge {
    /// Left edge samples `LeftCol[0..w+h)`.
    Left,
    /// Above edge samples `AboveRow[0..w+h)`.
    Above,
}

impl IntraDirectionalAngleEdge {
    /// Returns a stable human-readable edge name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Above => "above",
        }
    }
}

/// Caller-provided prepared edge samples for one-sided AV2 §7.13.2.8 prediction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDirectionalAngleEdges<'a, T: ReconSample> {
    left: Option<&'a [T]>,
    above: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraDirectionalAngleEdges<'a, T> {
    /// Creates a prepared edge set from optional left and above edges.
    ///
    /// Availability and fallback preparation remain outside this type and are
    /// owned by the broader AV2 §7.13.2.1 intra process.
    pub const fn new(left: Option<&'a [T]>, above: Option<&'a [T]>) -> Self {
        Self { left, above }
    }

    /// Creates an edge set with only `LeftCol[0..w+h)` available.
    pub const fn left(left: &'a [T]) -> Self {
        Self::new(Some(left), None)
    }

    /// Creates an edge set with only `AboveRow[0..w+h)` available.
    pub const fn above(above: &'a [T]) -> Self {
        Self::new(None, Some(above))
    }

    /// Creates an edge set with both left and above samples available.
    pub const fn both(left: &'a [T], above: &'a [T]) -> Self {
        Self::new(Some(left), Some(above))
    }

    /// Returns prepared left edge samples when available.
    pub const fn left_samples(self) -> Option<&'a [T]> {
        self.left
    }

    /// Returns prepared above edge samples when available.
    pub const fn above_samples(self) -> Option<&'a [T]> {
        self.above
    }
}

/// Supported middle directional-angle pAngle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntraMiddleDirectionalAngle {
    p_angle: u16,
}

impl IntraMiddleDirectionalAngle {
    /// AV2 `D113_PRED`, `Mode_To_Angle == 113`.
    pub const D113: Self = Self {
        p_angle: ANGLE_D113,
    };

    /// AV2 `D135_PRED`, `Mode_To_Angle == 135`.
    pub const D135: Self = Self {
        p_angle: ANGLE_D135,
    };

    /// AV2 `D157_PRED`, `Mode_To_Angle == 157`.
    pub const D157: Self = Self {
        p_angle: ANGLE_D157,
    };

    /// Creates a supported middle directional angle from an AV2 pAngle.
    ///
    /// Admits ANY pAngle in the §7.13.2.8 zone-2 middle band (`90 < pAngle <
    /// 180`); the §9.2 derivatives are `dx = Dr_Intra_Derivative[180 - pAngle]`
    /// and `dy = Dr_Intra_Derivative[pAngle - 90]` (AVM `av2_get_dx` / `av2_get_dy`
    /// for `90 < angle < 180`), so every in-band pAngle has a valid pair (both
    /// indices land in `1..=89`). The named modes `D113` / `D135` / `D157` are the
    /// `AngleDeltaY == 0` cases; non-zero `AngleDeltaY` and the §5.20.7.29
    /// wide-angle remap produce the other in-band pAngles.
    ///
    /// # Errors
    /// Returns [`ReconError::UnsupportedIntraMiddleDirectionalAngle`] for a pAngle
    /// outside the open zone-2 band (`pAngle <= 90` or `pAngle >= 180`).
    pub const fn try_from_p_angle(p_angle: u16) -> Result<Self> {
        if p_angle > ZONE_1_MAX && p_angle < ZONE_3_MIN {
            Ok(Self { p_angle })
        } else {
            Err(ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle })
        }
    }

    /// Returns the AV2 pAngle value.
    pub const fn p_angle(self) -> u16 {
        self.p_angle
    }

    fn branch(self) -> Result<MiddleDirectionalAngleBranch> {
        if self.p_angle <= ZONE_1_MAX || self.p_angle >= ZONE_3_MIN {
            return Err(ReconError::UnsupportedIntraMiddleDirectionalAngle {
                p_angle: self.p_angle,
            });
        }
        let dx = DR_INTRA_DERIVATIVE[usize::from(ZONE_3_MIN - self.p_angle)];
        let dy = DR_INTRA_DERIVATIVE[usize::from(self.p_angle - ZONE_1_MAX)];
        Ok(MiddleDirectionalAngleBranch { dx, dy })
    }
}

/// Caller-provided prepared edge samples for middle AV2 §7.13.2.8 prediction.
///
/// Each supplied slice starts with the logical `-1` sample: `slice[0]` maps to
/// `AboveRow[-1]` or `LeftCol[-1]`, and `slice[index + 1]` maps to logical
/// index `index`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraMiddleDirectionalAngleEdges<'a, T: ReconSample> {
    left_with_minus_one: Option<&'a [T]>,
    above_with_minus_one: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraMiddleDirectionalAngleEdges<'a, T> {
    /// Creates a prepared middle-edge set from optional left and above edges.
    ///
    /// Availability, fallback preparation, IDIF extension, and MRL remain
    /// outside this type and are owned by the broader AV2 §7.13.2.1 intra
    /// process.
    pub const fn new(
        left_with_minus_one: Option<&'a [T]>,
        above_with_minus_one: Option<&'a [T]>,
    ) -> Self {
        Self {
            left_with_minus_one,
            above_with_minus_one,
        }
    }

    /// Creates an edge set with both `LeftCol[-1..h)` and `AboveRow[-1..w)`.
    pub const fn both(left_with_minus_one: &'a [T], above_with_minus_one: &'a [T]) -> Self {
        Self::new(Some(left_with_minus_one), Some(above_with_minus_one))
    }

    /// Returns prepared left edge samples when available.
    pub const fn left_with_minus_one(self) -> Option<&'a [T]> {
        self.left_with_minus_one
    }

    /// Returns prepared above edge samples when available.
    pub const fn above_with_minus_one(self) -> Option<&'a [T]> {
        self.above_with_minus_one
    }
}

/// Caller-provided prepared edge samples for the luma IDIF middle
/// AV2 §7.13.2.8 prediction (`enableIdif == 1`).
///
/// The §7.13.2.8 IDIF 4-tap reads `Edge[base + t - 1]` for `t = 0..3`, i.e.
/// `Edge[base - 1 ..= base + 2]`, so the prepared edges span the logical range
/// `-2 ..= side + 1`: the spec extends `LeftCol[minBase - 1] = LeftCol[minBase]`
/// (with `minBase = -1` for `mrlIndex == 0`) and, for `90 < pAngle < 180`,
/// `LeftCol[h] = LeftCol[h + 1] = LeftCol[h - 1]` (and the analogous
/// `AboveRow[w] = AboveRow[w + 1] = AboveRow[w - 1]`). Each supplied slice maps
/// `slice[0]` to logical `-2`, `slice[1]` to the logical `-1` corner, and
/// `slice[index + 2]` to logical index `index`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraMiddleDirectionalAngleIdifEdges<'a, T: ReconSample> {
    left_idif: Option<&'a [T]>,
    above_idif: Option<&'a [T]>,
}

impl<'a, T: ReconSample> IntraMiddleDirectionalAngleIdifEdges<'a, T> {
    /// Creates an IDIF middle-edge set from optional left and above edges.
    ///
    /// Each slice spans the logical range `-2 ..= side + 1` (length `side + 4`):
    /// `slice[0]` is logical `-2`, `slice[1]` the `-1` corner, `slice[index + 2]`
    /// logical index `index`. AV2 §7.13.2.1 edge availability, fallback
    /// preparation, the §7.13.2.8 spec edge extension, MRL, and angle deltas
    /// remain outside this type.
    pub const fn new(left_idif: Option<&'a [T]>, above_idif: Option<&'a [T]>) -> Self {
        Self {
            left_idif,
            above_idif,
        }
    }

    /// Creates an IDIF edge set with both logical `LeftCol[-2..=h+1]` and
    /// `AboveRow[-2..=w+1]` available.
    pub const fn both(left_idif: &'a [T], above_idif: &'a [T]) -> Self {
        Self::new(Some(left_idif), Some(above_idif))
    }

    /// Returns prepared IDIF left edge samples when available.
    pub const fn left_idif(self) -> Option<&'a [T]> {
        self.left_idif
    }

    /// Returns prepared IDIF above edge samples when available.
    pub const fn above_idif(self) -> Option<&'a [T]> {
        self.above_idif
    }
}

/// Caller-provided prepared edge for the luma IDIF one-sided AV2 §7.13.2.8
/// prediction: the zone-1 above edge (`pAngle < 90`, step 1) or the symmetric
/// zone-3 left edge (`pAngle > 180`, step 3), both with `enableIdif == 1`.
///
/// The zone-1 step reads `AboveRow[base + t - 1]` for `t = 0..3` with
/// `base = (idx >> 6) + j` projecting up-and-right into the above-right; the
/// symmetric zone-3 step reads `LeftCol[base + t - 1]` with
/// `base = (idx >> 6) + i` projecting down-and-left into the below-left. Both
/// index far past the in-block edge: up to `base == maxBase` (with
/// `maxBase = w + h - 1 + (mrlIndex << 1)`), reading `Edge[maxBase + 2]`.
/// The §7.13.2.8 edge extension fills `Edge[maxBase + 1] = Edge[maxBase + 2]
/// = Edge[maxBase]` and `Edge[minBase - 1] = Edge[minBase]`
/// (`minBase = -(1 + mrlIndex) == -1` for `mrlIndex == 0`). The prepared slice
/// therefore spans the logical range `-2 ..= w + h + 1` (length `w + h + 4` for
/// `mrlIndex == 0`): `slice[0]` is logical `-2`, `slice[1]` the `-1` corner, and
/// `slice[index + 2]` logical index `index`.
///
/// AV2 §7.13.2.1 edge availability and fallback preparation (including reading
/// the real reconstructed above-right `CurrFrame[plane][y - 1][Min(aboveLimit,
/// x + i)]` via §5.20.7.25 `count_top_right_avail`, or the below-left
/// `CurrFrame[plane][Min(leftLimit, y + i)][x - 1]` via §5.20.7.25
/// `count_bottom_left_avail`, over §5.20.2.3 `BlockDecoded`), the §7.13.2.8 spec
/// edge extension, MRL, and angle deltas remain outside this type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntraDirectionalAngleIdifEdges<'a, T: ReconSample> {
    edge: &'a [T],
    direction: IntraDirectionalAngleEdge,
}

impl<'a, T: ReconSample> IntraDirectionalAngleIdifEdges<'a, T> {
    /// Creates a zone-1 IDIF above-edge set from the prepared above samples.
    ///
    /// `above_idif` spans the logical range `-2 ..= w + h + 1` (length
    /// `w + h + 4` for `mrlIndex == 0`): `slice[0]` is logical `-2`, `slice[1]`
    /// the `-1` corner, `slice[index + 2]` logical index `index`.
    pub const fn above(above_idif: &'a [T]) -> Self {
        Self {
            edge: above_idif,
            direction: IntraDirectionalAngleEdge::Above,
        }
    }

    /// Creates a zone-3 IDIF left-edge set from the prepared left samples.
    ///
    /// `left_idif` spans the logical range `-2 ..= w + h + 1` (length
    /// `w + h + 4` for `mrlIndex == 0`): `slice[0]` is logical `-2`, `slice[1]`
    /// the `-1` corner, `slice[index + 2]` logical index `index`.
    pub const fn left(left_idif: &'a [T]) -> Self {
        Self {
            edge: left_idif,
            direction: IntraDirectionalAngleEdge::Left,
        }
    }

    /// Returns the prepared zone-1 IDIF above edge samples (the zone-1
    /// constructor's edge); the zone-3 left edge is returned by
    /// [`IntraDirectionalAngleIdifEdges::edge_samples`].
    pub const fn above_idif(self) -> &'a [T] {
        self.edge
    }

    /// Returns the prepared IDIF edge samples (above or left, per the
    /// constructor used).
    pub const fn edge_samples(self) -> &'a [T] {
        self.edge
    }

    /// Returns which prepared edge (above for zone-1, left for zone-3) this set
    /// carries.
    pub const fn direction(self) -> IntraDirectionalAngleEdge {
        self.direction
    }
}

/// Writes a supported one-sided AV2 §7.13.2.8 directional prediction into caller storage.
///
/// This primitive intentionally covers only chroma/no-IDIF/no-MRL one-sided
/// pAngles `45`, `67`, and `203` over already-prepared edges. For pAngles `45`
/// and `67`, callers provide `AboveRow[0..w+h)`; for pAngle `203`, callers
/// provide `LeftCol[0..w+h)`. `output` points at the top-left destination sample
/// and `stride_samples` is the distance between adjacent output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// missing or wrong-length prepared edges, out-of-range edge samples, a
/// too-small stride, a too-small output buffer, or checked arithmetic overflow.
pub fn predict_intra_directional_angle_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let context = validate_inputs(bit_depth, size, angle, edges, output.len(), stride_samples)?;
    write_prediction(bit_depth, size, angle, context.edge, output, stride_samples)
}

/// Writes a supported one-sided directional prediction from a raw AV2 pAngle.
///
/// # Errors
/// In addition to [`predict_intra_directional_angle_rect_into`] errors, returns
/// [`ReconError::UnsupportedIntraDirectionalAngle`] before output mutation when
/// `p_angle` is not `45`, `67`, or `203`.
pub fn predict_intra_directional_angle_rect_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_directional_angle_rect_into(bit_depth, size, angle, edges, output, stride_samples)
}

/// Writes a supported middle AV2 §7.13.2.8 directional prediction into caller storage.
///
/// This primitive intentionally covers only chroma/no-IDIF/no-MRL middle
/// pAngles `113`, `135`, and `157` over already-prepared logical edges.
/// Callers provide `AboveRow[-1..w)` and `LeftCol[-1..h)`, where slice index
/// zero stores the logical `-1` sample. `output` points at the top-left
/// destination sample and `stride_samples` is the distance between adjacent
/// output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// unsupported pAngles, missing or wrong-length prepared edges, out-of-range
/// edge samples, a too-small stride, a too-small output buffer, or checked
/// arithmetic overflow.
pub fn predict_intra_middle_directional_angle_rect_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let context =
        validate_middle_inputs(bit_depth, size, angle, edges, output.len(), stride_samples)?;
    write_middle_prediction(
        size,
        angle,
        context.left,
        context.above,
        output,
        stride_samples,
    )
}

/// Writes a supported middle directional prediction from a raw AV2 pAngle.
///
/// # Errors
/// In addition to [`predict_intra_middle_directional_angle_rect_into`] errors,
/// returns [`ReconError::UnsupportedIntraMiddleDirectionalAngle`] before output
/// mutation when `p_angle` is not `113`, `135`, or `157`.
pub fn predict_intra_middle_directional_angle_rect_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraMiddleDirectionalAngleEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_middle_directional_angle_rect_into(
        bit_depth,
        size,
        angle,
        edges,
        output,
        stride_samples,
    )
}

/// Writes a supported luma IDIF middle AV2 §7.13.2.8 directional prediction
/// (`enableIdif == 1`) into caller storage.
///
/// This is the luma counterpart of
/// [`predict_intra_middle_directional_angle_rect_into`]: it applies the
/// §7.13.2.8 4-tap interpolation filter `Dr_Interp_Filter` instead of the
/// chroma `enableIdif == 0` bilinear branch. For each predicted sample the
/// projected `base`/`shift` are derived exactly as in the bilinear path, then
/// `s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * Edge[base + t - 1]` and
/// `pred[i][j] = Clip1(Round2(s, 7))`. At `shift == 0` the filter row is
/// `{0, 128, 0, 0}`, so the result reduces to `Edge[base]` — bit-identical to
/// the bilinear branch (e.g. every pAngle 135 projection has `shift == 0`).
///
/// Callers provide the wider IDIF edges `AboveRow[-2..=w+1]` and
/// `LeftCol[-2..=h+1]` (length `side + 4`); slice index zero is the logical
/// `-2` sample (the §7.13.2.8 `LeftCol[minBase - 1]` / spec edge extension).
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows. This primitive covers only the
/// no-MRL middle pAngles `113`, `135`, and `157` over already-prepared edges.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// unsupported pAngles, missing or wrong-length prepared edges, out-of-range
/// edge samples, a too-small stride, a too-small output buffer, or checked
/// arithmetic overflow.
pub fn predict_intra_middle_directional_angle_rect_idif_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleIdifEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let context =
        validate_middle_idif_inputs(bit_depth, size, angle, edges, output.len(), stride_samples)?;
    write_middle_idif_prediction(
        bit_depth,
        size,
        angle,
        context.left,
        context.above,
        output,
        stride_samples,
    )
}

/// Writes a supported luma IDIF middle directional prediction from a raw AV2
/// pAngle.
///
/// # Errors
/// In addition to [`predict_intra_middle_directional_angle_rect_idif_into`]
/// errors, returns [`ReconError::UnsupportedIntraMiddleDirectionalAngle`] before
/// output mutation when `p_angle` is not `113`, `135`, or `157`.
pub fn predict_intra_middle_directional_angle_rect_idif_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraMiddleDirectionalAngleIdifEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_middle_directional_angle_rect_idif_into(
        bit_depth,
        size,
        angle,
        edges,
        output,
        stride_samples,
    )
}

/// Writes a supported luma IDIF one-sided AV2 §7.13.2.8 directional prediction
/// into caller storage: the zone-1 above-reading angle (`pAngle < 90`, step 1)
/// or the symmetric zone-3 left-reading angle (`pAngle > 180`, step 3), both
/// with `enableIdif == 1`.
///
/// This is the luma counterpart of the bilinear one-sided
/// [`predict_intra_directional_angle_rect_into`]. For the ABOVE-reading zone-1
/// angle (D45/D67) it implements AV2 §7.13.2.8 step 1: for each predicted sample
/// `dx = Dr_Intra_Derivative[pAngle]`, `idx = (i + 1 + mrlIndex) * dx`,
/// `base = (idx >> 6) + j`, `shift = (idx >> 1) & 0x1F`. For the LEFT-reading
/// zone-3 angle (D203) it implements step 3 (the symmetric mirror):
/// `dy = Dr_Intra_Derivative[270 - pAngle]`, `idx = (j + 1 + mrlIndex) * dy`,
/// `base = (idx >> 6) + i`, `shift = (idx >> 1) & 0x1F`. In both cases
/// `maxBase = w + h - 1 + (mrlIndex << 1)`; when `base < maxBase + 1` the 4-tap
/// IDIF interpolates `s = Σ(t=0..3) Dr_Interp_Filter[shift][t] * Edge[base + t -
/// 1]`, `pred = Clip1(Round2(s, 7))`; otherwise `pred = Edge[maxBase]`. The
/// zone-1 projection reads the above row AND the above-right; the zone-3
/// projection reads the left column AND the below-left (`base` up to `maxBase`),
/// so callers supply the wider IDIF edge `Edge[-2 ..= w + h + 1]` (length
/// `w + h + 4` for `mrlIndex == 0`); slice index zero is the logical `-2`
/// sample. Use [`IntraDirectionalAngleIdifEdges::above`] for the zone-1 above
/// edge (D45/D67) and [`IntraDirectionalAngleIdifEdges::left`] for the zone-3
/// left edge (D203); the edge must match the angle's required edge.
///
/// For pAngle 45 (`dx = Dr_Intra_Derivative[45] = 64`) every projection has
/// `shift == 0`, so the IDIF 4-tap reduces to the copy `Edge[base]`
/// (bit-identical to the bilinear branch); D203 (`dy = Dr_Intra_Derivative[67] =
/// 24`) has genuinely nonzero shifts, exercising the 4-tap filter — but both
/// read far into the real reconstructed one-sided extension (above-right /
/// below-left) that the middle-angle path never touches. This primitive covers
/// only the no-MRL one-sided angles over already-prepared edges; `mrlIndex == 0`
/// is assumed.
///
/// `output` points at the top-left destination sample and `stride_samples` is
/// the distance between adjacent output rows.
///
/// # Errors
/// Returns [`ReconError`] for unsupported sample type/bit depth combinations,
/// an unsupported pAngle, a prepared edge that does not match the angle's
/// required edge, a wrong-length prepared edge, out-of-range edge samples, a
/// too-small stride, a too-small output buffer, or checked arithmetic overflow.
pub fn predict_intra_directional_angle_rect_one_sided_idif_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleIdifEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        bit_depth,
        size,
        angle,
        edges,
        0,
        output,
        stride_samples,
    )
}

/// Writes a supported luma IDIF one-sided directional prediction at a §5.20.5.5
/// `mrl_index` (the multi-reference-line distance; `0` is the immediate edge).
///
/// The §7.13.2.8 projection shifts out by `mrl_index` reference lines: `idx =
/// (scaled + 1 + mrl_index) * derivative`, `maxBase = w + h - 1 + (mrlIndex <<
/// 1)`. The prepared `edges` slice must be the wider MRL edge (length `w + h + 4 +
/// (mrlIndex << 1)`); index zero is still the logical `-2` sample. Otherwise
/// identical to [`predict_intra_directional_angle_rect_one_sided_idif_into`].
///
/// # Errors
/// Returns the same [`ReconError`] cases as
/// [`predict_intra_directional_angle_rect_one_sided_idif_into`].
pub fn predict_intra_directional_angle_rect_one_sided_idif_mrl_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleIdifEdges<'_, T>,
    mrl_index: usize,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let edge = validate_one_sided_idif_inputs(
        bit_depth,
        size,
        angle,
        edges,
        mrl_index,
        output.len(),
        stride_samples,
    )?;
    write_one_sided_idif_prediction(
        bit_depth,
        size,
        angle,
        edge,
        mrl_index,
        output,
        stride_samples,
    )
}

/// Writes a supported luma IDIF zone-1 one-sided directional prediction from a
/// raw AV2 pAngle.
///
/// # Errors
/// In addition to [`predict_intra_directional_angle_rect_one_sided_idif_into`]
/// errors, returns [`ReconError::UnsupportedIntraDirectionalAngle`] before
/// output mutation when `p_angle` is not a supported zone-1 above angle.
pub fn predict_intra_directional_angle_rect_one_sided_idif_from_p_angle_into<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    p_angle: u16,
    edges: IntraDirectionalAngleIdifEdges<'_, T>,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
    predict_intra_directional_angle_rect_one_sided_idif_into(
        bit_depth,
        size,
        angle,
        edges,
        output,
        stride_samples,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectionalAngleBranch {
    Above { derivative: u16 },
    Left { derivative: u16 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MiddleDirectionalAngleBranch {
    dx: u16,
    dy: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MiddleSampleReference {
    edge: IntraDirectionalAngleEdge,
    base: i64,
    shift: u16,
}

#[derive(Clone, Copy, Debug)]
struct ValidatedInputs<'a, T: ReconSample> {
    edge: &'a [T],
}

#[derive(Clone, Copy, Debug)]
struct ValidatedMiddleInputs<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
}

#[derive(Clone, Copy, Debug)]
struct ValidatedMiddleIdifInputs<'a, T: ReconSample> {
    left: &'a [T],
    above: &'a [T],
}

fn validate_inputs<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleEdges<'_, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<ValidatedInputs<'_, T>> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        size,
        output_len,
        stride_samples,
        "intra prediction output buffer length",
    )?;
    let expected_len = required_edge_len(size)?;
    validate_index_bounds(size, angle, expected_len)?;

    let edge_kind = angle.required_edge();
    let edge = match edge_kind {
        IntraDirectionalAngleEdge::Left => {
            edges
                .left
                .ok_or(ReconError::IntraDirectionalAngleEdgeUnavailable {
                    p_angle: angle.p_angle(),
                    edge: edge_kind,
                })?
        }
        IntraDirectionalAngleEdge::Above => {
            edges
                .above
                .ok_or(ReconError::IntraDirectionalAngleEdgeUnavailable {
                    p_angle: angle.p_angle(),
                    edge: edge_kind,
                })?
        }
    };
    validate_edge(edge_kind, edge, expected_len, bit_depth)?;

    Ok(ValidatedInputs { edge })
}

fn validate_middle_inputs<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleEdges<'_, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<ValidatedMiddleInputs<'_, T>> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        size,
        output_len,
        stride_samples,
        "intra prediction output buffer length",
    )?;

    let left = edges.left_with_minus_one.ok_or(
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Left,
        },
    )?;
    let above = edges.above_with_minus_one.ok_or(
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Above,
        },
    )?;

    let left_len = required_middle_left_len(size)?;
    let above_len = required_middle_above_len(size)?;
    validate_middle_edge(IntraDirectionalAngleEdge::Left, left, left_len, bit_depth)?;
    validate_middle_edge(
        IntraDirectionalAngleEdge::Above,
        above,
        above_len,
        bit_depth,
    )?;

    validate_middle_index_bounds(size, angle, left.len(), above.len())?;

    Ok(ValidatedMiddleInputs { left, above })
}

fn validate_middle_idif_inputs<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    edges: IntraMiddleDirectionalAngleIdifEdges<'_, T>,
    output_len: usize,
    stride_samples: usize,
) -> Result<ValidatedMiddleIdifInputs<'_, T>> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        size,
        output_len,
        stride_samples,
        "intra prediction output buffer length",
    )?;

    let left = edges
        .left_idif
        .ok_or(ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Left,
        })?;
    let above = edges
        .above_idif
        .ok_or(ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle,
            edge: IntraDirectionalAngleEdge::Above,
        })?;

    let left_len = required_middle_idif_left_len(size)?;
    let above_len = required_middle_idif_above_len(size)?;
    validate_middle_edge(IntraDirectionalAngleEdge::Left, left, left_len, bit_depth)?;
    validate_middle_edge(
        IntraDirectionalAngleEdge::Above,
        above,
        above_len,
        bit_depth,
    )?;

    validate_middle_idif_index_bounds(size, angle, left.len(), above.len())?;

    Ok(ValidatedMiddleIdifInputs { left, above })
}

/// IDIF left edge length: logical `-2 ..= h + 1` is `h + 4` samples.
fn required_middle_idif_left_len(size: IntraRectBlockSize) -> Result<usize> {
    size.height()
        .checked_add(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle IDIF left edge length",
        })
}

/// IDIF above edge length: logical `-2 ..= w + 1` is `w + 4` samples.
fn required_middle_idif_above_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(4)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle IDIF above edge length",
        })
}

fn validate_middle_idif_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left_len: usize,
    above_len: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let len = match reference.edge {
                IntraDirectionalAngleEdge::Left => left_len,
                IntraDirectionalAngleEdge::Above => above_len,
            };
            for tap in 0..(DR_INTERP_FILTER_TAPS as i64) {
                let logical =
                    reference
                        .base
                        .checked_add(tap - 1)
                        .ok_or(ReconError::ArithmeticOverflow {
                            context: "middle directional angle IDIF tap index",
                        })?;
                logical_idif_edge_offset(logical, len)?;
            }
        }
    }
    Ok(())
}

fn required_edge_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(size.height())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle prepared edge length",
        })
}

fn required_middle_left_len(size: IntraRectBlockSize) -> Result<usize> {
    size.height()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle left edge length",
        })
}

fn required_middle_above_len(size: IntraRectBlockSize) -> Result<usize> {
    size.width()
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle above edge length",
        })
}

fn validate_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edge_len: usize,
) -> Result<()> {
    let max_base = edge_len
        .checked_sub(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle maximum base index",
        })?;
    let branch = angle.branch();
    let (outer, inner) = match branch {
        DirectionalAngleBranch::Above { .. } => (size.height(), size.width()),
        DirectionalAngleBranch::Left { .. } => (size.width(), size.height()),
    };
    let derivative = match branch {
        DirectionalAngleBranch::Above { derivative }
        | DirectionalAngleBranch::Left { derivative } => usize::from(derivative),
    };
    let max_idx = outer
        .checked_mul(derivative)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "directional angle derivative product",
        })?;
    let max_base_prefix = max_idx >> 6;
    let max_inner = inner.checked_sub(1).ok_or(ReconError::ArithmeticOverflow {
        context: "directional angle inner dimension",
    })?;
    let max_referenced_base =
        max_base_prefix
            .checked_add(max_inner)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "directional angle referenced base index",
            })?;
    if max_referenced_base > max_base {
        return Err(ReconError::ArithmeticOverflow {
            context: "directional angle prepared edge coverage",
        });
    }
    Ok(())
}

fn validate_middle_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left_len: usize,
    above_len: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let len = match reference.edge {
                IntraDirectionalAngleEdge::Left => left_len,
                IntraDirectionalAngleEdge::Above => above_len,
            };
            logical_edge_offset(reference.base, len)?;
            let next_base =
                reference
                    .base
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "middle directional angle next edge base",
                    })?;
            logical_edge_offset(next_base, len)?;
        }
    }
    Ok(())
}

pub(super) fn validate_directional_edge<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    validate_edge(edge, samples, expected_len, bit_depth)
}

fn validate_edge<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    if samples.len() != expected_len {
        return Err(ReconError::IntraDirectionalAngleEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    let max = bit_depth.max_sample();
    for (sample_index, sample) in samples.iter().copied().enumerate() {
        let value = sample.to_u16();
        if value > max {
            return Err(ReconError::IntraDirectionalAngleSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            });
        }
    }

    Ok(())
}

fn validate_middle_edge<T: ReconSample>(
    edge: IntraDirectionalAngleEdge,
    samples: &[T],
    expected_len: usize,
    bit_depth: BitDepth,
) -> Result<()> {
    if samples.len() != expected_len {
        return Err(ReconError::IntraMiddleDirectionalAngleEdgeLengthMismatch {
            edge,
            expected: expected_len,
            actual: samples.len(),
        });
    }

    let max = bit_depth.max_sample();
    for (sample_index, sample) in samples.iter().copied().enumerate() {
        let value = sample.to_u16();
        if value > max {
            return Err(ReconError::IntraMiddleDirectionalAngleSampleOutOfRange {
                edge,
                sample_index,
                value,
                max,
            });
        }
    }

    Ok(())
}

fn write_prediction<T: ReconSample>(
    _bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edge: &[T],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let max_base = required_edge_len(size)? - 1;
    match angle.branch() {
        DirectionalAngleBranch::Above { derivative } => {
            for row in 0..size.height() {
                let idx = (row + 1) * usize::from(derivative);
                let base_prefix = idx >> 6;
                let shift = ((idx >> 1) & 0x1f) as u16;
                let row_start = row * stride_samples;
                for column in 0..size.width() {
                    let base = base_prefix + column;
                    let value = if base < max_base {
                        bilinear(edge[base], edge[base + 1], shift)
                    } else {
                        edge[max_base].to_u16()
                    };
                    output[row_start + column] = T::try_from_u16(value)?;
                }
            }
        }
        DirectionalAngleBranch::Left { derivative } => {
            for row in 0..size.height() {
                let row_start = row * stride_samples;
                for column in 0..size.width() {
                    let idx = (column + 1) * usize::from(derivative);
                    let base = (idx >> 6) + row;
                    let shift = ((idx >> 1) & 0x1f) as u16;
                    let value = if base < max_base {
                        bilinear(edge[base], edge[base + 1], shift)
                    } else {
                        edge[max_base].to_u16()
                    };
                    output[row_start + column] = T::try_from_u16(value)?;
                }
            }
        }
    }

    Ok(())
}

/// Validates the zone-1 one-sided IDIF inputs and returns the prepared above
/// edge. The supported zone-1 above angle (D45) reads only the above edge.
fn validate_one_sided_idif_inputs<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edges: IntraDirectionalAngleIdifEdges<'_, T>,
    mrl_index: usize,
    output_len: usize,
    stride_samples: usize,
) -> Result<&[T]> {
    validate_sample_type::<T>(bit_depth)?;
    validate_output_shape(
        size,
        output_len,
        stride_samples,
        "intra prediction output buffer length",
    )?;
    let direction = match angle.branch() {
        DirectionalAngleBranch::Above { .. } => IntraDirectionalAngleEdge::Above,
        DirectionalAngleBranch::Left { .. } => IntraDirectionalAngleEdge::Left,
    };
    if angle.required_edge() != direction || edges.direction != direction {
        return Err(ReconError::UnsupportedIntraDirectionalAngle {
            p_angle: angle.p_angle(),
        });
    }
    let edge = edges.edge;
    let edge_len = required_one_sided_idif_edge_len(size, mrl_index)?;
    validate_edge(direction, edge, edge_len, bit_depth)?;
    validate_one_sided_idif_index_bounds(size, angle, mrl_index, edge.len())?;
    Ok(edge)
}

/// One-sided IDIF edge length: logical `-2 ..= w + h + 1 + (mrlIndex << 1)` is
/// `w + h + 4 + (mrlIndex << 1)` samples, for both the zone-1 above edge and the
/// zone-3 left edge. `mrl_index == 0` is the immediate reference line.
fn required_one_sided_idif_edge_len(size: IntraRectBlockSize, mrl_index: usize) -> Result<usize> {
    required_edge_len(size)?
        .checked_add(4)
        .and_then(|v| v.checked_add(mrl_index << 1))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "one-sided directional angle IDIF edge length",
        })
}

fn validate_one_sided_idif_index_bounds(
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    mrl_index: usize,
    edge_len: usize,
) -> Result<()> {
    let derivative = one_sided_idif_derivative(angle);
    let branch = angle.branch();
    let max_base = one_sided_max_base(size, mrl_index)?;
    for row in 0..size.height() {
        for column in 0..size.width() {
            let reference = one_sided_idif_reference(branch, row, column, derivative, mrl_index)?;
            if reference.base <= max_base {
                for tap in 0..(DR_INTERP_FILTER_TAPS as i64) {
                    let logical = reference.base.checked_add(tap - 1).ok_or(
                        ReconError::ArithmeticOverflow {
                            context: "one-sided directional angle IDIF tap index",
                        },
                    )?;
                    logical_idif_edge_offset(logical, edge_len)?;
                }
            } else {
                logical_idif_edge_offset(max_base, edge_len)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OneSidedReference {
    base: i64,
    shift: u16,
}

/// The §7.13.2.8 one-sided derivative: `dx = Dr_Intra_Derivative[pAngle]` for the
/// zone-1 above angle (step 1) or `dy = Dr_Intra_Derivative[270 - pAngle]` for
/// the zone-3 left angle (step 3). Both are carried in the branch's `derivative`.
fn one_sided_idif_derivative(angle: IntraDirectionalAngle) -> i64 {
    match angle.branch() {
        DirectionalAngleBranch::Above { derivative }
        | DirectionalAngleBranch::Left { derivative } => i64::from(derivative),
    }
}

/// §7.13.2.8 `maxBaseX = maxBaseY = w + h - 1 + (mrlIndex << 1)`; identical for
/// the zone-1 above and zone-3 left one-sided projections. `mrl_index == 0` is the
/// immediate reference line.
fn one_sided_max_base(size: IntraRectBlockSize, mrl_index: usize) -> Result<i64> {
    let max_base = required_edge_len(size)?
        .checked_sub(1)
        .and_then(|v| v.checked_add(mrl_index << 1))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "one-sided directional angle maxBase",
        })?;
    i64::try_from(max_base).map_err(|_| ReconError::ArithmeticOverflow {
        context: "one-sided directional angle maxBase range",
    })
}

/// AV2 §7.13.2.8 one-sided projection for one predicted sample. Zone-1 above
/// (step 1, `pAngle < 90`): `idx = (i + 1 + mrlIndex) * dx`, `base = (idx >> 6) +
/// j`. Zone-3 left (step 3, `pAngle > 180`): `idx = (j + 1 + mrlIndex) * dy`,
/// `base = (idx >> 6) + i`. `shift = (idx >> 1) & 0x1F` in both. The §5.20.5.5
/// `mrlIndex` shifts the projection out by `mrlIndex` reference lines (`mrlIndex ==
/// 0` is the immediate edge).
fn one_sided_idif_reference(
    branch: DirectionalAngleBranch,
    row: usize,
    column: usize,
    derivative: i64,
    mrl_index: usize,
) -> Result<OneSidedReference> {
    let (scaled, offset) = match branch {
        DirectionalAngleBranch::Above { .. } => (row, column),
        DirectionalAngleBranch::Left { .. } => (column, row),
    };
    let scaled_plus_one = scaled
        .checked_add(1)
        .and_then(|v| v.checked_add(mrl_index))
        .and_then(|v| i64::try_from(v).ok())
        .ok_or(ReconError::ArithmeticOverflow {
            context: "one-sided directional angle projection index",
        })?;
    let idx = scaled_plus_one
        .checked_mul(derivative)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "one-sided directional angle derivative product",
        })?;
    let offset_i64 = i64::try_from(offset).map_err(|_| ReconError::ArithmeticOverflow {
        context: "one-sided directional angle offset index",
    })?;
    let base = (idx >> 6)
        .checked_add(offset_i64)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "one-sided directional angle base index",
        })?;
    Ok(OneSidedReference {
        base,
        shift: directional_shift(idx),
    })
}

fn write_one_sided_idif_prediction<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraDirectionalAngle,
    edge: &[T],
    mrl_index: usize,
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let derivative = one_sided_idif_derivative(angle);
    let branch = angle.branch();
    let max_base = one_sided_max_base(size, mrl_index)?;
    for row in 0..size.height() {
        let row_start = row * stride_samples;
        for column in 0..size.width() {
            let reference = one_sided_idif_reference(branch, row, column, derivative, mrl_index)?;
            let value = if reference.base <= max_base {
                idif_tap(edge, reference.base, reference.shift, bit_depth)?
            } else {
                logical_idif_edge_sample(edge, max_base)?.to_u16()
            };
            output[row_start + column] = T::try_from_u16(value)?;
        }
    }
    Ok(())
}

fn write_middle_prediction<T: ReconSample>(
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left: &[T],
    above: &[T],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        let row_start = row * stride_samples;
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let edge = match reference.edge {
                IntraDirectionalAngleEdge::Left => left,
                IntraDirectionalAngleEdge::Above => above,
            };
            let next_base =
                reference
                    .base
                    .checked_add(1)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "middle directional angle next edge base",
                    })?;
            let value = bilinear(
                logical_edge_sample(edge, reference.base)?,
                logical_edge_sample(edge, next_base)?,
                reference.shift,
            );
            output[row_start + column] = T::try_from_u16(value)?;
        }
    }

    Ok(())
}

fn write_middle_idif_prediction<T: ReconSample>(
    bit_depth: BitDepth,
    size: IntraRectBlockSize,
    angle: IntraMiddleDirectionalAngle,
    left: &[T],
    above: &[T],
    output: &mut [T],
    stride_samples: usize,
) -> Result<()> {
    let branch = angle.branch()?;
    for row in 0..size.height() {
        let row_start = row * stride_samples;
        for column in 0..size.width() {
            let reference = middle_sample_reference(row, column, branch)?;
            let edge = match reference.edge {
                IntraDirectionalAngleEdge::Left => left,
                IntraDirectionalAngleEdge::Above => above,
            };
            let value = idif_tap(edge, reference.base, reference.shift, bit_depth)?;
            output[row_start + column] = T::try_from_u16(value)?;
        }
    }

    Ok(())
}

/// Computes one §7.13.2.8 IDIF sample: `s = Σ(t=0..3) Dr_Interp_Filter[shift][t]
/// * Edge[base + t - 1]`, then `Clip1(Round2(s, 7))`. The 4-tap sum is signed
/// (the filter has negative taps), so `Round2` floors the signed value and
/// `Clip1` clamps a negative result to `0`.
fn idif_tap<T: ReconSample>(edge: &[T], base: i64, shift: u16, bit_depth: BitDepth) -> Result<u16> {
    let taps = DR_INTERP_FILTER
        .get(usize::from(shift))
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle IDIF shift index",
        })?;
    let mut sum: i64 = 0;
    for (tap_index, &tap) in taps.iter().enumerate() {
        let logical =
            base.checked_add(tap_index as i64 - 1)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "middle directional angle IDIF tap index",
                })?;
        let sample = i64::from(logical_idif_edge_sample(edge, logical)?.to_u16());
        let term = i64::from(tap)
            .checked_mul(sample)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle IDIF tap product",
            })?;
        sum = sum
            .checked_add(term)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle IDIF tap sum",
            })?;
    }
    Ok(idif_round2_clip(sum, bit_depth))
}

/// AV2 §4 `Clip1(Round2(s, 7))` for a signed IDIF sum: `Round2(x, n) =
/// ⌊(x + 2^(n-1)) / 2^n⌋` (floor, via arithmetic shift on the signed value),
/// then clamp to `0..=max_sample`.
fn idif_round2_clip(sum: i64, bit_depth: BitDepth) -> u16 {
    let rounded = (sum + (1i64 << (DR_INTERP_FILTER_ROUND - 1))) >> DR_INTERP_FILTER_ROUND;
    if rounded <= 0 {
        return 0;
    }
    let max = i64::from(bit_depth.max_sample());
    let clamped = rounded.min(max);
    clip1(clamped as u16, bit_depth)
}

fn logical_idif_edge_sample<T: ReconSample>(samples: &[T], logical_index: i64) -> Result<T> {
    let offset = logical_idif_edge_offset(logical_index, samples.len())?;
    Ok(samples[offset])
}

/// Maps an IDIF logical edge index (`-2 ..= side + 1`) to a slice offset:
/// `slice[0]` is logical `-2`, so `offset = logical_index + 2`.
fn logical_idif_edge_offset(logical_index: i64, len: usize) -> Result<usize> {
    let shifted = logical_index
        .checked_add(2)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle IDIF logical edge offset",
        })?;
    let offset = usize::try_from(shifted).map_err(|_| ReconError::ArithmeticOverflow {
        context: "middle directional angle IDIF logical edge coverage",
    })?;
    if offset >= len {
        return Err(ReconError::ArithmeticOverflow {
            context: "middle directional angle IDIF logical edge coverage",
        });
    }
    Ok(offset)
}

fn middle_sample_reference(
    row: usize,
    column: usize,
    branch: MiddleDirectionalAngleBranch,
) -> Result<MiddleSampleReference> {
    let column_scaled = checked_scaled_i64(column, "middle directional angle above index prefix")?;
    let row_plus_one = checked_usize_plus_one_i64(row, "middle directional angle above row index")?;
    let above_delta =
        row_plus_one
            .checked_mul(i64::from(branch.dx))
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle above derivative product",
            })?;
    let above_idx =
        column_scaled
            .checked_sub(above_delta)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "middle directional angle above index",
            })?;
    let above_base = above_idx >> 6;
    if above_base >= -1 {
        return Ok(MiddleSampleReference {
            edge: IntraDirectionalAngleEdge::Above,
            base: above_base,
            shift: directional_shift(above_idx),
        });
    }

    let row_scaled = checked_scaled_i64(row, "middle directional angle left index prefix")?;
    let column_plus_one =
        checked_usize_plus_one_i64(column, "middle directional angle left column index")?;
    let left_delta = column_plus_one.checked_mul(i64::from(branch.dy)).ok_or(
        ReconError::ArithmeticOverflow {
            context: "middle directional angle left derivative product",
        },
    )?;
    let left_idx = row_scaled
        .checked_sub(left_delta)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle left index",
        })?;
    Ok(MiddleSampleReference {
        edge: IntraDirectionalAngleEdge::Left,
        base: left_idx >> 6,
        shift: directional_shift(left_idx),
    })
}

fn checked_scaled_i64(value: usize, context: &'static str) -> Result<i64> {
    let value = i64::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })?;
    value
        .checked_mul(64)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn checked_usize_plus_one_i64(value: usize, context: &'static str) -> Result<i64> {
    let value = i64::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })?;
    value
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn directional_shift(idx: i64) -> u16 {
    ((idx >> 1) & 0x1f) as u16
}

fn logical_edge_sample<T: ReconSample>(samples: &[T], logical_index: i64) -> Result<T> {
    let offset = logical_edge_offset(logical_index, samples.len())?;
    Ok(samples[offset])
}

fn logical_edge_offset(logical_index: i64, len: usize) -> Result<usize> {
    let shifted = logical_index
        .checked_add(1)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "middle directional angle logical edge offset",
        })?;
    let offset = usize::try_from(shifted).map_err(|_| ReconError::ArithmeticOverflow {
        context: "middle directional angle logical edge coverage",
    })?;
    if offset >= len {
        return Err(ReconError::ArithmeticOverflow {
            context: "middle directional angle logical edge coverage",
        });
    }
    Ok(offset)
}

fn bilinear<T: ReconSample>(a: T, b: T, shift: u16) -> u16 {
    let weighted = u64::from(a.to_u16()) * u64::from(INTERP_SCALE - shift)
        + u64::from(b.to_u16()) * u64::from(shift);
    round2(weighted, 5)
}

/// Number of §7.13.2.18 intra edge filter kernel rows (`INTRA_EDGE_KERNELS`).
const INTRA_EDGE_KERNELS: usize = 3;
/// Number of taps per §7.13.2.18 intra edge filter kernel (`INTRA_EDGE_TAPS`).
const INTRA_EDGE_TAPS: usize = 5;

/// AV2 §7.13.2.18 `Intra_Edge_Kernel[INTRA_EDGE_KERNELS][INTRA_EDGE_TAPS]`,
/// transcribed VERBATIM from the committed spec mirror
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-18`. Indexed by
/// `strength - 1` (strength `1..=3`).
const INTRA_EDGE_KERNEL: [[i32; INTRA_EDGE_TAPS]; INTRA_EDGE_KERNELS] =
    [[0, 4, 8, 4, 0], [0, 5, 6, 5, 0], [2, 4, 4, 4, 2]];

/// AV2 §7.13.2.18 Intra edge filter process. Filters the first `sz` samples of
/// `edge` in place with the given `strength` (`0..=3`). `strength == 0` is a
/// no-op (the process returns without modifying the edge). Transcribed VERBATIM
/// from the committed spec mirror (`av2_filter_intra_edge_high_c`,
/// `~/Devel/avm/av2/common/reconintra.c:997-1018`):
///
/// `edge` holds the §7.13.2.1 reference samples with `edge[i]` the §7.13.2.18
/// array element (the caller positions the slice so `edge[0]` is the logical
/// `AboveRow[-1]` / `LeftCol[-1]` corner, `edge[1]` the logical `[0]`, etc). For
/// `i = 1..sz-1`: `s = Σ(j=0..4) Intra_Edge_Kernel[strength-1][j] *
/// edge[Clip3(0, sz-1, i-2+j)]`, then `edge[i] = (s + 8) >> 4`. The `i == 0`
/// corner sample is read but never overwritten (matching the §7.13.2.18
/// `AboveRow[-1]` / `LeftCol[-1]` invariance), so a preceding §7.13.2.14 corner
/// rewrite survives into the §7.13.2.8 prediction.
///
/// # Errors
/// Returns [`ReconError::ArithmeticOverflow`] when `sz` exceeds `edge.len()`, or
/// a filtered sample falls outside the representable range (the `(s + 8) >> 4`
/// average of in-range samples is always representable, so this cannot fire for
/// valid edges, but the conversion is checked rather than truncating).
pub fn apply_intra_edge_filter<T: ReconSample>(
    edge: &mut [T],
    sz: usize,
    strength: u8,
) -> Result<()> {
    if strength == 0 {
        return Ok(());
    }
    if sz == 0 {
        return Ok(());
    }
    if sz > edge.len() {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra edge filter size exceeds edge length",
        });
    }
    let Some(filt) = usize::from(strength).checked_sub(1) else {
        return Ok(());
    };
    let Some(kernel) = INTRA_EDGE_KERNEL.get(filt) else {
        return Err(ReconError::ArithmeticOverflow {
            context: "intra edge filter strength out of range",
        });
    };
    let original: Vec<u16> = edge[..sz].iter().map(|s| s.to_u16()).collect();
    let last = sz - 1;
    for (i, slot) in edge.iter_mut().enumerate().take(sz).skip(1) {
        let mut s: i32 = 0;
        for (j, &tap) in kernel.iter().enumerate() {
            let raw = i as i64 - 2 + j as i64;
            let k = raw.clamp(0, last as i64) as usize;
            s += tap * i32::from(original[k]);
        }
        let value = (s + 8) >> 4;
        let value = u16::try_from(value).map_err(|_| ReconError::ArithmeticOverflow {
            context: "intra edge filter output out of range",
        })?;
        *slot = T::try_from_u16(value)?;
    }
    Ok(())
}

/// AV2 §7.13.2.14 Filter corner process. Returns the three-tap top-left corner
/// value `Round2(LeftCol[0] * 5 + AboveRow[-1] * 6 + AboveRow[0] * 5, 4)`,
/// written by the caller into BOTH `AboveRow[-1]` and `LeftCol[-1]`. Transcribed
/// VERBATIM from the committed spec mirror
/// (`filter_intra_edge_corner_high`, `~/Devel/avm/av2/common/reconintra.c:1020-1028`):
/// `s = left0 * 5 + above_neg1 * 6 + above0 * 5; out = (s + 8) >> 4`.
#[must_use]
pub fn filter_intra_edge_corner(left0: u16, above_neg1: u16, above0: u16) -> u16 {
    let s = i32::from(left0) * 5 + i32::from(above_neg1) * 6 + i32::from(above0) * 5;
    ((s + 8) >> 4) as u16
}

#[cfg(test)]
#[path = "intra_directional_angle_tests.rs"]
mod tests;
