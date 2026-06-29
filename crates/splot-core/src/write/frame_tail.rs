// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 frame-header **intra-tail** writers (`ENC-BITSTREAM-WRITER`) — the inverses of the
//! § 5.18.8.1 / § 5.18.10.1 / § 5.18.2 intra-tail parsers in [`crate::headers::frame`]:
//!
//! - [`write_tx_mode`] — `read_tx_mode()` (§ 5.18.8.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-8-1`): `ONLY_4X4` inferred for a
//!   lossless frame (no bit), else `tx_mode_select` `f(1)`.
//! - [`write_film_grain_config`] — `film_grain_config()` (§ 5.18.10.1,
//!   `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-1`): the gated `apply_grain` and,
//!   when set, `fgm_id` `f(3)` and `grain_seed` `f(16)`.
//! - [`write_intra_tail`] — the § 5.18.2 intra tail
//!   (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`): `read_tx_mode()`, the no-bit
//!   intra inferences (`reference_select` / `skip_mode_present` / `allow_bawp` /
//!   `allow_warpmv_mode` / `use_global_motion`), `reduced_tx_set` `f(2)`, and
//!   `film_grain_config()`.
//!
//! Like the other frame-header config writers, this module is additive: it depends on the
//! model/parser read-only and serializes a parsed structure back to bits via [`BitWriter`].
//! Each writer threads the same gating inputs the parser receives (`coded_lossless`, the
//! [`FrameTailInput`] sequence/output state) and validates the whole structure before any bit
//! is written (reject-before-write): every reject path leaves `writer.bit_len() == 0`. The
//! inferred no-bit fields are re-derived and rejected on mismatch, never coded.

use crate::headers::frame::{FilmGrainConfig, FrameHeaderTail, FrameTailInput, TxMode};
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `fgm_id` is `f(3)` (AV2 v1.0.0 § 5.18.10.1), so it fits `0..8`.
const FGM_ID_MAX_PLUS_1: u8 = 8;
/// `reduced_tx_set` is `f(2)` (AV2 v1.0.0 § 5.18.2), so it fits `0..4`.
const REDUCED_TX_SET_MAX_PLUS_1: u8 = 4;

/// Writes `read_tx_mode()` (AV2 v1.0.0 § 5.18.8.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-8-1`), the inverse of
/// [`crate::headers::frame::read_tx_mode`].
///
/// When `coded_lossless` the parser infers `ONLY_4X4` with no bit, so the model must be
/// `ONLY_4X4`; otherwise `tx_mode_select` `f(1)` is written (`TX_MODE_SELECT` -> `1`,
/// `TX_MODE_LARGEST` -> `0`) and the model must be one of those two (the inferred `ONLY_4X4`
/// is unreachable when not lossless). Validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`] (`tx_mode`) if a `coded_lossless` model is not
/// `ONLY_4X4`, or a non-`coded_lossless` model is `ONLY_4X4`.
pub fn write_tx_mode(
    writer: &mut BitWriter,
    tx_mode: TxMode,
    coded_lossless: bool,
) -> WriteResult<()> {
    if coded_lossless {
        if tx_mode != TxMode::Only4x4 {
            return Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" });
        }
        return Ok(());
    }
    let bit = match tx_mode {
        TxMode::Largest => 0,
        TxMode::Select => 1,
        TxMode::Only4x4 => {
            return Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" });
        }
    };
    writer.write_bit(bit)
}

