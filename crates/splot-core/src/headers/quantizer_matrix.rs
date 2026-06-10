// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 quantizer matrix OBU syntax model (AV2 v1.0.0 § 5.13, § 5.4.11).
//!
//! [`parse_quantizer_matrix`] reads `quantizer_matrix_obu()` (§ 5.13). The OBU
//! carries a 15-bit `qm_bit_map` selecting which custom quantizer-matrix levels are
//! present:
//!
//! - `qm_bit_map == 0` is the reset/default path: no per-level payload follows; the
//!   validator marks every level protected/default (§ 6.12).
//! - For each set level bit, `qm_is_default_flag` selects the default matrix or a
//!   user-defined matrix parsed by the shared `user_defined_qm` helper (§ 5.4.11).
//!
//! `user_defined_qm` is shared syntax: per the AV2 grammar it is reached only from
//! quantizer-matrix syntax (here via [`parse_quantizer_matrix`]), **not** as a direct
//! `sequence_header_obu()` child call, and the same helper is reusable by future
//! frame-level quantization syntax. The three fundamental transform shapes
//! `Fundamental_Tx_Size[3] = { TX_8X8, TX_8X4, TX_4X8 }` are filled in AV2 2D
//! diagonal scan order (`get_scan(txSz, TX_CLASS_2D)`, § 5.20.7.30) with `svlc()`
//! coefficient deltas. Scan/row-column derivation follows the AV2 spec (§ 5.20.7.30,
//! § 9 transform-size tables) and the AVM oracle `read_qm_obu` / `read_qm_data`
//! (`av2/decoder/obu_qm.c`); no AV1 scan or transform tables are copied.

use crate::bitio::BitReader;
use crate::error::{Error, Result};

/// `NUM_CUSTOM_QMS`: number of custom quantizer-matrix levels (AV2 v1.0.0 § 3).
pub const NUM_CUSTOM_QMS: usize = 15;

/// Inclusive minimum of the conformant `quant_delta` range (AV2 v1.0.0 § 6.4.11).
const QUANT_DELTA_MIN: i32 = -128;
/// Inclusive maximum of the conformant `quant_delta` range (AV2 v1.0.0 § 6.4.11).
const QUANT_DELTA_MAX: i32 = 127;

/// `qm_bit_map` is a 15-bit field (`f(NUM_CUSTOM_QMS)`).
const QM_BIT_MAP_BITS: u32 = NUM_CUSTOM_QMS as u32;

/// Initial running quantizer value for the user-defined fill (`quant = 32`,
/// AV2 v1.0.0 § 5.4.11).
const INITIAL_QM_QUANT: u8 = 32;

/// One of the three fundamental quantizer-matrix transform shapes
/// `Fundamental_Tx_Size[3] = { TX_8X8, TX_8X4, TX_4X8 }` (AV2 v1.0.0 § 5.4.11).
///
/// The width/height come from the AV2 `Tx_Width` / `Tx_Height` tables (§ 9) for
/// `TX_8X8` (8x8), `TX_8X4` (8 wide, 4 high), and `TX_4X8` (4 wide, 8 high).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundamentalQmTransform {
    /// `TX_8X8` (`t == 0`): 8x8, the `qm_8x8_is_symmetric` path.
    Tx8x8,
    /// `TX_8X4` (`t == 1`): 8 wide, 4 high.
    Tx8x4,
    /// `TX_4X8` (`t == 2`): 4 wide, 8 high, the `qm_4x8_is_transpose_of_8x4` path.
    Tx4x8,
}

impl FundamentalQmTransform {
    /// The three fundamental transforms in `Fundamental_Tx_Size` order.
    pub const ALL: [Self; 3] = [Self::Tx8x8, Self::Tx8x4, Self::Tx4x8];

    /// Returns `(width, height)` in samples (AV2 `Tx_Width` / `Tx_Height`, § 9).
    #[must_use]
    pub const fn dimensions(self) -> (u8, u8) {
        match self {
            Self::Tx8x8 => (8, 8),
            Self::Tx8x4 => (8, 4),
            Self::Tx4x8 => (4, 8),
        }
    }

