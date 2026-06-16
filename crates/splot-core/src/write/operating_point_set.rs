// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `operating_point_set_obu()` writer (AV2 v1.0.0 § 5.10, § 5.11,
//! § 5.11.1-§ 5.11.5, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-10`) — the
//! inverse of [`crate::headers::operating_point_set::parse_operating_point_set`].
//!
//! The OBU is identified by `(obu_xlayer_id, ops_id)`. `obu_xlayer_id`
//! ([`GLOBAL_XLAYER_ID`](crate::types::GLOBAL_XLAYER_ID) vs a local id) selects the
//! global vs local syntax branches just as it does in the parser, so this writer
//! takes it as an argument (threaded from the OBU header by the dispatch). The OPS
//! header (§ 5.10) is `ops_reset_flag` `f(1)`, `ops_id` `f(4)`, `ops_cnt` `f(3)`,
//! and — only when `ops_cnt > 0` — `ops_priority` `f(4)`, `ops_intent` `f(7)`,
//! `ops_intent_present_flag` `f(1)`, `ops_ptl_present_flag` `f(1)`,
//! `ops_color_info_present_flag` `f(1)`, and either `ops_mlayer_info_idc` `f(2)`
//! (global) or `ops_reserved_2bits` `f(2)` (local), followed by `ops_cnt`
//! `operating_point_payload()` structures (§ 5.11). Each payload is `ops_data_size`
//! `leb128()`, the gated body, then `byte_alignment()`. `OBU_OPERATING_POINT_SET`
//! is an **extensible** OBU type, so the OBU tail is the dispatch's generic
//! extensible tail (`obu_extension_flag = 0` then `trailing_bits()`); this writer
//! emits the body, not the tail.
//!
//! Every value the § 5.11 parser derives from the parse context rather than reading
//! from the wire — the per-payload `index` (the loop counter), the `xlayer_entries`
//! drawn from the `ops_xlayer_map` set bits in ascending order, the per-payload
//! `computed_size_bytes` (`opsBytes`), and the gated `Option`/source presence flags
//! — is re-derived here and a model that disagrees is rejected before any bit is
//! written, so `read(write(x)) == x` holds for every parser-produced model.

use crate::headers::operating_point_set::{
    OperatingPointPayload, OperatingPointSet, OpsAggregateInfo, OpsColorInfo, OpsDecoderModelInfo,
    OpsMlayerInfo, OpsMlayerSource, OpsSeqProfileTierLevelInfo, OpsXlayerEntry,
};
use crate::types::ExtendedLayerId;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `ops_id` is `f(4)`.
const OPS_ID_BITS: u32 = 4;
/// `ops_cnt` is `f(3)`.
const OPS_COUNT_BITS: u32 = 3;
/// `ops_priority` is `f(4)`.
const OPS_PRIORITY_BITS: u32 = 4;
/// `ops_intent` / `ops_op_intent` is `f(7)`.
const OPS_INTENT_BITS: u32 = 7;
/// Shared 2-bit field (`ops_mlayer_info_idc`, `ops_reserved_2bits`,
/// `ops_ptl_reserved_2bits`).
const OPS_RESERVED_2BITS: u32 = 2;
/// `ops_xlayer_map` is `f(31)` (`MAX_NUM_XLAYERS - 1`); bit `j` selects layer `j`.
const OPS_XLAYER_MAP_BITS: u32 = 31;
/// `ops_mlayer_map` is `f(8)` (`MAX_NUM_MLAYERS`).
const OPS_MLAYER_MAP_BITS: u32 = 8;
/// `ops_tlayer_map` is `f(4)` (`MAX_NUM_TLAYERS`).
const OPS_TLAYER_MAP_BITS: u32 = 4;
/// `ops_embedded_ops_id` is `f(4)`.
const OPS_EMBEDDED_OPS_ID_BITS: u32 = 4;
/// `ops_embedded_op_index` is `f(3)`.
const OPS_EMBEDDED_OP_INDEX_BITS: u32 = 3;
/// `ops_initial_display_delay_minus_1` is `f(4)`.
const OPS_INITIAL_DISPLAY_DELAY_BITS: u32 = 4;
/// `ops_color_description_idc` is `rg(2)`.
const OPS_COLOR_DESCRIPTION_RG: u32 = 2;
/// `ops_config_idc` is `f(6)`.
const OPS_CONFIG_IDC_BITS: u32 = 6;
/// `ops_aggregate_level_idx` is `f(5)`.
const OPS_AGGREGATE_LEVEL_BITS: u32 = 5;
/// `ops_max_interop` is `f(4)`.
const OPS_MAX_INTEROP_BITS: u32 = 4;
/// `ops_seq_profile_idc` / `ops_level_idx` are `f(5)`.
const OPS_PTL_F5: u32 = 5;
/// `ops_mlayer_count` is `f(3)`.
const OPS_MLAYER_COUNT_BITS: u32 = 3;
/// 8-bit color fields (`ops_color_primaries`, `ops_transfer_characteristics`,
/// `ops_matrix_coefficients`).
const OPS_COLOR_F8: u32 = 8;
/// The number of extended layers selectable by `ops_xlayer_map` (bits `0..30`).
const OPS_XLAYER_MAP_LAYERS: u8 = 31;
/// The number of embedded layers selectable by `ops_mlayer_map` (bits `0..7`).
const OPS_MLAYER_MAP_LAYERS: u8 = 8;

