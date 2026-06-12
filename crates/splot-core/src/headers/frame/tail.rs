// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AV2 § 5.18.2 frame-header **intra tail** structures
//! (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`, mirror :5307-5341).
//!
//! After `ccso_params()` (§ 5.18.7.12) the § 5.18.2 grammar reads, in order:
//!
//! ```text
//! read_tx_mode( )            // § 5.18.8.1, mirror :5307
//! frame_reference_mode( )    // § 5.18.8.3, mirror :5309
//! skip_mode_params( )        // § 5.18.8.2, mirror :5311
//! if (!FrameIsIntra && enable_bawp) allow_bawp                f(1)   // else allow_bawp = 0
//! if (!FrameIsIntra && frame_enabled_motion_modes[DELTAWARP])
//!     allow_warpmv_mode                                       f(1)   // else allow_warpmv_mode = 0
//! reduced_tx_set                                             f(2)    // mirror :5337
//! global_motion_params( )    // § 5.18.9.1, mirror :5339
//! film_grain_config( )       // § 5.18.10.1, mirror :5341
//! ```
//!
//! This module models **only the intra arm**, where every conditional that needs
//! reference-frame state collapses to a no-bit derivation determined by `FrameIsIntra`
//! and `CodedLossless` (both already known to the core parser):
//!
//! - `read_tx_mode()` reads `tx_mode_select` `f(1)` unless `CodedLossless == 1`, in
//!   which case `TxMode = ONLY_4X4` with no bits (mirror :7636).
//! - `frame_reference_mode()` infers `reference_select = 0` on the intra path with no
//!   bits (mirror :7741).
//! - `skip_mode_params()` infers `skipModeAllowed = 0` and `skip_mode_present = 0` on
//!   the intra path (`FrameIsIntra`, mirror :7673) with no bits.
//! - `allow_bawp` / `allow_warpmv_mode` are inferred `0` on the intra path (their
//!   `!FrameIsIntra` guards are false, mirror :5313 / :5327) with no bits.
//! - `reduced_tx_set` `f(2)` is read unconditionally (mirror :5337).
//! - `global_motion_params()` returns immediately on the intra path (`FrameIsIntra`,
//!   mirror :7792), reading no bits — every `GmType[ref]` is `IDENTITY` and the
//!   `gm_params` are the identity warp.
//! - `film_grain_config()` (§ 5.18.10.1) reads `apply_grain` (`f(1)`, gated), and when
//!   `apply_grain`, `fgm_id` `f(3)` and `grain_seed` `f(16)` (mirror :8163). The
//!   `load_grain_model( fgm_id )` call (mirror :8183) reads **no bits**: it is a
//!   memory-load reference to a film-grain model slot previously decoded by a
//!   `film_grain_obu()` (§ 5.14), per § 6.17.10.1 (`load_grain_model(idx)` "indicates
//!   that all syntax elements read in film_grain_model should be set equal to the values
//!   stored in an area of memory indexed by idx"). No in-band `film_grain_model()`
//!   (§ 5.18.10.2) syntax is present here, so the § 5.14
//!   [`parse_film_grain`](crate::headers::film_grain::parse_film_grain) model parser is
//!   intentionally **not** invoked from the frame-header path.
//!
//! The inter-path arms (the `tx_mode_select` always-read, `reference_select` `f(1)`,
//! `skip_mode_present` gating, `allow_bawp` / `allow_warpmv_mode` reads, and the
//! `global_motion_params()` per-reference subexp-coded warp model) are out of scope and
//! tracked as named residuals on the matrix rows.

use crate::bitio::BitReader;
use crate::error::Result;

/// `TxMode` (AV2 v1.0.0 § 5.18.8.1, mirror :7634).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TxMode {
    /// `ONLY_4X4`, inferred when `CodedLossless == 1` (no bit read).
    Only4x4,
    /// `TX_MODE_LARGEST`, when `tx_mode_select == 0`.
    Largest,
    /// `TX_MODE_SELECT`, when `tx_mode_select == 1`.
    Select,
}

impl TxMode {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Only4x4 => "only_4x4",
            Self::Largest => "tx_mode_largest",
            Self::Select => "tx_mode_select",
        }
    }
}

