// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Encoder block-symbol trace composers for the **general** AV2 intra decode path.
//!
//! The minimal-tier composers in `block_symbol_trace` model the frozen single-block
//! acceptor; these model the symbol stream the AVM-validated general intra decode
//! path reads for one undivided 64x64 superblock, which leads with the § 5.20.3.2
//! `do_split` partition flag and codes the `txb_skip` symbols at the 64x64-leaf
//! transform contexts. The composed traces are driven through the shared § 8.2
//! coder by `block_symbol_trace::roundtrip_block_symbol_trace`.

// The encoder runtime does not yet consume these composers (it returns
// `NeedMoreData`); they are exercised by the block-symbol-trace roundtrip tests,
// matching the sibling emission modules' policy.
#![allow(dead_code)]

use crate::block_symbol_trace::{
    BlockSymbolToken, compose_minimal_intra_dc_block_mode_trace, encode_block_symbol_trace,
};
use crate::coefficient_tokenization::{
    chroma_v_all_zero_token, general_intra_32x32_chroma_u_all_zero_token,
    general_intra_32x32_chroma_u_dc_coded_tokens, general_intra_32x32_chroma_v_dc_coded_tokens,
    general_intra_64x64_luma_all_zero_token, general_intra_64x64_luma_dc_coded_tokens,
    general_intra_64x64_luma_two_coeff_tokens, general_intra_64x64_luma_two_nonzero_base_tokens,
    general_intra_64x64_luma_visible_ac_tokens, luma_dc_sign_token,
};
use crate::error::{Error, Result};
use crate::partition_emission::emit_root_do_split_none;

/// The coefficient CDF q-context for a skip frame whose `base_q_idx <= 90`:
/// `coeff_cdf_q_ctx_from_base_q_idx` bank `0` (the same bank the AVM-validated
/// `syn-flat-intra-64x64-q80` fixture's `base_q_idx == 80` selects).
const SKIP_FRAME_COEFF_CDF_Q_CTX: usize = 0;

/// The § 8.3.2 neutral V `txb_skip` context: `0`. For these frames the chroma block equals its
/// transform and U is all-zero (`EobU == 0`), so neither the `+3` nor the `+6` term applies.
const V_TXB_SKIP_CTX_NEUTRAL: usize = 0;

/// Composes the complete ordered general intra DC skip-block trace read on the AV2
/// general intra decode path for one undivided 64x64 superblock: the § 5.20.3.2
/// `do_split == false` (`PARTITION_NONE`) flag, the § 5.20.5.3 mode-info prefix
/// (`y_mode_set`, `y_mode_index`, `uv_mode`, all `0` for DC), then the per-plane
/// § 5.20.7.27 `all_zero` (`txb_skip`) symbols (`1` each) in `residual()` order
/// Y, U, V.
///
/// Unlike `block_symbol_trace::compose_minimal_intra_dc_complete_all_zero_block_trace`
/// it leads with `do_split` and codes the luma/U `txb_skip` at the `TX_64X64` /
/// `TX_32X32` `txSzCtx` of a 64x64 4:2:0 leaf rather than the minimal `TX_4X4` ctx;
/// the V `txb_skip` keeps `ctx 0`. The coefficient CDF q-context is `0`.
pub(crate) fn compose_general_intra_dc_skip_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let total = modes
        .len()
        .checked_add(4)
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general skip block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general skip block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// The unsigned luma DC magnitude the coded frame emits: `6` (`coeff_base_eob` saturated at
/// `4`, level 5, plus `coeff_br == 1`). Negative, it reconstructs flat luma `127` at
/// `base_q_idx == 80`. It is the largest magnitude **below the § 5.20.7.28 golomb threshold** on
/// this frame: the minimal header's luma uses TCQ, so `read_quant` reads a golomb tail once
/// `quant >= maxLevel - allowTcq == 7`. Magnitude `7` (the q80 luma level, reconstructing `100`)
/// would need that tail — a follow-up brick.
const CODED_LUMA_DC_MAGNITUDE: u32 = 6;