/// Writes an `operating_point_set_obu()` body (AV2 v1.0.0 § 5.10, § 5.11), the
/// inverse of [`crate::headers::operating_point_set::parse_operating_point_set`].
/// `obu_xlayer_id` is the OBU's `obu_xlayer_id` (it selects the global vs local
/// branches and must equal `ops.xlayer_id`). The OBU header and the extensible OBU
/// tail are the dispatch's job ([`crate::write::write_complete_obu`]); this writes
/// the typed body only.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU
///   payload begins on a byte boundary).
/// - [`WriteError::NonCanonicalOperatingPointSet`] for a constructed model the
///   § 5.10/§ 5.11 parser could never produce, so it would not round-trip. The
///   `what` label names the offending field:
///   - `"xlayer_id"`: `obu_xlayer_id` disagrees with `ops.xlayer_id` (the parser
///     stores the OBU's id on the model, so the writer's scope argument must match).
///   - `"reset_branch_field"`: a header field whose presence is gated on
///     `ops_cnt > 0` (`priority` / `intent`) is present when `ops_cnt == 0` or
///     absent when `ops_cnt > 0`.
///   - `"intent_present_reset"` / `"ptl_present_reset"` / `"color_present_reset"`:
///     a present-flag that the parser forces `false` when `ops_cnt == 0` is `true`.
///   - `"mlayer_info_idc_scope"`: `mlayer_info_idc` is `Some` for a non-global or
///     reset OPS, or `None` for a global OPS with `ops_cnt > 0` (the parser reads
///     it only on the global, `ops_cnt > 0` branch).
///   - `"local_reserved_scope"`: `local_reserved_2bits` is `Some` for a global or
///     reset OPS, or `None` for a local OPS with `ops_cnt > 0`.
///   - `"payload_count"`: `payloads.len()` disagrees with `ops_cnt`.
///   - `"payload_index"`: a payload `index` disagrees with its position (the parser
///     loop counter).
///   - `"op_intent_gate"`: a payload `op_intent` presence disagrees with
///     `intent_present`.
///   - `"aggregate_info_gate"`: the global top-level `ops_aggregate_info()` presence
///     disagrees with `ptl_present`, or a local payload carries one (it never does).
///   - `"color_info_gate"`: a payload `color_info` presence disagrees with
///     `color_info_present`.
///   - `"xlayer_map_scope"`: `xlayer_map` is `Some` for a local or `None` for a
///     global payload.
///   - `"xlayer_entries"`: the entries do not match the `xlayer_map` set bits in
///     ascending order (global), or a local payload does not carry exactly one
///     entry whose `xlayer_id` equals the OBU's.
///   - `"entry_ptl_gate"`: an entry's `ptl_info` presence disagrees with
///     `ptl_present` (the global per-layer PTL, or the local single-entry PTL), or
///     its `target_xlayer_id` disagrees with the entry layer.
///   - `"local_entry_mlayer"`: a local entry's mlayer source is not `Explicit`
///     (the parser always codes `ops_mlayer_info()` for a local OPS).
///   - `"global_mlayer_source"`: a global entry's mlayer source does not match what
///     `ops_mlayer_info_idc` codes (`idc 0`/`3` → `Absent`, `idc 1` → `Explicit`,
///     `idc 2` → `Explicit`/`Inherited`).
///   - `"color_triple_gate"`: an `ops_color_info()` explicit-triple presence
///     disagrees with `color_description_idc == 0`.
///   - `"mlayer_tlayer_maps"`: an `ops_mlayer_info()` `tlayer_maps` set does not
///     match the `mlayer_map` set bits in ascending order.
///   - `"ops_computed_size"`: a payload's stored `computed_size_bytes` (`opsBytes`)
///     disagrees with the byte length the writer re-derives from the emitted body — a
///     parse measurement a reparse would overwrite, so a disagreement could not
///     round-trip. (The declared `ops_data_size`, by contrast, is emitted verbatim even
///     when it disagrees with the body — the parser preserves that § 6.10.2
///     non-conformance, so the writer reproduces it rather than rejecting.)
/// - [`WriteError::ValueTooWide`] / [`WriteError::ValueOutOfRange`] from the
///   primitive writers for a field value outside its descriptor's domain.
///
/// All checks run before any bit reaches `writer` (the body is drafted into a
/// scratch and appended only on full success), so a rejected model leaves `writer`
/// unchanged and the writer never panics.
pub fn write_operating_point_set(
    writer: &mut BitWriter,
    ops: &OperatingPointSet,
    obu_xlayer_id: ExtendedLayerId,
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    // The parser stores the OBU's obu_xlayer_id on the model and branches on it; a
    // model whose stored id disagrees with the OBU header's scope could not have been
    // parsed from this OBU and would not round-trip.
    if ops.xlayer_id != obu_xlayer_id {
        return Err(non_canonical("xlayer_id"));
    }
    let is_global = obu_xlayer_id.is_global();

    let mut scratch = BitWriter::new();
    scratch.write_bit(u8::from(ops.reset_flag))?;
    scratch.write_bits_u8(ops.ops_id, OPS_ID_BITS)?;
    scratch.write_bits_u8(ops.ops_cnt, OPS_COUNT_BITS)?;

    if ops.ops_cnt == 0 {
        // § 5.10: the header optional fields and every payload are gated on
        // ops_cnt > 0, so a reset OPS carries none of them. A model that stores any
        // of them could never have been parsed.
        reject_reset_invariants(ops)?;
    } else {
        // § 5.10: ops_priority / ops_intent are present iff ops_cnt > 0.
        let priority = ops
            .priority
            .ok_or_else(|| non_canonical("reset_branch_field"))?;
        let intent = ops
            .intent
            .ok_or_else(|| non_canonical("reset_branch_field"))?;
        scratch.write_bits_u8(priority, OPS_PRIORITY_BITS)?;
        scratch.write_bits_u8(intent, OPS_INTENT_BITS)?;
        scratch.write_bit(u8::from(ops.intent_present))?;
        scratch.write_bit(u8::from(ops.ptl_present))?;
        scratch.write_bit(u8::from(ops.color_info_present))?;
        // § 5.10: global reads ops_mlayer_info_idc, local reads ops_reserved_2bits.
        if is_global {
            let idc = ops
                .mlayer_info_idc
                .ok_or_else(|| non_canonical("mlayer_info_idc_scope"))?;
            if ops.local_reserved_2bits.is_some() {
                return Err(non_canonical("local_reserved_scope"));
            }
            scratch.write_bits_u8(idc, OPS_RESERVED_2BITS)?;
        } else {
            let reserved = ops
                .local_reserved_2bits
                .ok_or_else(|| non_canonical("local_reserved_scope"))?;
            if ops.mlayer_info_idc.is_some() {
                return Err(non_canonical("mlayer_info_idc_scope"));
            }
            scratch.write_bits_u8(reserved, OPS_RESERVED_2BITS)?;
        }

        // § 5.10: exactly ops_cnt operating_point_payload() structures follow.
        if ops.payloads.len() != usize::from(ops.ops_cnt) {
            return Err(non_canonical("payload_count"));
        }
        for (index, payload) in ops.payloads.iter().enumerate() {
            // `index` is the parser's loop counter (0..ops_cnt), not a wire field.
            if usize::from(payload.index) != index {
                return Err(non_canonical("payload_index"));
            }
            write_operating_point_payload(&mut scratch, ops, obu_xlayer_id, payload)?;
        }
    }

    writer.append(&scratch)
}