/// Parsed `film_grain_config()` (AV2 v1.0.0 § 5.18.10.1, mirror :8163).
///
/// Only the in-band syntax elements are modeled; `load_grain_model( fgm_id )` reads no
/// bits (§ 6.17.10.1), so no film-grain model is parsed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilmGrainConfig {
    /// `apply_grain`. Inferred `0` when `!film_grain_params_present` or neither output
    /// flag is set; inferred `1` for a single-picture header; otherwise read `f(1)`.
    pub apply_grain: bool,
    /// `fgm_id` (`f(3)`), present only when `apply_grain` (selects the loaded model
    /// slot via `load_grain_model( fgm_id )`).
    pub fgm_id: Option<u8>,
    /// `grain_seed` (`f(16)`), present only when `apply_grain`.
    pub grain_seed: Option<u16>,
}

/// Parsed § 5.18.2 intra-tail structures (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`, mirror :5307-5341).
///
/// Every field is determined by `FrameIsIntra`, `CodedLossless`, and the parsed
/// sequence/output state, so the whole tail is exactly decidable on the intra path. The
/// no-bit inferred fields (`reference_select`, `skip_mode_present`, `allow_bawp`,
/// `allow_warpmv_mode`, `use_global_motion`) are recorded as derivations, not reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeaderTail {
    /// `TxMode` from `read_tx_mode()` (§ 5.18.8.1).
    pub tx_mode: TxMode,
    /// `reference_select` from `frame_reference_mode()` (§ 5.18.8.3): always `false`
    /// (`0`) on the intra path (no bit read).
    pub reference_select: bool,
    /// `skip_mode_present` from `skip_mode_params()` (§ 5.18.8.2): always `false` (`0`)
    /// on the intra path (`skipModeAllowed == 0`, no bit read).
    pub skip_mode_present: bool,
    /// `allow_bawp` (mirror :5313): always `false` on the intra path (no bit read).
    pub allow_bawp: bool,
    /// `allow_warpmv_mode` (mirror :5327): always `false` on the intra path (no bit
    /// read).
    pub allow_warpmv_mode: bool,
    /// `reduced_tx_set` (`f(2)`, mirror :5337), always read.
    pub reduced_tx_set: u8,
    /// `use_global_motion` from `global_motion_params()` (§ 5.18.9.1): always `false`
    /// on the intra path (the structure returns before reading it, mirror :7792).
    pub use_global_motion: bool,
    /// Parsed `film_grain_config()` (§ 5.18.10.1).
    pub film_grain: FilmGrainConfig,
}

/// Sequence/frame inputs the § 5.18.2 intra tail consumes (AV2 v1.0.0 § 5.18.2).
///
/// All values are already known to the core parser when the tail is reached: the
/// derived `CodedLossless` (from `parse_lossless_info`), `film_grain_params_present`
/// (§ 5.4.1), `single_picture_header_flag` (§ 5.4.1), and the two parsed output flags
/// (`immediate_output_frame` / `implicit_output_frame`).
#[derive(Debug, Clone, Copy)]
pub struct FrameTailInput {
    /// `CodedLossless`, gating `read_tx_mode()` (§ 5.18.8.1).
    pub coded_lossless: bool,
    /// `film_grain_params_present` (§ 5.4.1), gating `film_grain_config()`'s
    /// `apply_grain` derivation.
    pub film_grain_params_present: bool,
    /// `single_picture_header_flag` (§ 5.4.1): forces `apply_grain = 1` when grain is
    /// present (mirror :8169).
    pub single_picture_header_flag: bool,
    /// `immediate_output_frame` (§ 5.18.2): part of the `apply_grain` output gate.
    pub immediate_output_frame: bool,
    /// `implicit_output_frame` (§ 5.18.2): part of the `apply_grain` output gate.
    pub implicit_output_frame: bool,
}