/// Composes the general intra DC coded-block trace: like
/// [`compose_general_intra_dc_skip_block_trace`] but the luma block carries one coded DC
/// coefficient of unsigned `magnitude` and the given sign (`txb_skip == 0`, `eob_pt == 0`,
/// `coeff_base_eob`, optional `coeff_br`, `dc_sign`) at the general `TX_64X64` contexts,
/// while U and V stay skipped. The V `txb_skip` keeps the neutral `ctx 0` (`EobU == 0`,
/// since U is skipped).
fn compose_general_intra_dc_coded_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma =
        general_intra_64x64_luma_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, magnitude, negative)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(3)) // do_split + U + V txb_skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Encodes the general intra DC skip-block trace into its AV2 § 8.2.4-finalized
/// `tile_data` bytes — the entropy-coded payload of a single-tile general intra
/// frame, which the decoder consumes directly from byte 0 via § 8.2.2
/// `init_symbol` with no structural prefix (a single last tile carries no
/// `tile_size_minus_1` field).
///
/// These are the § 5.20 `tile_data` bytes for one 64x64 DC skip (all-zero)
/// superblock. For the decoder to read them back identically, the muxing frame
/// header (a later brick) must set `base_q_idx <= 90` (so the decoder derives
/// coefficient CDF q-context `0`, matching [`SKIP_FRAME_COEFF_CDF_Q_CTX`]) and
/// `disable_cdf_update == 0` (so the tile reader's adaptive CDFs match the
/// `CdfUpdateMode::Enabled` this trace is coded under). This function emits only
/// the tile bytes; container assembly and the cross-crate decode oracle are later
/// bricks.
pub(crate) fn encode_general_intra_dc_skip_tile_data() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_skip_block_trace()?;
    encode_block_symbol_trace(&trace)
}

/// The `base_q_idx` the minimal intra skip frame is muxed at: 80, the AVM- and
/// dav2d-validated `syn-flat-intra-64x64-q80` fixture's value. It is `<= 90`, so the decoder
/// derives coefficient CDF q-context `0` — the q-context [`encode_general_intra_dc_skip_tile_data`]
/// codes its `txb_skip` symbols under.
const SKIP_FRAME_BASE_Q_IDX: u8 = 80;

/// Emits a complete, decodable AV2 IVF stream: one 64x64 all-intra `OBU_CLOSED_LOOP_KEY` frame
/// whose single tile is a DC skip (all-zero residual) block. This is `splot-encode`'s first
/// end-to-end decodable output.
///
/// It pairs the general-intra DC skip `tile_data` (`encode_general_intra_dc_skip_tile_data`)
/// with the `base_q_idx`-80 container
/// ([`splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx`]) whose
/// decoder-derived coefficient CDF q-context (`0`) matches the tile's coding. Decoding the
/// stream reconstructs a flat frame: every luma and chroma sample is the § 7.13.2 DC
/// prediction of a no-neighbour block — `128` for 8-bit — because the block is skipped.
///
/// The container is byte-identical to the `syn-flat-intra-64x64-q80` fixture apart from the
/// `tile_data`; only the tile bytes encode the skip block. The cross-crate decode that proves
/// the flat reconstruction lives in `splot-cli` (which depends on both `splot-encode` and
/// `splot-decode`).
pub fn emit_minimal_intra_skip_ivf() -> Result<Vec<u8>> {
    let tile_data = encode_general_intra_dc_skip_tile_data()?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries a single **coded** DC coefficient (U and V skipped).
///
/// Unlike [`emit_minimal_intra_skip_ivf`] (all-zero residual → flat 128), this emits real
/// residual: decoding reconstructs a flat luma plane of `127` (`128` minus the dequantized
/// negative DC of magnitude `CODED_LUMA_DC_MAGNITUDE`) and flat `128` chroma. It is the
/// encoder's first decodable output carrying a coded coefficient. The cross-crate decode
/// oracle that proves the reconstruction lives in `splot-cli`.
pub fn emit_minimal_intra_coded_dc_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true)?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// The unsigned chroma U DC magnitude the coded-chroma frame emits: `4` (`coeff_base_eob` 3,
/// level 4 — the base tier, no `coeff_br`/golomb). Negative, it dequantizes to a flat U
/// reconstruction below `128`.
const CODED_CHROMA_U_DC_MAGNITUDE: u32 = 4;