/// Rejects a reset (`ops_cnt == 0`) OPS that stores any of the `ops_cnt > 0`-gated
/// fields, which the § 5.10 parser leaves `None`/`false`/empty.
fn reject_reset_invariants(ops: &OperatingPointSet) -> WriteResult<()> {
    if ops.priority.is_some() || ops.intent.is_some() {
        return Err(non_canonical("reset_branch_field"));
    }
    if ops.intent_present {
        return Err(non_canonical("intent_present_reset"));
    }
    if ops.ptl_present {
        return Err(non_canonical("ptl_present_reset"));
    }
    if ops.color_info_present {
        return Err(non_canonical("color_present_reset"));
    }
    if ops.mlayer_info_idc.is_some() {
        return Err(non_canonical("mlayer_info_idc_scope"));
    }
    if ops.local_reserved_2bits.is_some() {
        return Err(non_canonical("local_reserved_scope"));
    }
    if !ops.payloads.is_empty() {
        return Err(non_canonical("payload_count"));
    }
    Ok(())
}

/// Writes one `operating_point_payload()` (AV2 v1.0.0 § 5.11): `ops_data_size`
/// `leb128()`, the gated body, then `byte_alignment()`. `opsBytes` is re-derived
/// from the emitted body and must equal the model's declared `ops_data_size`.
fn write_operating_point_payload(
    scratch: &mut BitWriter,
    ops: &OperatingPointSet,
    obu_xlayer_id: ExtendedLayerId,
    payload: &OperatingPointPayload,
) -> WriteResult<()> {
    // Draft the body (everything after ops_data_size, through byte_alignment()) so
    // its byte length is the opsBytes the parser would compute. ops_data_size is
    // written before it, then the body appended.
    let mut body = BitWriter::new();

    // § 5.11: ops_op_intent is read iff ops_intent_present_flag.
    if ops.intent_present {
        let op_intent = payload
            .op_intent
            .ok_or_else(|| non_canonical("op_intent_gate"))?;
        body.write_bits_u8(op_intent, OPS_INTENT_BITS)?;
    } else if payload.op_intent.is_some() {
        return Err(non_canonical("op_intent_gate"));
    }

    // § 5.11: the top-level PTL, in parse order (before ops_color_info()). For a global
    // payload this is ops_aggregate_info() (gated on ops_ptl_present_flag); for a local
    // payload it is the single ops_seq_profile_tier_level_info() targeting the OBU's own
    // layer (also gated on ops_ptl_present_flag), stored on the lone xlayer entry. The
    // local xlayer entry is validated here (count, layer) so its PTL can be written now;
    // the entry's ops_mlayer_info() is written later in write_xlayer_section.
    write_top_level_ptl(&mut body, ops, obu_xlayer_id, payload)?;

    // § 5.11: ops_color_info() is read iff ops_color_info_present_flag.
    if ops.color_info_present {
        let color = payload
            .color_info
            .as_ref()
            .ok_or_else(|| non_canonical("color_info_gate"))?;
        write_ops_color_info(&mut body, color)?;
    } else if payload.color_info.is_some() {
        return Err(non_canonical("color_info_gate"));
    }

    // § 5.11: ops_decoder_model_info_for_this_op_present_flag then the optional
    // ops_decoder_model_info().
    body.write_bit(u8::from(payload.decoder_model_info.is_some()))?;
    if let Some(dm) = &payload.decoder_model_info {
        write_ops_decoder_model_info(&mut body, dm)?;
    }

    // § 5.11: ops_initial_display_delay_present_flag then the optional
    // ops_initial_display_delay_minus_1 f(4).
    body.write_bit(u8::from(payload.initial_display_delay_minus_1.is_some()))?;
    if let Some(delay) = payload.initial_display_delay_minus_1 {
        body.write_bits_u8(delay, OPS_INITIAL_DISPLAY_DELAY_BITS)?;
    }

    // § 5.11: the xlayer map / per-layer loop (global) or the single local layer.
    write_xlayer_section(&mut body, ops, obu_xlayer_id, payload)?;

    // § 5.11: byte_alignment() closes the payload; opsBytes counts from after
    // ops_data_size through this padding.
    body.align_to_byte();
    let ops_bytes =
        u32::try_from(body.bit_len() / 8).map_err(|_| non_canonical("ops_data_size"))?;
    // The parser stores BOTH the declared `ops_data_size` (the wire leb128) and the
    // computed `opsBytes` (the body length it measured), and it TOLERATES `declared !=
    // computed` — that is the § 6.10.2 `ops/payload-size-mismatch` non-conformance the
    // validator flags, not a parse error (parse_operating_point_payload returns Ok with
    // both values; see `OperatingPointPayload::has_size_mismatch`). So `declared_size_bytes`
    // is a wire field we reproduce VERBATIM, even when it disagrees with the body (like the
    // reserved `local_reserved_2bits` / `mlayer_info_idc == 3` / PTL reserved bits this
    // writer already preserves). `computed_size_bytes`, by contrast, is a parse measurement:
    // a reparse overwrites it with the re-derived `opsBytes`, so a model whose `computed`
    // disagrees with the body the writer lays out could not round-trip — that one IS
    // locally-decidable, so reject it. Do NOT require `declared == computed`.
    if payload.computed_size_bytes != ops_bytes {
        return Err(non_canonical("ops_computed_size"));
    }

    // § 5.11: emit `ops_data_size` as the declared wire leb128 verbatim — reproducing a
    // tolerated declared-vs-computed mismatch faithfully; the parser advances by the actual
    // body length (`opsBytes`), not by `declared`, so a non-conformant `declared` does not
    // mis-position the next payload and the round-trip holds.
    scratch.write_leb128(payload.declared_size_bytes)?;
    scratch.append(&body)
}