    /// Returns a stable `"WxH"` shape label for inspector/JSON output.
    #[must_use]
    pub const fn shape_label(self) -> &'static str {
        match self {
            Self::Tx8x8 => "8x8",
            Self::Tx8x4 => "8x4",
            Self::Tx4x8 => "4x8",
        }
    }
}

/// Parsed `quantizer_matrix_obu()` syntax (AV2 v1.0.0 § 5.13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuantizerMatrixObu {
    /// `qm_bit_map` (`f(15)`): bitmask of present quantizer-matrix levels. A value of
    /// `0` is the reset/default path (no per-level payload).
    pub qm_bit_map: u16,
    /// `qm_chroma_info_present_flag` (`f(1)`): whether chroma matrices are present.
    pub chroma_info_present: bool,
    /// `numPlanes = qm_chroma_info_present_flag ? 3 : 1`.
    pub num_planes: u8,
    /// Per-level records for each set bit of `qm_bit_map`, in ascending level order.
    /// Empty for the reset/default path (`qm_bit_map == 0`).
    pub levels: Vec<QuantizerMatrixLevel>,
}

impl QuantizerMatrixObu {
    /// Returns `true` for the reset/default OBU (`qm_bit_map == 0`, § 6.12).
    #[must_use]
    pub const fn is_reset(&self) -> bool {
        self.qm_bit_map == 0
    }
}

/// One quantizer-matrix level selected by a set bit of `qm_bit_map`
/// (AV2 v1.0.0 § 5.13).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuantizerMatrixLevel {
    /// Level index in `0..NUM_CUSTOM_QMS`.
    pub level: u8,
    /// `qm_is_default_flag` (`f(1)`): `true` selects the default matrix (no
    /// user-defined data; `QmDataPresent` is set to 0).
    pub is_default: bool,
    /// User-defined matrices (one [`UserDefinedQmTransform`] per fundamental
    /// transform), present only when `is_default` is `false`.
    pub matrices: Option<Vec<UserDefinedQmTransform>>,
}

/// One fundamental-transform matrix set within a user-defined quantizer-matrix level
/// (AV2 v1.0.0 § 5.4.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserDefinedQmTransform {
    /// Which fundamental transform shape this matrix set covers.
    pub transform: FundamentalQmTransform,
    /// One plane per `numPlanes`, in plane order (`Y`, then `U`, then `V`).
    pub planes: Vec<UserDefinedQmPlane>,
}

/// A single user-defined quantizer matrix plane (AV2 v1.0.0 § 5.4.11).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UserDefinedQmPlane {
    /// Matrix width in samples.
    pub width: u8,
    /// Matrix height in samples.
    pub height: u8,
    /// Row-major `height * width` coefficients (`UserQm[level][t][plane][row][col]`).
    /// Coefficients are in `1..=255`; a `quant2 == 0` result triggers coefficient
    /// repeat (the last value is replicated to the end) rather than a stored `0`.
    pub values: Vec<u8>,
}

/// Parses a `quantizer_matrix_obu()` (AV2 v1.0.0 § 5.13).
///
/// The defining layer ids (`obu_tlayer_id` / `obu_mlayer_id`, used for per-level HLS
/// availability, § 6.12) come from the OBU header rather than the payload, so the
/// validator reads them from the OBU envelope instead of threading them through here.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`],
/// [`Error::InvalidUvlc`], or
/// [`Error::BitWidthTooLarge`] from
/// [`BitReader`] when the input is truncated or a descriptor is malformed, or
/// [`Error::InvalidQuantizerMatrix`] when a
/// `quant_delta` is outside the conformant `-128..=127` range (AV2 § 6.4.11).
pub fn parse_quantizer_matrix(reader: &mut BitReader<'_>) -> Result<QuantizerMatrixObu> {
    // qm_bit_map fits in 15 bits, so the u32 read never exceeds u16::MAX.
    let qm_bit_map = reader.read_bits(QM_BIT_MAP_BITS)? as u16;
    let chroma_info_present = reader.read_bit()? != 0;
    let num_planes: u8 = if chroma_info_present { 3 } else { 1 };

    let mut levels = Vec::new();
    if qm_bit_map != 0 {
        for level in 0..NUM_CUSTOM_QMS {
            if qm_bit_map & (1 << level) == 0 {
                continue;
            }
            let is_default = reader.read_bit()? != 0;
            let matrices = if is_default {
                None
            } else {
                Some(parse_user_defined_qm_level(reader, num_planes)?)
            };
            // level < NUM_CUSTOM_QMS (15), so it fits in u8.
            levels.push(QuantizerMatrixLevel {
                level: level as u8,
                is_default,
                matrices,
            });
        }
    }

    Ok(QuantizerMatrixObu {
        qm_bit_map,
        chroma_info_present,
        num_planes,
        levels,
    })
}