/// The § 8.3.2 V `txb_skip` context when the U plane is coded (`EobU != 0`): `6`.
const V_TXB_SKIP_CTX_EOBU: usize = 6;

/// A chroma DC `sign_bit` is a § 8.2.5 `L(1)` bypass literal.
const CHROMA_SIGN_BIT_WIDTH: u32 = 1;

/// Composes the general intra coded-*chroma* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, then a single coded U DC coefficient (`txb_skip == 0`,
/// `eob_pt == 0`, `coeff_base_eob`, then the U DC `sign_bit` § 8.2.5 bypass literal) at the
/// general `TX_32X32` chroma contexts, then a skipped V plane whose `txb_skip` uses the
/// `EobU != 0` context `6`. The decoded frame isolates chroma residual: luma stays flat 128,
/// U carries the dequantized residual, V stays flat 128.
fn compose_general_intra_coded_chroma_u_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let u_coeffs =
        general_intra_32x32_chroma_u_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, magnitude)?;
    let total = modes
        .len()
        .checked_add(u_coeffs.len())
        .and_then(|n| n.checked_add(4)) // do_split + luma skip + U sign + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_EOBU,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **chroma U** block carries a single negative coded DC coefficient (luma and V
/// skipped).
///
/// This proves chroma residual reconstruction: decoding reconstructs a flat luma plane of
/// `128` (skipped), a flat U plane below `128` (the dequantized negative chroma DC), and a
/// flat V plane of `128` (skipped). The cross-crate decode oracle lives in `splot-cli`.
pub fn emit_minimal_intra_coded_chroma_ivf() -> Result<Vec<u8>> {
    let trace =
        compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Composes the general intra coded-*chroma-V* block trace: `do_split`, the mode prefix, a
/// **skipped** luma plane, a **skipped** U plane, then a single coded V DC coefficient
/// (`VTxbSkip == 0`, `eob_pt == 0`, `coeff_base_eob`, then the V DC `sign_bit` § 8.2.5 bypass
/// literal) at the general `TX_32X32` chroma contexts. The V `txb_skip` uses the neutral
/// context `0` (`EobU == 0`, since U is skipped). The decoded frame isolates V residual: luma
/// and U stay flat 128, V carries the dequantized residual.
fn compose_general_intra_coded_chroma_v_block_trace(
    magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let v_coeffs = general_intra_32x32_chroma_v_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        magnitude,
        V_TXB_SKIP_CTX_NEUTRAL,
    )?;
    let total = modes
        .len()
        .checked_add(v_coeffs.len())
        .and_then(|n| n.checked_add(4)) // do_split + luma skip + U skip + V sign
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma V block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general coded chroma V block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_64x64_luma_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **chroma V** block carries a single negative coded DC coefficient (luma and U
/// skipped).
///
/// The V-plane counterpart of [`emit_minimal_intra_coded_chroma_ivf`]: decoding reconstructs a
/// flat luma plane of `128` (skipped), a flat U plane of `128` (skipped), and a flat V plane
/// below `128` (the dequantized negative chroma DC). With the U and V coded frames this
/// completes the per-plane coded-residual set. The cross-crate decode oracle lives in
/// `splot-cli`.
pub fn emit_minimal_intra_coded_chroma_v_ivf() -> Result<Vec<u8>> {
    let trace =
        compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Composes the general intra all-planes-coded block trace: `do_split`, the mode prefix, then
/// a single coded DC coefficient on **each** of luma (`TX_64X64`, CDF `dc_sign`), U, and V
/// (`TX_32X32`, `sign_bit` § 8.2.5 bypass), in `residual()` order Y, U, V. Because the U plane
/// is coded (`EobU != 0`), the V `txb_skip` uses the § 8.3.2 context `6`. This mirrors the q80
/// fixture's all-three-planes-coded structure with sub-golomb magnitudes.
fn compose_general_intra_all_planes_coded_block_trace(
    luma_magnitude: u32,
    chroma_magnitude: u32,
    negative: bool,
) -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        luma_magnitude,
        negative,
    )?;
    let u_coeffs =
        general_intra_32x32_chroma_u_dc_coded_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX, chroma_magnitude)?;
    let v_coeffs = general_intra_32x32_chroma_v_dc_coded_tokens(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        chroma_magnitude,
        V_TXB_SKIP_CTX_EOBU,
    )?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(u_coeffs.len()))
        .and_then(|n| n.checked_add(v_coeffs.len()))
        .and_then(|n| n.checked_add(3)) // do_split + U sign + V sign
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general all-planes coded block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general all-planes coded block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.extend(u_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    trace.extend(v_coeffs.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(
        CHROMA_SIGN_BIT_WIDTH,
        negative as u32,
    ));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose **luma, U, and V** blocks each carry a single negative coded DC coefficient.
///
/// This is the encoder's first frame with all three planes coded at once, mirroring the q80
/// fixture's structure: decoding reconstructs flat planes below `128` on every plane. The
/// cross-crate decode oracle lives in `splot-cli`.
pub fn emit_minimal_intra_all_planes_coded_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_all_planes_coded_block_trace(
        CODED_LUMA_DC_MAGNITUDE,
        CODED_CHROMA_U_DC_MAGNITUDE,
        true,
    )?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Composes the general intra eob=2 multi-coefficient luma block trace: `do_split`, the mode
/// prefix, a coded luma block carrying a single nonzero AC coefficient (level 1) at scan index
/// 1 with a zero DC (`txb_skip == 0`, `eob_pt_1024 == 1`, AC `coeff_base_eob`, DC `coeff_base`,
/// then the AC `sign_bit` § 8.2.5 bypass), then skipped U and V. The minimal level-1 AC residual
/// is sub-visible, so the reconstruction is flat 128 (see `emit_minimal_intra_two_coeff_ivf`).
fn compose_general_intra_two_coeff_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_two_coeff_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(4)) // do_split + AC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general two-coefficient block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general two-coefficient block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // The AC `sign_bit` is a § 8.2.5 bypass literal (positive); the zero DC has no sign.
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries a single nonzero **AC** coefficient (eob=2, U and V skipped).
///
/// This is the encoder's first multi-coefficient (`eob > 1`) frame, exercising the scan walk
/// and the non-EOB `coeff_base` pass. The single AC coefficient is the minimal **level 1**,
/// whose dequantized residual is sub-visible (it rounds to ~0), so the decoded frame is flat at
/// 128 — distinct from a skip frame only in the entropy stream, not yet the reconstruction. A
/// visibly non-flat (cosine) AC needs a larger magnitude, whose `Level[]`-derived DC
/// `coeff_base` context differs and is computed per AC level; that is a follow-up. The
/// cross-crate decode oracle (it validates the eob=2 stream and reconstructs the frame) lives in
/// `splot-cli`.
pub fn emit_minimal_intra_two_coeff_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_two_coeff_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// Composes the general intra eob=2 **visible** multi-coefficient luma block trace: `do_split`,
/// the mode prefix, a coded luma block carrying a single nonzero AC coefficient of level 4 at
/// scan index 1 with a zero DC, then the AC `sign_bit` § 8.2.5 bypass, then skipped U and V.
/// Unlike the minimal level-1 AC (which rounds back to flat 128), the level-4 AC dequantizes to
/// a residual that reconstructs a visibly non-flat (low-frequency cosine) luma plane.
fn compose_general_intra_visible_ac_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_visible_ac_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(4)) // do_split + AC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general visible-AC block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general visible-AC block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries a single nonzero **level-4 AC** coefficient (eob=2, U and V
/// skipped), reconstructing a **visibly non-flat** low-frequency cosine luma plane.
///
/// This is the encoder's first frame where a coefficient visibly shapes the reconstruction
/// (every prior frame was flat). It builds on `emit_minimal_intra_two_coeff_ivf` (the level-1
/// AC, sub-visible) by raising the AC to level 4 — the largest `coeff_base_eob` base level with
/// no `coeff_br` tail — so the dequantized residual survives rounding. The cross-crate decode
/// oracle lives in `splot-cli`.
pub fn emit_minimal_intra_visible_ac_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_visible_ac_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