/// Writes the top-level PTL structure of a payload in parse order (before
/// `ops_color_info()`): `ops_aggregate_info()` for a global OPS (gated on
/// `ops_ptl_present_flag`), or the single `ops_seq_profile_tier_level_info()` for a
/// local OPS (also gated, targeting the OBU's own layer, stored on the lone entry).
/// For the local branch the single xlayer entry is validated structurally here (so
/// its PTL can be written in parse order); the entry's `ops_mlayer_info()` is written
/// later by [`write_xlayer_section`].
fn write_top_level_ptl(
    body: &mut BitWriter,
    ops: &OperatingPointSet,
    obu_xlayer_id: ExtendedLayerId,
    payload: &OperatingPointPayload,
) -> WriteResult<()> {
    if obu_xlayer_id.is_global() {
        if ops.ptl_present {
            let aggregate = payload
                .aggregate_info
                .as_ref()
                .ok_or_else(|| non_canonical("aggregate_info_gate"))?;
            write_ops_aggregate_info(body, aggregate)?;
        } else if payload.aggregate_info.is_some() {
            return Err(non_canonical("aggregate_info_gate"));
        }
    } else {
        // A local payload never carries ops_aggregate_info().
        if payload.aggregate_info.is_some() {
            return Err(non_canonical("aggregate_info_gate"));
        }
        // § 5.11: local OPS -> XCount == 1, the single layer is the OBU's own.
        if payload.xlayer_map.is_some() {
            return Err(non_canonical("xlayer_map_scope"));
        }
        if payload.xlayer_entries.len() != 1 {
            return Err(non_canonical("xlayer_entries"));
        }
        let entry = &payload.xlayer_entries[0];
        if entry.xlayer_id != obu_xlayer_id {
            return Err(non_canonical("xlayer_entries"));
        }
        // The local PTL (gated on ops_ptl_present_flag, targeting the OBU's own layer)
        // is read at this point, before ops_color_info().
        write_entry_ptl(body, ops, entry, obu_xlayer_id)?;
    }
    Ok(())
}