/// Parses the `for t { for plane { user_defined_qm(level, t, plane) } }` loop of a
/// single non-default level (AV2 v1.0.0 § 5.13 / § 5.4.11).
fn parse_user_defined_qm_level(
    reader: &mut BitReader<'_>,
    num_planes: u8,
) -> Result<Vec<UserDefinedQmTransform>> {
    let mut transforms: Vec<UserDefinedQmTransform> =
        Vec::with_capacity(FundamentalQmTransform::ALL.len());
    for t in 0..FundamentalQmTransform::ALL.len() {
        transforms.push(UserDefinedQmTransform {
            transform: FundamentalQmTransform::ALL[t],
            planes: Vec::with_capacity(num_planes as usize),
        });
        for plane in 0..num_planes as usize {
            user_defined_qm(reader, &mut transforms, t, plane)?;
        }
    }
    Ok(transforms)
}

/// Parses one `user_defined_qm(level, t, plane)` matrix and appends it to
/// `transforms[t].planes` (AV2 v1.0.0 § 5.4.11).
///
/// `transforms[t]` must already exist (with `planes[0..plane]` filled), and for the
/// `TX_4X8` transpose path `transforms[1]` (`TX_8X4`) must be fully populated for this
/// plane — both invariants hold under the § 5.13 / [`parse_user_defined_qm_level`]
/// loop order (`t` ascending, `plane` ascending).
///
/// # Errors
/// Propagates [`BitReader`] descriptor errors when the input is truncated or an
/// `svlc()` coefficient delta is malformed.
fn user_defined_qm(
    reader: &mut BitReader<'_>,
    transforms: &mut [UserDefinedQmTransform],
    t: usize,
    plane: usize,
) -> Result<()> {
    let transform = FundamentalQmTransform::ALL[t];
    let (width, height) = transform.dimensions();
    let (w, h) = (width as usize, height as usize);

    // AV2 § 5.4.11: plane > 0 may copy the previously parsed plane of the same
    // transform (qm_copy_from_previous_plane).
    if plane > 0 {
        let copy_from_previous_plane = reader.read_bit()? != 0;
        if copy_from_previous_plane {
            let values = transforms[t].planes[plane - 1].values.clone();
            transforms[t].planes.push(UserDefinedQmPlane {
                width,
                height,
                values,
            });
            return Ok(());
        }
    }

    let mut symmetric = false;
    if t == 0 {
        // AV2 § 5.4.11: TX_8X8 may signal a symmetric matrix.
        symmetric = reader.read_bit()? != 0;
    } else if t == 2 {
        // AV2 § 5.4.11: TX_4X8 may be the transpose of the same plane's TX_8X4
        // matrix: UserQm[level][2][plane][i][j] = UserQm[level][1][plane][j][i].
        let is_transpose_of_8x4 = reader.read_bit()? != 0;
        if is_transpose_of_8x4 {
            let source = &transforms[1].planes[plane];
            let source_width = source.width as usize;
            let mut values = vec![0u8; w * h];
            for i in 0..h {
                for j in 0..w {
                    values[i * w + j] = source.values[j * source_width + i];
                }
            }
            transforms[t].planes.push(UserDefinedQmPlane {
                width,
                height,
                values,
            });
            return Ok(());
        }
    }

    // AV2 § 5.4.11: scan = get_scan(txSz, TX_CLASS_2D); fill coefficients in scan
    // order with svlc() deltas and the quant2 == 0 coefficient-repeat behavior.
    let scan = diagonal_scan_2d(w, h);
    let mut values = vec![0u8; w * h];
    let mut quant = INITIAL_QM_QUANT;
    let mut coef_repeat = false;
    for pos in scan {
        let row = pos / w;
        let col = pos % w;
        if t == 0 && symmetric && col > row {
            // Mirror the already-filled lower-triangle coefficient (same anti-diagonal,
            // visited earlier in the 2D scan).
            quant = values[col * w + row];
            values[pos] = quant;
        } else if coef_repeat {
            values[pos] = quant;
        } else {
            let delta_offset = reader.byte_offset();
            let delta_bit_offset = reader.bit_offset();
            let quant_delta = reader.read_svlc()?;
            // AV2 § 6.4.11: it is a requirement of bitstream conformance that
            // quant_delta is in -128..=127.
            if !(QUANT_DELTA_MIN..=QUANT_DELTA_MAX).contains(&quant_delta) {
                return Err(Error::InvalidQuantizerMatrix {
                    offset: delta_offset,
                    bit_offset: delta_bit_offset,
                    message: format!(
                        "quant_delta {quant_delta} must be in {QUANT_DELTA_MIN}..={QUANT_DELTA_MAX}"
                    ),
                });
            }
            // AV2 § 5.4.11: quant2 = (quant + quant_delta) & 255. The mask gives the
            // low byte (mathematical mod 256 in 0..=255) even for a negative sum.
            let quant2 = (i32::from(quant) + quant_delta) & 0xFF;
            if quant2 == 0 {
                coef_repeat = true;
            } else {
                quant = quant2 as u8;
            }
            values[pos] = quant;
        }
    }

    transforms[t].planes.push(UserDefinedQmPlane {
        width,
        height,
        values,
    });
    Ok(())
}