/// Writes `film_grain_config()` (AV2 v1.0.0 § 5.18.10.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-1`), the inverse of
/// [`crate::headers::frame::parse_film_grain_config`].
///
/// `apply_grain` is inferred `0` when grain is absent or the frame is not output, inferred `1`
/// for a single-picture header, else written `f(1)`; when set, `fgm_id` `f(3)` and `grain_seed`
/// `f(16)` follow. Validated before any bit is written.
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`]: a model whose `apply_grain` disagrees with an
/// inferred value (`film_grain_apply_grain`); an `apply_grain` whose `fgm_id` / `grain_seed`
/// presence is wrong (`film_grain_fields`); or an `fgm_id` outside its `f(3)` field
/// (`film_grain_fgm_id`).
pub fn write_film_grain_config(
    writer: &mut BitWriter,
    fg: &FilmGrainConfig,
    input: &FrameTailInput,
) -> WriteResult<()> {
    check_film_grain_encodable(*fg, *input)?;

    let grain_gated_off = !input.film_grain_params_present
        || (!input.immediate_output_frame && !input.implicit_output_frame);
    if !grain_gated_off && !input.single_picture_header_flag {
        writer.write_flag(fg.apply_grain)?;
    }

    if fg.apply_grain {
        if let Some(fgm_id) = fg.fgm_id {
            writer.write_bits_u8(fgm_id, 3)?;
        }
        if let Some(grain_seed) = fg.grain_seed {
            writer.write_bits(u32::from(grain_seed), 16)?;
        }
    }
    Ok(())
}

/// Validates a [`FilmGrainConfig`] is a model the § 5.18.10.1
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-1`) parser could have produced for
/// `input`, before any bit is written.
fn check_film_grain_encodable(fg: FilmGrainConfig, input: FrameTailInput) -> WriteResult<()> {
    let grain_gated_off = !input.film_grain_params_present
        || (!input.immediate_output_frame && !input.implicit_output_frame);
    if grain_gated_off {
        if fg.apply_grain {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_apply_grain",
            });
        }
    } else if input.single_picture_header_flag && !fg.apply_grain {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "film_grain_apply_grain",
        });
    }

    if fg.apply_grain {
        let Some(fgm_id) = fg.fgm_id else {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fields",
            });
        };
        if fg.grain_seed.is_none() {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fields",
            });
        }
        if fgm_id >= FGM_ID_MAX_PLUS_1 {
            return Err(WriteError::NonCanonicalFrameHeader {
                what: "film_grain_fgm_id",
            });
        }
    } else if fg.fgm_id.is_some() || fg.grain_seed.is_some() {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "film_grain_fields",
        });
    }
    Ok(())
}

/// Writes the § 5.18.2 intra tail (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`), the inverse of
/// [`crate::headers::frame::parse_intra_tail`]: `read_tx_mode()`, the no-bit intra inferences,
/// `reduced_tx_set` `f(2)`, and `film_grain_config()`.
///
/// The no-bit fields (`reference_select`, `skip_mode_present`, `allow_bawp`, `allow_warpmv_mode`,
/// `use_global_motion`) are all inferred `false` on the intra path; a model carrying a `true`
/// for any of them could not have been produced and is rejected. Validated before any bit.
///
/// # Errors
/// [`WriteError::NonCanonicalFrameHeader`]: a non-`false` intra inference (`intra_tail_inference`);
/// a `reduced_tx_set` outside its `f(2)` field (`reduced_tx_set`); or any
/// [`write_tx_mode`] / [`write_film_grain_config`] reject.
pub fn write_intra_tail(
    writer: &mut BitWriter,
    tail: &FrameHeaderTail,
    input: &FrameTailInput,
) -> WriteResult<()> {
    check_intra_tail_encodable(tail, *input)?;

    write_tx_mode(writer, tail.tx_mode, input.coded_lossless)?;
    writer.write_bits_u8(tail.reduced_tx_set, 2)?;
    write_film_grain_config(writer, &tail.film_grain, input)?;
    Ok(())
}

/// Validates a [`FrameHeaderTail`] is a model the § 5.18.2
/// (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) intra-tail parser could have
/// produced for `input`, before any bit is written.
fn check_intra_tail_encodable(tail: &FrameHeaderTail, input: FrameTailInput) -> WriteResult<()> {
    if tail.reference_select
        || tail.skip_mode_present
        || tail.allow_bawp
        || tail.allow_warpmv_mode
        || tail.use_global_motion
    {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "intra_tail_inference",
        });
    }
    if tail.reduced_tx_set >= REDUCED_TX_SET_MAX_PLUS_1 {
        return Err(WriteError::NonCanonicalFrameHeader {
            what: "reduced_tx_set",
        });
    }
    if input.coded_lossless && tail.tx_mode != TxMode::Only4x4 {
        return Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" });
    }
    if !input.coded_lossless && tail.tx_mode == TxMode::Only4x4 {
        return Err(WriteError::NonCanonicalFrameHeader { what: "tx_mode" });
    }
    check_film_grain_encodable(tail.film_grain, input)?;
    Ok(())
}

#[cfg(test)]
include!("frame_tail_tests.rs");
#[cfg(test)]
include!("frame_tail_proptests.rs");