/// Writes the payload's xlayer section (§ 5.11): for a global OPS, `ops_xlayer_map`
/// `f(31)` then a per-set-bit loop of the optional per-layer PTL and the
/// `ops_mlayer_info_idc`-driven mlayer source; for a local OPS, the single entry's
/// `ops_mlayer_info()`.
fn write_xlayer_section(
    body: &mut BitWriter,
    ops: &OperatingPointSet,
    obu_xlayer_id: ExtendedLayerId,
    payload: &OperatingPointPayload,
) -> WriteResult<()> {
    if obu_xlayer_id.is_global() {
        let map = payload
            .xlayer_map
            .ok_or_else(|| non_canonical("xlayer_map_scope"))?;
        // The parser visits set bits 0..30 in ascending order, one entry each; the
        // entries must match that exact set and order to round-trip.
        let expected: Vec<u8> = (0..OPS_XLAYER_MAP_LAYERS)
            .filter(|j| map & (1u32 << u32::from(*j)) != 0)
            .collect();
        if payload.xlayer_entries.len() != expected.len() {
            return Err(non_canonical("xlayer_entries"));
        }
        body.write_bits(map, OPS_XLAYER_MAP_BITS)?;
        for (entry, layer_bit) in payload.xlayer_entries.iter().zip(expected) {
            let entry_xlayer = ExtendedLayerId::from_bits(layer_bit);
            if entry.xlayer_id != entry_xlayer {
                return Err(non_canonical("xlayer_entries"));
            }
            write_global_entry(body, ops, entry, entry_xlayer)?;
        }
    } else {
        // § 5.11: local OPS -> XCount == 1; the single entry (and its PTL) was already
        // validated and its PTL written in parse order by write_top_level_ptl. Here only
        // ops_mlayer_info() remains (always coded for a local OPS).
        let entry = &payload.xlayer_entries[0];
        match &entry.mlayer {
            OpsMlayerSource::Explicit(mlayer) => write_ops_mlayer_info(body, mlayer)?,
            _ => return Err(non_canonical("local_entry_mlayer")),
        }
    }
    Ok(())
}