/// Builds the AV2 2D (up-right diagonal) coefficient scan for a `width`-by-`height`
/// matrix: `get_scan(txSz, TX_CLASS_2D)` (AV2 v1.0.0 § 5.20.7.30).
///
/// Returns the raster positions `row * width + col` in scan order. The fundamental
/// quantizer-matrix transforms are at most 8x8, so the spec's `Min(.., 32)` width/
/// height clamps never apply.
fn diagonal_scan_2d(width: usize, height: usize) -> Vec<usize> {
    let mut out = vec![0usize; width * height];
    let (w, h) = (width as i64, height as i64);
    let (mut x, mut y) = (0i64, 0i64);
    for slot in out.iter_mut() {
        // Loop invariant (AV2 § 5.20.7.30): 0 <= x < w and 0 <= y < h at each step.
        *slot = (y as usize) * width + (x as usize);
        x += 1;
        y -= 1;
        if y < 0 || x >= w {
            x += 1;
            let s = x.min(h - 1 - y);
            x -= s;
            y += s;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    /// MSB-first bit writer for building QM payloads in tests.
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

        fn uvlc(&mut self, value: u32) {
            let code_num = value + 1;
            let leading_zeros = u32::BITS - 1 - code_num.leading_zeros();
            for _ in 0..leading_zeros {
                self.bit(0);
            }
            self.bit(1);
            if leading_zeros > 0 {
                self.f(code_num - (1 << leading_zeros), leading_zeros);
            }
        }

        /// Appends an `svlc()` code (AV2 § 4.11.4) for `value`.
        fn svlc(&mut self, value: i32) {
            // Invert half = (uvlc + 1) >> 1 with sign in the uvlc parity.
            let uvlc = if value > 0 {
                2 * (value as u32) - 1
            } else {
                2 * (value.unsigned_abs())
            };
            self.uvlc(uvlc);
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

    fn parse(bytes: &[u8]) -> Result<QuantizerMatrixObu> {
        let mut reader = BitReader::new(bytes, ByteOffset::new(0));
        parse_quantizer_matrix(&mut reader)
    }

    /// Fills one plane in scan order with a constant `svlc(0)` delta so every
    /// coefficient equals `start + 0 = start`. Returns the appended bit count helper.
    fn write_flat_plane(bits: &mut Bits, w: usize, h: usize) {
        for _ in 0..(w * h) {
            bits.svlc(0); // delta 0 keeps quant at its running value (32)
        }
    }

    #[test]
    fn svlc_round_trips_in_test_writer() {
        for value in [-4i32, -1, 0, 1, 2, 5] {
            let mut bits = Bits::default();
            bits.svlc(value);
            let data = bits.into_bytes();
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            assert_eq!(reader.read_svlc().unwrap(), value);
        }
    }

    #[test]
    fn diagonal_scan_is_a_permutation() {
        for (w, h) in [(8usize, 8usize), (8, 4), (4, 8)] {
            let scan = diagonal_scan_2d(w, h);
            assert_eq!(scan.len(), w * h);
            let mut seen = vec![false; w * h];
            for &pos in &scan {
                assert!(pos < w * h, "scan position {pos} out of range for {w}x{h}");
                assert!(!seen[pos], "scan position {pos} repeated for {w}x{h}");
                seen[pos] = true;
            }
            assert_eq!(scan[0], 0, "2D scan starts at the DC coefficient");
        }
    }

    #[test]
    fn qm_reset_obu_parses_without_levels() {
        let mut bits = Bits::default();
        bits.f(0, 15); // qm_bit_map == 0
        bits.bit(0); // qm_chroma_info_present_flag
        let data = bits.into_bytes();
        let qm = parse(&data).unwrap();
        assert!(qm.is_reset());
        assert_eq!(qm.num_planes, 1);
        assert!(qm.levels.is_empty());
    }

    #[test]
    fn qm_default_level_parses() {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0 only
        bits.bit(1); // qm_chroma_info_present_flag -> 3 planes
        bits.bit(1); // qm_is_default_flag for level 0
        let data = bits.into_bytes();
        let qm = parse(&data).unwrap();
        assert!(!qm.is_reset());
        assert_eq!(qm.num_planes, 3);
        assert_eq!(qm.levels.len(), 1);
        assert_eq!(qm.levels[0].level, 0);
        assert!(qm.levels[0].is_default);
        assert!(qm.levels[0].matrices.is_none());
    }

    #[test]
    fn user_defined_level_fills_planes_with_symmetry_and_transpose() {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // qm_chroma_info_present_flag = 0 -> 1 plane (Y only)
        bits.bit(0); // qm_is_default_flag = 0 -> user-defined

        // t == 0 (TX_8X8), plane 0: symmetric, flat deltas (all 0 -> all 32).
        bits.bit(1); // qm_8x8_is_symmetric
        // Only lower-triangle-or-diagonal cells (col <= row) read a delta; the rest
        // mirror. For a flat all-32 matrix every read delta is 0.
        let lower_tri_8x8 = (0..8).map(|r| r + 1).sum::<usize>(); // 36 cells
        for _ in 0..lower_tri_8x8 {
            bits.svlc(0);
        }

        // t == 1 (TX_8X4), plane 0: flat, no symmetry/transpose flags.
        write_flat_plane(&mut bits, 8, 4);

        // t == 2 (TX_4X8), plane 0: transpose of the TX_8X4 matrix.
        bits.bit(1); // qm_4x8_is_transpose_of_8x4

        let data = bits.into_bytes();
        let qm = parse(&data).unwrap();
        let level = &qm.levels[0];
        let matrices = level.matrices.as_ref().expect("user-defined matrices");
        assert_eq!(matrices.len(), 3);

        // TX_8X8 symmetric: all coefficients equal the running quant (32).
        let tx8x8 = &matrices[0];
        assert_eq!(tx8x8.transform, FundamentalQmTransform::Tx8x8);
        assert_eq!(tx8x8.planes.len(), 1);
        assert!(tx8x8.planes[0].values.iter().all(|&v| v == 32));
        assert_eq!(tx8x8.planes[0].values.len(), 64);

        // TX_4X8 is the transpose of TX_8X4 (also flat 32), so still all 32.
        let tx4x8 = &matrices[2];
        assert_eq!(tx4x8.transform, FundamentalQmTransform::Tx4x8);
        assert_eq!(tx4x8.planes[0].width, 4);
        assert_eq!(tx4x8.planes[0].height, 8);
        assert!(tx4x8.planes[0].values.iter().all(|&v| v == 32));
    }

    #[test]
    fn user_defined_plane_copy_and_coefficient_repeat() {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(1); // qm_chroma_info_present_flag = 1 -> 3 planes
        bits.bit(0); // qm_is_default_flag = 0

        // t == 0 (TX_8X8), plane 0 (Y): non-symmetric. First delta sets quant to a
        // distinct value, the next delta drives quant2 to 0 to trigger coef repeat.
        bits.bit(0); // qm_8x8_is_symmetric = 0
        bits.svlc(8); // 32 + 8 = 40 at scan position 0
        bits.svlc(-40); // (40 - 40) & 255 = 0 -> coefficient repeat starts; cell keeps 40
        // remaining 62 cells of the 8x8 repeat 40 (no further reads).

        // t == 0, plane 1 (U): copy previous plane.
        bits.bit(1); // qm_copy_from_previous_plane
        // t == 0, plane 2 (V): copy previous plane.
        bits.bit(1); // qm_copy_from_previous_plane

        // t == 1 (TX_8X4), planes 0..3: flat, plane 1/2 copy plane 0.
        write_flat_plane(&mut bits, 8, 4); // plane 0
        bits.bit(1); // plane 1 copies plane 0
        bits.bit(1); // plane 2 copies plane 0

        // t == 2 (TX_4X8), planes 0..3: transpose of TX_8X4, then copies.
        bits.bit(1); // plane 0 transpose of 8x4
        bits.bit(1); // plane 1 copies plane 0
        bits.bit(1); // plane 2 copies plane 0

        let data = bits.into_bytes();
        let qm = parse(&data).unwrap();
        let matrices = qm.levels[0].matrices.as_ref().unwrap();
        let tx8x8 = &matrices[0];
        assert_eq!(tx8x8.planes.len(), 3);
        // Position 0 in the 8x8 is the first coefficient: 40, then coefficient repeat
        // keeps 40 for the rest.
        assert!(tx8x8.planes[0].values.iter().all(|&v| v == 40));
        // Planes 1 and 2 are exact copies of plane 0.
        assert_eq!(tx8x8.planes[1].values, tx8x8.planes[0].values);
        assert_eq!(tx8x8.planes[2].values, tx8x8.planes[0].values);
    }

    #[test]
    fn diagonal_scan_matches_av2_oracle_order() {
        // Golden up-right (TX_CLASS_2D) scan order, derived from AV2 § 5.20.7.30 and
        // cross-checked against AVM get_scan(txSz, DCT_DCT) (obu_qm.c). Positions are
        // raster indices row * width + col.
        assert_eq!(
            diagonal_scan_2d(8, 8)[..10],
            [0, 8, 1, 16, 9, 2, 24, 17, 10, 3]
        );
        assert_eq!(diagonal_scan_2d(8, 4)[..6], [0, 8, 1, 16, 9, 2]);
        assert_eq!(
            diagonal_scan_2d(4, 8)[..10],
            [0, 4, 1, 8, 5, 2, 12, 9, 6, 3]
        );
    }

    #[test]
    fn quant_delta_out_of_range_is_rejected() {
        // AV2 § 6.4.11: quant_delta must be in -128..=127. A user-defined matrix whose
        // first quant_delta is 128 must be rejected.
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // 1 plane
        bits.bit(0); // qm_is_default_flag = 0
        bits.bit(0); // qm_8x8_is_symmetric = 0
        bits.svlc(128); // out of range (> 127)
        let data = bits.into_bytes();
        assert!(matches!(
            parse(&data),
            Err(crate::error::Error::InvalidQuantizerMatrix { .. })
        ));
    }

    #[test]
    fn truncated_user_defined_qm_is_error_not_panic() {
        let mut bits = Bits::default();
        bits.f(1, 15); // qm_bit_map: level 0
        bits.bit(0); // 1 plane
        bits.bit(0); // qm_is_default_flag = 0
        bits.bit(0); // qm_8x8_is_symmetric = 0
        bits.svlc(1); // only one of 64 coefficients present, then EOF
        let data = bits.into_bytes();
        assert!(parse(&data).is_err());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The quantizer-matrix parser must never panic on arbitrary input.
        #[test]
        fn quantizer_matrix_parser_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..256),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_quantizer_matrix(&mut reader);
        }
    }
}