/// The DC sign for the two-nonzero-coefficient block: **negative** (`dc_sign == 1`). A negative
/// DC makes the oracle genuinely exercise the AV2 § 5.20.7.27 reverse-scan sign order: with both
/// signs positive the block reconstructs identically under either sign order, so an ordering bug
/// would pass undetected. With the DC negative and the AC positive, only the spec-correct order
/// (AC `sign_bit` before DC `dc_sign`) reconstructs consistently.
const TWO_NONZERO_DC_NEGATIVE: bool = true;

/// Composes the general intra eob=2 **two-nonzero-coefficient** luma block trace: `do_split`,
/// the mode prefix, the coded luma base pass (a level-4 AC at scan index 1 and a level-1 DC at
/// scan index 0), then the sign pass in AV2 § 5.20.7.27 order `c = eob-1 .. 0` (reverse scan):
/// the AC `sign_bit` § 8.2.5 bypass (c=1) FIRST, then the DC `dc_sign` CDF symbol (c=0). U and V
/// are skipped. This is the first block where two coefficients are nonzero; the reconstruction is
/// the visible-AC vertical cosine superimposed on the negative DC offset.
fn compose_general_intra_two_nonzero_block_trace() -> Result<Vec<BlockSymbolToken>> {
    let modes = compose_minimal_intra_dc_block_mode_trace()?;
    let luma = general_intra_64x64_luma_two_nonzero_base_tokens(SKIP_FRAME_COEFF_CDF_Q_CTX)?;
    let total = modes
        .len()
        .checked_add(luma.len())
        .and_then(|n| n.checked_add(5)) // do_split + AC sign + DC sign + U skip + V skip
        .ok_or(Error::BlockSymbolTraceAllocationFailed {
            context: "general two-nonzero block trace length",
        })?;
    let mut trace = Vec::new();
    trace
        .try_reserve_exact(total)
        .map_err(|_| Error::BlockSymbolTraceAllocationFailed {
            context: "general two-nonzero block trace",
        })?;
    trace.push(BlockSymbolToken::Partition(emit_root_do_split_none()));
    trace.extend(modes.into_iter().map(BlockSymbolToken::Mode));
    trace.extend(luma.into_iter().map(BlockSymbolToken::Coeff));
    // Sign pass, reverse scan (`c = eob-1 .. 0`): the AC `sign_bit` § 8.2.5 bypass (c=1) is
    // emitted FIRST, then the DC `dc_sign` CDF symbol (c=0).
    trace.push(BlockSymbolToken::bypass(CHROMA_SIGN_BIT_WIDTH, 0));
    trace.push(BlockSymbolToken::Coeff(luma_dc_sign_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        TWO_NONZERO_DC_NEGATIVE,
    )));
    trace.push(BlockSymbolToken::Coeff(
        general_intra_32x32_chroma_u_all_zero_token(SKIP_FRAME_COEFF_CDF_Q_CTX),
    ));
    trace.push(BlockSymbolToken::Coeff(chroma_v_all_zero_token(
        SKIP_FRAME_COEFF_CDF_Q_CTX,
        V_TXB_SKIP_CTX_NEUTRAL,
    )));
    Ok(trace)
}