/// Writes one global xlayer entry: its optional per-layer PTL (gated on
/// `ops_ptl_present_flag`) then the `ops_mlayer_info_idc`-driven mlayer source.
fn write_global_entry(
    body: &mut BitWriter,
    ops: &OperatingPointSet,
    entry: &OpsXlayerEntry,
    entry_xlayer: ExtendedLayerId,
) -> WriteResult<()> {
    write_entry_ptl(body, ops, entry, entry_xlayer)?;
    write_global_mlayer_source(body, ops.mlayer_info_idc, &entry.mlayer)
}

/// Writes an entry's `ops_seq_profile_tier_level_info()` (§ 5.11.2) when
/// `ops_ptl_present_flag` is set, rejecting a presence/target mismatch.
fn write_entry_ptl(
    body: &mut BitWriter,
    ops: &OperatingPointSet,
    entry: &OpsXlayerEntry,
    target_xlayer: ExtendedLayerId,
) -> WriteResult<()> {
    if ops.ptl_present {
        let ptl = entry
            .ptl_info
            .as_ref()
            .ok_or_else(|| non_canonical("entry_ptl_gate"))?;
        // The parser stamps the entry's own layer onto target_xlayer_id.
        if ptl.target_xlayer_id != target_xlayer {
            return Err(non_canonical("entry_ptl_gate"));
        }
        write_ops_seq_profile_tier_level_info(body, ptl)?;
    } else if entry.ptl_info.is_some() {
        return Err(non_canonical("entry_ptl_gate"));
    }
    Ok(())
}