/// Reads `read_tx_mode()` (AV2 v1.0.0 § 5.18.8.1, mirror :7634).
///
/// `CodedLossless == 1` forces `TxMode = ONLY_4X4` with no bit read; otherwise
/// `tx_mode_select` `f(1)` selects `TX_MODE_SELECT` (`1`) or `TX_MODE_LARGEST` (`0`).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends before `tx_mode_select` can be read.
pub fn read_tx_mode(reader: &mut BitReader<'_>, coded_lossless: bool) -> Result<TxMode> {
    if coded_lossless {
        // AV2 § 5.18.8.1: CodedLossless == 1 -> TxMode = ONLY_4X4 (no bits).
        Ok(TxMode::Only4x4)
    } else {
        let tx_mode_select = reader.read_bit()? != 0; // tx_mode_select f(1)
        Ok(if tx_mode_select {
            TxMode::Select
        } else {
            TxMode::Largest
        })
    }
}

/// Parses `film_grain_config()` (AV2 v1.0.0 § 5.18.10.1, mirror :8163) on a path where
/// `apply_grain` is determined by the given inputs.
///
/// `load_grain_model( fgm_id )` reads no bits (§ 6.17.10.1), so when `apply_grain` is
/// set this reads only `fgm_id` `f(3)` and `grain_seed` `f(16)`.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends before a needed `apply_grain` / `fgm_id` / `grain_seed` field can be read.
pub fn parse_film_grain_config(
    reader: &mut BitReader<'_>,
    input: &FrameTailInput,
) -> Result<FilmGrainConfig> {
    // AV2 § 5.18.10.1 (mirror :8165): apply_grain = 0 unless grain is present AND a
    // frame that is output (immediate or implicit).
    let apply_grain = if !input.film_grain_params_present
        || (!input.immediate_output_frame && !input.implicit_output_frame)
    {
        false
    } else if input.single_picture_header_flag {
        // AV2 § 5.18.10.1 (mirror :8169): a single-picture header infers apply_grain = 1
        // with no bit read.
        true
    } else {
        reader.read_bit()? != 0 // apply_grain f(1)
    };

    let (fgm_id, grain_seed) = if apply_grain {
        // fgm_id f(3); load_grain_model( fgm_id ) reads no bits (§ 6.17.10.1).
        let fgm_id = reader.read_bits_u8(3)?;
        // grain_seed f(16); at most 65535, so `as u16` is exact.
        let grain_seed = reader.read_bits(16)? as u16;
        (Some(fgm_id), Some(grain_seed))
    } else {
        (None, None)
    };

    Ok(FilmGrainConfig {
        apply_grain,
        fgm_id,
        grain_seed,
    })
}