/// Emits a complete, decodable AV2 IVF stream for one 64x64 all-intra `OBU_CLOSED_LOOP_KEY`
/// frame whose luma block carries **two nonzero coefficients** — a positive level-4 AC at scan
/// index 1 and a **negative** level-1 DC at scan index 0 (eob=2, U and V skipped). This is the
/// encoder's first block with more than one nonzero coefficient; it exercises the AV2 § 5.20.7.27
/// reverse-scan sign pass (AC `sign_bit` before DC `dc_sign`). The reconstruction is the
/// visible-AC vertical cosine superimposed on the negative DC offset. The cross-crate decode
/// oracle lives in `splot-cli`.
pub fn emit_minimal_intra_two_nonzero_ivf() -> Result<Vec<u8>> {
    let trace = compose_general_intra_two_nonzero_block_trace()?;
    let tile_data = encode_block_symbol_trace(&trace)?;
    splot_core::headers::frame::encode_minimal_intra_clk_ivf_with_base_q_idx(
        SKIP_FRAME_BASE_Q_IDX,
        &tile_data,
    )
    .map_err(|source| Error::MinimalIntraSkipIvf { source })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::block_symbol_trace::roundtrip_block_symbol_trace;

    #[test]
    fn composes_general_skip_block_trace_in_order() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();

        assert_eq!(trace.len(), 7);
        // do_split partition flag, mode prefix (Y set/index, UV), then per-plane
        // all_zero (Y, U, V) in residual() order.
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(trace[1], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[2], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[3], BlockSymbolToken::Mode(_)));
        assert!(matches!(trace[4], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[5], BlockSymbolToken::Coeff(_)));
        assert!(matches!(trace[6], BlockSymbolToken::Coeff(_)));
        // do_split=0, y_mode_set=0, y_mode_index=0, uv_mode=0, then luma/U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn general_skip_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 1]);
        assert_eq!(proof.symbol_count(), 7);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn general_skip_block_roundtrip_is_deterministic() {
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let first = roundtrip_block_symbol_trace(&trace).unwrap();
        let second = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(first.decoded_symbols(), second.decoded_symbols());
    }

    #[test]
    fn skip_tile_data_is_nonempty_and_equals_the_proven_trace_bytes() {
        let tile_data = encode_general_intra_dc_skip_tile_data().unwrap();
        // A zero-size tile is a §8.2.2 defect; the §8.2.4 finalization always
        // emits at least the exit-window bytes.
        assert!(!tile_data.is_empty());

        // The emitted tile_data IS the finalized byte stream of the proven skip
        // trace — identical to the bytes the roundtrip decodes back, so the
        // standalone emission inherits that decodability proof.
        let trace = compose_general_intra_dc_skip_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(tile_data, proof.bytes());
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn skip_tile_data_is_deterministic() {
        assert_eq!(
            encode_general_intra_dc_skip_tile_data().unwrap(),
            encode_general_intra_dc_skip_tile_data().unwrap()
        );
    }

    #[test]
    fn emit_minimal_intra_skip_ivf_is_a_parseable_one_frame_av02_ivf() {
        let ivf = emit_minimal_intra_skip_ivf().unwrap();
        assert!(!ivf.is_empty());

        // Structurally a single-frame AV02 64x64 IVF; the full decode-to-flat-128 proof is
        // the cross-crate oracle in splot-cli.
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        let header = parsed.header.unwrap();
        assert_eq!(&header.fourcc, b"AV02");
        assert_eq!((header.width, header.height), (64, 64));
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn emit_minimal_intra_skip_ivf_is_deterministic() {
        assert_eq!(
            emit_minimal_intra_skip_ivf().unwrap(),
            emit_minimal_intra_skip_ivf().unwrap()
        );
    }

    #[test]
    fn composes_general_coded_block_trace_in_order() {
        // Magnitude 6: coeff_base_eob saturates at 4 (level 5) plus coeff_br == 1.
        let trace =
            compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true).unwrap();

        // do_split, 3 mode symbols, 5 coded-luma symbols (txb_skip, eob_pt, coeff_base_eob,
        // coeff_br, dc_sign), then U and V txb_skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        for token in &trace[1..4] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        for token in &trace[4..11] {
            assert!(matches!(token, BlockSymbolToken::Coeff(_)));
        }
        // do_split=0, modes 0/0/0, luma txb_skip=0, eob_pt=0, coeff_base_eob=4, coeff_br=1,
        // dc_sign=1 (negative), then U/V all_zero=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 0, 4, 1, 1, 1, 1]
        );
    }

    #[test]
    fn general_coded_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_dc_coded_block_trace(CODED_LUMA_DC_MAGNITUDE, true).unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 0, 4, 1, 1, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_dc_ivf_is_a_parseable_one_frame_av02_ivf() {
        let ivf = emit_minimal_intra_coded_dc_ivf().unwrap();
        assert!(!ivf.is_empty());

        // Structurally a single-frame AV02 64x64 IVF; the decode-to-flat-127 luma proof is
        // the cross-crate oracle in splot-cli.
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        let header = parsed.header.unwrap();
        assert_eq!(&header.fourcc, b"AV02");
        assert_eq!((header.width, header.height), (64, 64));
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn emit_minimal_intra_coded_dc_ivf_is_deterministic() {
        assert_eq!(
            emit_minimal_intra_coded_dc_ivf().unwrap(),
            emit_minimal_intra_coded_dc_ivf().unwrap()
        );
    }

    #[test]
    fn composes_general_coded_chroma_block_trace_in_order() {
        // Magnitude 4: U coeff_base_eob = 3 (level 4), no coeff_br/golomb.
        let trace =
            compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();

        // do_split, 3 modes, luma txb_skip (skip), 3 coded-U symbols, U sign bypass,
        // V txb_skip (skip) = 10 tokens.
        assert_eq!(trace.len(), 10);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        for token in &trace[1..4] {
            assert!(matches!(token, BlockSymbolToken::Mode(_)));
        }
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split=0, modes 0/0/0, luma txb_skip=1 (skip), U txb_skip=0, U eob_pt=0,
        // U coeff_base_eob=3, U sign_bit=1 (negative), V txb_skip=1 (skip).
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 0, 0, 3, 1, 1]
        );
    }

    #[test]
    fn general_coded_chroma_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_coded_chroma_u_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 0, 0, 3, 1, 1]);
        assert_eq!(proof.symbol_count(), 10);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_chroma_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_coded_chroma_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_coded_chroma_ivf().unwrap());
    }

    #[test]
    fn composes_general_coded_chroma_v_block_trace_in_order() {
        let trace =
            compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();

        // do_split, 3 modes, luma txb_skip (skip), U txb_skip (skip), 3 coded-V symbols,
        // V sign bypass = 10 tokens.
        assert_eq!(trace.len(), 10);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[9],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split=0, modes 0/0/0, luma txb_skip=1, U txb_skip=1, V txb_skip=0, V eob_pt=0,
        // V coeff_base_eob=3, V sign_bit=1 (negative).
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 1, 1, 0, 0, 3, 1]
        );
    }

    #[test]
    fn general_coded_chroma_v_block_trace_roundtrips_through_one_coder() {
        let trace =
            compose_general_intra_coded_chroma_v_block_trace(CODED_CHROMA_U_DC_MAGNITUDE, true)
                .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 1, 1, 0, 0, 3, 1]);
        assert_eq!(proof.symbol_count(), 10);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_coded_chroma_v_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_coded_chroma_v_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_coded_chroma_v_ivf().unwrap());
    }

    #[test]
    fn composes_general_all_planes_coded_block_trace_in_order() {
        let trace = compose_general_intra_all_planes_coded_block_trace(
            CODED_LUMA_DC_MAGNITUDE,
            CODED_CHROMA_U_DC_MAGNITUDE,
            true,
        )
        .unwrap();

        // do_split, 3 modes, 5 coded-luma, 3 coded-U, U sign, 3 coded-V, V sign = 17 tokens.
        assert_eq!(trace.len(), 17);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[12],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        assert!(matches!(
            trace[16],
            BlockSymbolToken::Bypass { width: 1, value: 1 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0,eob_pt=0,base=4,br=1,dc_sign=1;
        // U txb_skip=0,eob_pt=0,base=3; U sign=1; V txb_skip=0,eob_pt=0,base=3; V sign=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 0, 4, 1, 1, 0, 0, 3, 1, 0, 0, 3, 1]
        );
    }

    #[test]
    fn general_all_planes_coded_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_all_planes_coded_block_trace(
            CODED_LUMA_DC_MAGNITUDE,
            CODED_CHROMA_U_DC_MAGNITUDE,
            true,
        )
        .unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();

        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 0, 4, 1, 1, 0, 0, 3, 1, 0, 0, 3, 1]
        );
        assert_eq!(proof.symbol_count(), 17);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_all_planes_coded_ivf_is_parseable_and_deterministic() {
        let ivf = emit_minimal_intra_all_planes_coded_ivf().unwrap();
        assert!(!ivf.is_empty());
        let parsed = splot_core::ivf::parse_ivf_partial(&ivf);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(ivf, emit_minimal_intra_all_planes_coded_ivf().unwrap());
    }

    #[test]
    fn composes_general_two_coeff_block_trace_in_order() {
        let trace = compose_general_intra_two_coeff_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, U skip, V skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1 (eob 2), AC coeff_base_eob=0
        // (level 1), DC coeff_base=0 (level 0), AC sign=0; U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1]
        );
    }

    #[test]
    fn general_two_coeff_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_two_coeff_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_two_coeff_ivf_differs_from_skip_and_is_deterministic() {
        let two = emit_minimal_intra_two_coeff_ivf().unwrap();
        assert!(!two.is_empty());
        // A distinct entropy stream from the skip frame (it carries the eob=2 AC symbols).
        assert_ne!(two, emit_minimal_intra_skip_ivf().unwrap());
        assert_eq!(two, emit_minimal_intra_two_coeff_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&two);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn composes_general_visible_ac_block_trace_in_order() {
        let trace = compose_general_intra_visible_ac_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, U skip, V skip = 11 tokens.
        assert_eq!(trace.len(), 11);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1 (eob 2), AC coeff_base_eob=3
        // (level 4, the largest no-coeff_br base level), DC coeff_base=0 (level 0), AC sign=0;
        // U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 3, 0, 0, 1, 1]
        );
    }

    #[test]
    fn general_visible_ac_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_visible_ac_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(proof.decoded_symbols(), &[0, 0, 0, 0, 0, 1, 3, 0, 0, 1, 1]);
        assert_eq!(proof.symbol_count(), 11);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_visible_ac_ivf_differs_from_two_coeff_and_is_deterministic() {
        let visible = emit_minimal_intra_visible_ac_ivf().unwrap();
        assert!(!visible.is_empty());
        // A distinct entropy stream from the level-1 (sub-visible) eob=2 frame.
        assert_ne!(visible, emit_minimal_intra_two_coeff_ivf().unwrap());
        assert_eq!(visible, emit_minimal_intra_visible_ac_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&visible);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }

    #[test]
    fn composes_general_two_nonzero_block_trace_in_order() {
        let trace = compose_general_intra_two_nonzero_block_trace().unwrap();

        // do_split, 3 modes, 4 coded-luma base (txb_skip, eob_pt, AC base_eob, DC base),
        // AC sign bypass, DC dc_sign, U skip, V skip = 12 tokens.
        assert_eq!(trace.len(), 12);
        assert!(matches!(trace[0], BlockSymbolToken::Partition(_)));
        // Reverse-scan sign pass (§5.20.7.27 c=eob-1..0): the AC sign_bit bypass (c=1) comes
        // FIRST, then the DC dc_sign CDF symbol (c=0).
        assert!(matches!(
            trace[8],
            BlockSymbolToken::Bypass { width: 1, value: 0 }
        ));
        assert!(matches!(trace[9], BlockSymbolToken::Coeff(_)));
        // do_split, modes 0/0/0; luma txb_skip=0, eob_pt_1024=1, AC coeff_base_eob=3 (level 4),
        // DC coeff_base=1 (level 1); AC sign=0 (positive); DC dc_sign=1 (negative); U/V txb_skip=1.
        assert_eq!(
            trace.iter().map(|token| token.symbol()).collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 0, 1, 3, 1, 0, 1, 1, 1]
        );
    }

    #[test]
    fn general_two_nonzero_block_trace_roundtrips_through_one_coder() {
        let trace = compose_general_intra_two_nonzero_block_trace().unwrap();
        let proof = roundtrip_block_symbol_trace(&trace).unwrap();
        assert_eq!(
            proof.decoded_symbols(),
            &[0, 0, 0, 0, 0, 1, 3, 1, 0, 1, 1, 1]
        );
        assert_eq!(proof.symbol_count(), 12);
        assert!(!proof.bytes().is_empty());
    }

    #[test]
    fn emit_minimal_intra_two_nonzero_ivf_differs_from_visible_ac_and_is_deterministic() {
        let two = emit_minimal_intra_two_nonzero_ivf().unwrap();
        assert!(!two.is_empty());
        // Distinct from the single-nonzero (visible-AC) frame: it adds the nonzero DC + its sign.
        assert_ne!(two, emit_minimal_intra_visible_ac_ivf().unwrap());
        assert_eq!(two, emit_minimal_intra_two_nonzero_ivf().unwrap());
        let parsed = splot_core::ivf::parse_ivf_partial(&two);
        assert!(parsed.error.is_none());
        assert_eq!(parsed.frames.len(), 1);
    }
}