/// Writes a global entry's mlayer source per `ops_mlayer_info_idc` (§ 5.11): `idc 0`
/// and the reserved `idc 3` code nothing (`Absent`); `idc 1` codes
/// `ops_mlayer_info()` (`Explicit`); `idc 2` codes `ops_mlayer_explicit_info_flag`
/// then either `ops_mlayer_info()` (`Explicit`) or the embedded reference
/// (`Inherited`). A source that disagrees with `idc` is rejected.
fn write_global_mlayer_source(
    body: &mut BitWriter,
    mlayer_info_idc: Option<u8>,
    source: &OpsMlayerSource,
) -> WriteResult<()> {
    match mlayer_info_idc {
        Some(1) => match source {
            OpsMlayerSource::Explicit(mlayer) => write_ops_mlayer_info(body, mlayer),
            _ => Err(non_canonical("global_mlayer_source")),
        },
        Some(2) => match source {
            OpsMlayerSource::Explicit(mlayer) => {
                // ops_mlayer_explicit_info_flag = 1, then ops_mlayer_info().
                body.write_bit(1)?;
                write_ops_mlayer_info(body, mlayer)
            }
            OpsMlayerSource::Inherited {
                embedded_ops_id,
                embedded_op_index,
            } => {
                // ops_mlayer_explicit_info_flag = 0, then the embedded reference.
                body.write_bit(0)?;
                body.write_bits_u8(*embedded_ops_id, OPS_EMBEDDED_OPS_ID_BITS)?;
                body.write_bits_u8(*embedded_op_index, OPS_EMBEDDED_OP_INDEX_BITS)
            }
            OpsMlayerSource::Absent => Err(non_canonical("global_mlayer_source")),
        },
        // idc 0, the reserved idc 3, and (defensively) a missing idc code nothing:
        // the source must be Absent.
        _ => match source {
            OpsMlayerSource::Absent => Ok(()),
            _ => Err(non_canonical("global_mlayer_source")),
        },
    }
}

/// Writes `ops_aggregate_info()` (AV2 v1.0.0 § 5.11.1).
fn write_ops_aggregate_info(body: &mut BitWriter, info: &OpsAggregateInfo) -> WriteResult<()> {
    body.write_bits_u8(info.config_idc, OPS_CONFIG_IDC_BITS)?;
    body.write_bits_u8(info.aggregate_level_idx, OPS_AGGREGATE_LEVEL_BITS)?;
    body.write_bit(u8::from(info.max_tier_flag))?;
    body.write_bits_u8(info.max_interop, OPS_MAX_INTEROP_BITS)
}