/// Parses the full § 5.18.2 intra tail after `ccso_params()` (AV2 v1.0.0 § 5.18.2,
/// mirror :5307-5341): `read_tx_mode()`, the no-bit `frame_reference_mode()` /
/// `skip_mode_params()` / `allow_bawp` / `allow_warpmv_mode` intra inferences,
/// `reduced_tx_set` `f(2)`, the no-bit intra arm of `global_motion_params()`, and
/// `film_grain_config()`.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the payload
/// ends partway through the tail; the caller decides whether that is a truncation
/// (facts preserved) or a hard failure. Earlier fields read before the EOF are not lost
/// because the caller reads the tail incrementally (see `parse_intra_tail_structures`).
pub fn parse_intra_tail(
    reader: &mut BitReader<'_>,
    input: &FrameTailInput,
) -> Result<FrameHeaderTail> {
    // AV2 § 5.18.8.1: read_tx_mode().
    let tx_mode = read_tx_mode(reader, input.coded_lossless)?;
    // AV2 § 5.18.8.3 / § 5.18.8.2 / mirror :5313 / :5327: intra inferences (no bits).
    let reference_select = false;
    let skip_mode_present = false;
    let allow_bawp = false;
    let allow_warpmv_mode = false;
    // AV2 § 5.18.2 (mirror :5337): reduced_tx_set f(2) is always read.
    // f(2) is at most 3, so `as u8` is exact.
    let reduced_tx_set = reader.read_bits_u8(2)?;
    // AV2 § 5.18.9.1 (mirror :7792): the intra arm returns before use_global_motion.
    let use_global_motion = false;
    // AV2 § 5.18.10.1: film_grain_config().
    let film_grain = parse_film_grain_config(reader, input)?;

    Ok(FrameHeaderTail {
        tx_mode,
        reference_select,
        skip_mode_present,
        allow_bawp,
        allow_warpmv_mode,
        reduced_tx_set,
        use_global_motion,
        film_grain,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
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

    fn reader(bytes: &[u8]) -> BitReader<'_> {
        BitReader::new(bytes, ByteOffset::new(0))
    }

    /// A representative output intra frame with grain present and not single-picture.
    fn base_input() -> FrameTailInput {
        FrameTailInput {
            coded_lossless: false,
            film_grain_params_present: true,
            single_picture_header_flag: false,
            immediate_output_frame: true,
            implicit_output_frame: false,
        }
    }

    #[test]
    fn read_tx_mode_lossless_reads_no_bits() {
        let mut r = reader(&[]);
        assert_eq!(read_tx_mode(&mut r, true).unwrap(), TxMode::Only4x4);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn read_tx_mode_select_and_largest() {
        let mut select_bits = Bits::default();
        select_bits.bit(1); // tx_mode_select = 1
        let select = select_bits.into_bytes();
        let mut r = reader(&select);
        assert_eq!(read_tx_mode(&mut r, false).unwrap(), TxMode::Select);
        assert_eq!(r.consumed_bits(), 1);

        let mut largest_bits = Bits::default();
        largest_bits.bit(0); // tx_mode_select = 0
        let largest = largest_bits.into_bytes();
        let mut r = reader(&largest);
        assert_eq!(read_tx_mode(&mut r, false).unwrap(), TxMode::Largest);
    }

    #[test]
    fn read_tx_mode_eof_is_error() {
        let mut r = reader(&[]);
        assert!(matches!(
            read_tx_mode(&mut r, false),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn film_grain_config_no_grain_present_infers_apply_grain_zero() {
        let mut input = base_input();
        input.film_grain_params_present = false;
        let mut r = reader(&[]);
        let fg = parse_film_grain_config(&mut r, &input).unwrap();
        assert!(!fg.apply_grain);
        assert_eq!(fg.fgm_id, None);
        assert_eq!(fg.grain_seed, None);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn film_grain_config_non_output_frame_infers_apply_grain_zero() {
        let mut input = base_input();
        input.immediate_output_frame = false;
        input.implicit_output_frame = false;
        let mut r = reader(&[]);
        let fg = parse_film_grain_config(&mut r, &input).unwrap();
        assert!(!fg.apply_grain);
        assert_eq!(r.consumed_bits(), 0);
    }

    #[test]
    fn film_grain_config_single_picture_infers_apply_grain_one_then_reads_id_and_seed() {
        let mut input = base_input();
        input.single_picture_header_flag = true;
        // apply_grain inferred 1 (no bit); then fgm_id f(3) + grain_seed f(16).
        let mut bits = Bits::default();
        bits.f(5, 3); // fgm_id = 5
        bits.f(0xBEEF, 16); // grain_seed = 0xBEEF
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let fg = parse_film_grain_config(&mut r, &input).unwrap();
        assert!(fg.apply_grain);
        assert_eq!(fg.fgm_id, Some(5));
        assert_eq!(fg.grain_seed, Some(0xBEEF));
        assert_eq!(r.consumed_bits(), 19); // 3 + 16, no apply_grain bit
    }

    #[test]
    fn film_grain_config_reads_apply_grain_bit_then_fields() {
        let input = base_input(); // present, output, not single-picture -> apply_grain f(1)
        let mut bits = Bits::default();
        bits.bit(1); // apply_grain = 1
        bits.f(2, 3); // fgm_id = 2
        bits.f(0x1234, 16); // grain_seed
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let fg = parse_film_grain_config(&mut r, &input).unwrap();
        assert!(fg.apply_grain);
        assert_eq!(fg.fgm_id, Some(2));
        assert_eq!(fg.grain_seed, Some(0x1234));
        assert_eq!(r.consumed_bits(), 20); // 1 + 3 + 16
    }

    #[test]
    fn film_grain_config_apply_grain_zero_when_bit_clear() {
        let input = base_input();
        let mut bits = Bits::default();
        bits.bit(0); // apply_grain = 0
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let fg = parse_film_grain_config(&mut r, &input).unwrap();
        assert!(!fg.apply_grain);
        assert_eq!(fg.fgm_id, None);
        assert_eq!(r.consumed_bits(), 1);
    }

    #[test]
    fn film_grain_config_eof_inside_seed_is_error() {
        let input = base_input();
        let mut bits = Bits::default();
        bits.bit(1); // apply_grain = 1
        bits.f(0, 3); // fgm_id
        bits.f(0, 8); // only 8 of 16 grain_seed bits
        let data = bits.into_bytes();
        let mut r = reader(&data);
        assert!(matches!(
            parse_film_grain_config(&mut r, &input),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn parse_intra_tail_full_lossy_with_grain() {
        let input = base_input();
        let mut bits = Bits::default();
        bits.bit(1); // tx_mode_select = 1 -> Select
        bits.f(2, 2); // reduced_tx_set = 2
        bits.bit(1); // apply_grain = 1
        bits.f(7, 3); // fgm_id = 7
        bits.f(0xABCD, 16); // grain_seed
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let tail = parse_intra_tail(&mut r, &input).unwrap();
        assert_eq!(tail.tx_mode, TxMode::Select);
        assert!(!tail.reference_select);
        assert!(!tail.skip_mode_present);
        assert!(!tail.allow_bawp);
        assert!(!tail.allow_warpmv_mode);
        assert_eq!(tail.reduced_tx_set, 2);
        assert!(!tail.use_global_motion);
        assert!(tail.film_grain.apply_grain);
        assert_eq!(tail.film_grain.fgm_id, Some(7));
        assert_eq!(tail.film_grain.grain_seed, Some(0xABCD));
        // 1 (tx) + 2 (reduced_tx_set) + 1 (apply_grain) + 3 + 16.
        assert_eq!(r.consumed_bits(), 23);
    }

    #[test]
    fn parse_intra_tail_lossless_skips_tx_bit_and_no_grain() {
        let mut input = base_input();
        input.coded_lossless = true;
        input.film_grain_params_present = false;
        let mut bits = Bits::default();
        bits.f(0, 2); // reduced_tx_set = 0
        let data = bits.into_bytes();
        let mut r = reader(&data);
        let tail = parse_intra_tail(&mut r, &input).unwrap();
        assert_eq!(tail.tx_mode, TxMode::Only4x4);
        assert_eq!(tail.reduced_tx_set, 0);
        assert!(!tail.film_grain.apply_grain);
        // No tx bit (lossless), no apply_grain bit (no grain): just reduced_tx_set f(2).
        assert_eq!(r.consumed_bits(), 2);
    }

    #[test]
    fn parse_intra_tail_eof_at_tx_mode_select_is_error() {
        // Lossy frame: read_tx_mode() reads tx_mode_select f(1) first. An empty payload
        // makes that first read overrun.
        let input = base_input(); // coded_lossless == false
        let mut empty = reader(&[]);
        assert!(matches!(
            parse_intra_tail(&mut empty, &input),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn parse_intra_tail_eof_at_reduced_tx_set_is_error() {
        // CodedLossless == 1 -> read_tx_mode() reads no bit, so reduced_tx_set f(2) is the
        // first tail read. An empty payload makes it overrun immediately.
        let mut input = base_input();
        input.coded_lossless = true;
        let mut empty = reader(&[]);
        assert!(matches!(
            parse_intra_tail(&mut empty, &input),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    proptest! {
        /// The intra-tail parser must never panic on arbitrary input.
        #[test]
        fn parse_intra_tail_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..32),
            coded_lossless in any::<bool>(),
            film_grain_params_present in any::<bool>(),
            single_picture_header_flag in any::<bool>(),
            immediate_output_frame in any::<bool>(),
            implicit_output_frame in any::<bool>(),
        ) {
            let input = FrameTailInput {
                coded_lossless,
                film_grain_params_present,
                single_picture_header_flag,
                immediate_output_frame,
                implicit_output_frame,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_intra_tail(&mut reader, &input);
        }
    }
}