/// Writes `ops_seq_profile_tier_level_info()` (AV2 v1.0.0 § 5.11.2). `target_xlayer_id`
/// is a parse-context field (not on the wire) and is checked by the caller.
fn write_ops_seq_profile_tier_level_info(
    body: &mut BitWriter,
    ptl: &OpsSeqProfileTierLevelInfo,
) -> WriteResult<()> {
    body.write_bits_u8(ptl.seq_profile_idc.get(), OPS_PTL_F5)?;
    body.write_bits_u8(ptl.level_idx, OPS_PTL_F5)?;
    body.write_bit(u8::from(ptl.tier_flag))?;
    body.write_bits_u8(ptl.mlayer_count, OPS_MLAYER_COUNT_BITS)?;
    body.write_bits_u8(ptl.reserved_2bits, OPS_RESERVED_2BITS)
}

/// Writes `ops_decoder_model_info()` (AV2 v1.0.0 § 5.11.3).
fn write_ops_decoder_model_info(body: &mut BitWriter, dm: &OpsDecoderModelInfo) -> WriteResult<()> {
    body.write_uvlc(dm.decoder_buffer_delay)?;
    body.write_uvlc(dm.encoder_buffer_delay)?;
    body.write_bit(u8::from(dm.low_delay_mode_flag))
}

/// Writes `ops_color_info()` (AV2 v1.0.0 § 5.11.4): `ops_color_description_idc`
/// `rg(2)`, the explicit color triple iff that idc is `0`, then
/// `ops_full_range_flag` `f(1)`.
fn write_ops_color_info(body: &mut BitWriter, color: &OpsColorInfo) -> WriteResult<()> {
    body.write_rg(color.color_description_idc, OPS_COLOR_DESCRIPTION_RG)?;
    if color.color_description_idc == 0 {
        // § 5.11.4: the explicit color_primaries / transfer / matrix triple.
        let primaries = color
            .color_primaries
            .ok_or_else(|| non_canonical("color_triple_gate"))?;
        let transfer = color
            .transfer_characteristics
            .ok_or_else(|| non_canonical("color_triple_gate"))?;
        let matrix = color
            .matrix_coefficients
            .ok_or_else(|| non_canonical("color_triple_gate"))?;
        body.write_bits_u8(primaries, OPS_COLOR_F8)?;
        body.write_bits_u8(transfer, OPS_COLOR_F8)?;
        body.write_bits_u8(matrix, OPS_COLOR_F8)?;
    } else if color.color_primaries.is_some()
        || color.transfer_characteristics.is_some()
        || color.matrix_coefficients.is_some()
    {
        return Err(non_canonical("color_triple_gate"));
    }
    body.write_bit(u8::from(color.full_range_flag))
}

/// Writes `ops_mlayer_info()` (AV2 v1.0.0 § 5.11.5): `ops_mlayer_map` `f(8)` then
/// one `ops_tlayer_map` `f(4)` per set bit of the map in ascending order. The
/// `tlayer_maps` set must match the map's set bits exactly to round-trip.
fn write_ops_mlayer_info(body: &mut BitWriter, mlayer: &OpsMlayerInfo) -> WriteResult<()> {
    let expected: Vec<u8> = (0..OPS_MLAYER_MAP_LAYERS)
        .filter(|j| mlayer.mlayer_map & (1u8 << j) != 0)
        .collect();
    if mlayer.tlayer_maps.len() != expected.len() {
        return Err(non_canonical("mlayer_tlayer_maps"));
    }
    body.write_bits_u8(mlayer.mlayer_map, OPS_MLAYER_MAP_BITS)?;
    for (&(layer, tlayer_map), expected_layer) in mlayer.tlayer_maps.iter().zip(expected) {
        if layer != expected_layer {
            return Err(non_canonical("mlayer_tlayer_maps"));
        }
        body.write_bits_u8(tlayer_map, OPS_TLAYER_MAP_BITS)?;
    }
    Ok(())
}

/// Helper constructing the OPS-specific non-canonical reject with a stable `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalOperatingPointSet { what }
}

// The round-trip / reject tests live in a sibling file (kept under the advisory
// source-line limit); `include!` pastes them into this module so their `super::*`
// resolves to the writer above.
#[cfg(test)]
include!("operating_point_set_tests.rs");
