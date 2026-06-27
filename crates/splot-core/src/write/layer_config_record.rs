// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `layer_config_record_obu()` writer (AV2 v1.0.0 § 5.8,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-8`) — the inverse of
//! [`crate::headers::layer_config_record::parse_layer_config_record`].
//!
//! The OBU branches on `obu_xlayer_id`: the global scope parses `lcr_global_info()`
//! (§ 5.8.1) and any other scope parses `lcr_local_info(obu_xlayer_id)` (§ 5.8.2). The
//! parsed model is the [`LayerConfigurationRecord`] enum, so this writer branches on the
//! `Global` / `Local` variant rather than on a threaded header id — the header's
//! `obu_xlayer_id` is the dispatch's concern ([`crate::write::write_complete_obu`] threads
//! it, and the global/local variant of a *parsed* record always agrees with it).
//!
//! The body inverts, in read order: the `lcr_global_info()` / `lcr_local_info()` prefix
//! (including the atlas-id-vs-`lcr_*_reserved_zero_3bits` else-branch), the optional
//! `lcr_aggregate_info()` (§ 5.8.3), the per-xlayer `lcr_seq_profile_tier_level_info()`
//! (§ 5.8.4) and length-bounded `lcr_global_payload()` (§ 5.8.5) loops, and — for a local
//! record — the embedded `lcr_xlayer_info(0, xId)` (§ 5.8.6). `lcr_xlayer_info()` nests
//! `lcr_rep_info()` (§ 5.8.7), `lcr_xlayer_color_info()` (§ 5.8.9), a `byte_alignment()`
//! ([`BitWriter::align_to_byte`]), and then either `lcr_embedded_layer_info()` (§ 5.8.8) or
//! the else-branch atlas reference (`isGlobal && lcr_global_atlas_id_present_flag`).
//!
//! **Header-derived ids.** A record carries `obu_xlayer_id`-derived ids that have no bit
//! representation in the § 5.8 body: [`LcrLocalInfo::xlayer_id`] and the `xlayer_id` of each
//! [`LcrSeqProfileTierLevelInfo`] (the `i` / `xId` argument). For a *global* record the PTL /
//! payload ids are the `lcr_xlayer_map` set-bit ids and are checked against the map (a length /
//! id disagreement is rejected, since the map is a written field). For a *local* record both
//! are the OBU header's `obu_xlayer_id`, which the dispatch threads in: the writer rejects a
//! record whose scope (`Global` / `Local`) or stored `xlayer_id` disagrees with the header, and
//! a PTL whose `xlayer_id` disagrees with the record's — a `(header, record)` mismatch is
//! parser-unproducible (the parser picks the variant and fills both ids from the one
//! `obu_xlayer_id`) and would reparse as a different model. None of these ids are emitted; they
//! are re-derived on reparse from the threaded header, so a *parsed* record round-trips.
//!
//! **`lcr_global_payload()` filler (§ 5.8.5).** The payload is exactly `lcr_data_size * 8`
//! bits: after `lcr_num_dependent_xlayer_map` and `lcr_xlayer_info(1, n)`, the remaining
//! bits are reserved `lcr_remaining_payload_bit`s the model stores as
//! [`LcrGlobalPayload::remaining_payload_bits`]. The writer measures the content bits it
//! emitted and rejects unless `content_bits + remaining_payload_bits == lcr_data_size * 8`
//! (the value the parser re-derives), then emits that many zero bits. The filler length is
//! bounded by the model's own `lcr_data_size`, a real syntax field — not by arbitrary
//! external bytes — so there is no unbounded allocation.
//!
//! `OBU_LAYER_CONFIGURATION_RECORD` is an **extensible** OBU type (§ 5.2.1), so the OBU tail
//! is the dispatch's generic extensible tail (`obu_extension_flag = 0` then
//! `trailing_bits()`); this writer emits the body, not the tail.

use crate::headers::layer_config_record::{
    LayerConfigurationRecord, LcrAggregateInfo, LcrEmbeddedLayerInfo, LcrGlobalInfo,
    LcrGlobalPayload, LcrLocalInfo, LcrRepInfo, LcrSeqProfileTierLevelInfo, LcrXlayerColorInfo,
    LcrXlayerInfo,
};
use crate::types::ExtendedLayerId;
use crate::write::bit_writer::BitWriter;
use crate::write::error::{WriteError, WriteResult};

/// `MAX_NUM_TLAYERS` (AV2 § 3): the bit width of `lcr_tlayer_map` (`f(n)`,
/// `n == MAX_NUM_TLAYERS`) in `lcr_embedded_layer_info()` (§ 5.8.8).
const MAX_NUM_TLAYERS: u32 = 4;
/// `AUX_LAYER` (AV2 § 3 / § 6.8.9): `lcr_layer_type` value that codes an `lcr_auxiliary_type`.
const AUX_LAYER: u8 = 1;
/// `VIEW_EXPLICIT` (AV2 § 6.8.9, `lcr_view_type` table): codes an explicit `lcr_view_id`.
const VIEW_EXPLICIT: u8 = 4;

/// `lcr_xlayer_map` is `f(31)`; the set bits are the associated `LcrXLayerID[]`.
const XLAYER_MAP_BITS: u32 = 31;
/// `lcr_global_config_record_id` / `lcr_global_id` / `lcr_local_id` / atlas ids are `f(3)`.
const F3: u32 = 3;
/// `lcr_global_*_reserved_zero_5bits` / `lcr_local_reserved_zero_5bits` are `f(5)`.
const F5: u32 = 5;
/// `lcr_global_purpose_id` / `lcr_xlayer_purpose_id` are `f(7)`.
const F7: u32 = 7;
/// `lcr_*_atlas_segment_id` / `lcr_*_priority_order` / `lcr_*_rendering_method` /
/// `lcr_mlayer_map` / `lcr_layer_type` / `lcr_view_type` and friends are `f(8)`.
const F8: u32 = 8;
/// `lsptli_reserved_2bits` is `f(2)`; `layer_color_description_idc` is `rg(2)`.
const F2: u32 = 2;
/// `lcr_config_idc` is `f(6)`.
const F6: u32 = 6;
/// `lcr_aggregate_level_idx` / `lcr_seq_profile_idc` / `lcr_max_level_idx` are `f(5)`.
const AGG_F5: u32 = 5;
/// `lcr_max_interop` is `f(4)`.
const F4: u32 = 4;
/// `lcr_max_mlayer_count` is `f(3)`.
const MLAYER_COUNT_BITS: u32 = 3;

/// The atlas-presence context threaded into `lcr_xlayer_info()` (AV2 § 5.8.6): the
/// `isGlobal` flag and the global / local atlas-id-present flags that select the
/// else-branch atlas reference and the per-embedded-layer atlas fields (§ 5.8.8).
struct XlayerCtx {
    /// `isGlobal`: `true` for `lcr_global_payload()`, `false` for `lcr_local_info()`.
    is_global: bool,
    /// `lcr_global_atlas_id_present_flag` from `lcr_global_info()`.
    global_atlas_id_present: bool,
    /// `lcr_local_atlas_id_present_flag[xId]` from `lcr_local_info()`.
    local_atlas_id_present: bool,
}

impl XlayerCtx {
    /// `atlasSegmentPresent` per AV2 § 5.8.8: the global flag for a global record,
    /// otherwise the per-xlayer local flag.
    const fn atlas_segment_present(&self) -> bool {
        if self.is_global {
            self.global_atlas_id_present
        } else {
            self.local_atlas_id_present
        }
    }
}

/// Writes a `layer_config_record_obu()` body (AV2 v1.0.0 § 5.8), the inverse of
/// [`crate::headers::layer_config_record::parse_layer_config_record`]. The OBU header and
/// the extensible OBU tail are the dispatch's job ([`crate::write::write_complete_obu`]);
/// this writes the typed body only.
///
/// `obu_xlayer_id` is the OBU header's `obu_xlayer_id` (the dispatch threads the real value;
/// [`crate::write::write_obu_payload`] defaults it to the non-global `0`). The § 5.8 parser
/// selects the `Global` / `Local` body from it, so the writer rejects a record whose variant or
/// stored local `xlayer_id` disagrees with it.
///
/// # Errors
/// - [`WriteError::WriterNotByteAligned`] if `writer` is not byte-aligned (an OBU payload
///   begins on a byte boundary).
/// - [`WriteError::NonCanonicalLayerConfigRecord`] for a constructed model the § 5.8 parser
///   could never produce (a `Global` / `Local` variant or local `xlayer_id` that disagrees with
///   `obu_xlayer_id`, a local PTL `xlayer_id` that disagrees with the record's, a gated `Option`
///   vs its flag, a set-bit-derived list length / id, the embedded-vs-atlas exclusivity, or the
///   `lcr_global_payload` filler invariant); the `what` label names the offending field.
/// - [`WriteError::ValueTooWide`] / [`WriteError::ValueOutOfRange`] from the primitive
///   writers for a field outside its descriptor domain (e.g. a `lcr_global_config_record_id`
///   that does not fit `f(3)`, or a `lcr_max_pic_width` of `u32::MAX`, which the `uvlc`
///   reader could never have produced).
///
/// All checks run before any bit reaches `writer` (the body is drafted into a scratch and
/// appended only on full success), so a rejected model leaves `writer` unchanged and the
/// writer never panics.
pub fn write_layer_config_record(
    writer: &mut BitWriter,
    record: &LayerConfigurationRecord,
    obu_xlayer_id: ExtendedLayerId,
) -> WriteResult<()> {
    if !writer.is_byte_aligned() {
        return Err(WriteError::WriterNotByteAligned);
    }

    let mut scratch = BitWriter::new();
    // § 5.8: the parser selects the Global vs Local branch SOLELY from obu_xlayer_id, so the
    // model variant must agree with the header the dispatch threads in — a (header, record) pair
    // whose scope disagrees is parser-unproducible and would reparse as the other variant (like
    // the sibling §5.10 OPS writer, which the dispatch passes obu_xlayer_id and which rejects an
    // ops.xlayer_id mismatch).
    match record {
        LayerConfigurationRecord::Global(info) => {
            if !obu_xlayer_id.is_global() {
                return Err(non_canonical("xlayer_scope"));
            }
            write_lcr_global_info(&mut scratch, info)?;
        }
        LayerConfigurationRecord::Local(info) => {
            if obu_xlayer_id.is_global() {
                return Err(non_canonical("xlayer_scope"));
            }
            write_lcr_local_info(&mut scratch, info, obu_xlayer_id)?;
        }
    }
    writer.append(&scratch)
}

/// Writes `lcr_global_info()` (AV2 v1.0.0 § 5.8.1).
fn write_lcr_global_info(scratch: &mut BitWriter, info: &LcrGlobalInfo) -> WriteResult<()> {
    // The `f(31)` map domain rejects bit 31; derive_xlayer_ids matches the parser's loop.
    let xlayer_ids = derive_xlayer_ids(info.xlayer_map);

    scratch.write_bits_u8(info.global_config_record_id, F3)?;
    scratch.write_bits(info.xlayer_map, XLAYER_MAP_BITS)?;
    scratch.write_flag(info.aggregate_info_present)?;
    scratch.write_flag(info.seq_ptl_info_present)?;
    scratch.write_flag(info.global_payload_present)?;
    scratch.write_flag(info.dependent_xlayers_flag)?;
    scratch.write_flag(info.global_atlas_id_present)?;
    scratch.write_bits_u8(info.global_purpose_id, F7)?;
    scratch.write_flag(info.doh_constraint_flag)?;
    scratch.write_flag(info.enforce_tile_alignment_flag)?;

    // § 5.8.1: lcr_global_atlas_id f(3) when present, else lcr_global_reserved_zero_3bits
    // f(3) — the parser forces the reserved field to 0 in the atlas-present branch.
    write_atlas_or_reserved_3bits(
        scratch,
        info.global_atlas_id_present,
        info.global_atlas_id,
        info.reserved_zero_3bits,
    )?;
    scratch.write_bits_u8(info.reserved_zero_5bits, F5)?;

    // § 5.8.1: lcr_aggregate_info() iff lcr_aggregate_info_present_flag.
    match (info.aggregate_info_present, &info.aggregate_info) {
        (true, Some(agg)) => write_lcr_aggregate_info(scratch, agg)?,
        (false, None) => {}
        _ => return Err(non_canonical("aggregate_info_gate")),
    }

    // § 5.8.1: one lcr_seq_profile_tier_level_info(xId) per set bit of the map, iff present.
    if info.seq_ptl_info_present {
        if info.seq_ptl_infos.len() != xlayer_ids.len() {
            return Err(non_canonical("seq_ptl_info_count"));
        }
        for (ptl, &xid) in info.seq_ptl_infos.iter().zip(&xlayer_ids) {
            if ptl.xlayer_id != xid {
                return Err(non_canonical("seq_ptl_xlayer_id"));
            }
            write_lcr_seq_profile_tier_level_info(scratch, *ptl)?;
        }
    } else if !info.seq_ptl_infos.is_empty() {
        return Err(non_canonical("seq_ptl_info_count"));
    }

    // § 5.8.1: lcr_data_size[i] leb128() + lcr_global_payload(xId, sz) per set bit, iff present.
    if info.global_payload_present {
        if info.payloads.len() != xlayer_ids.len() {
            return Err(non_canonical("payload_count"));
        }
        for (payload, &xid) in info.payloads.iter().zip(&xlayer_ids) {
            if payload.xlayer_id != xid {
                return Err(non_canonical("payload_xlayer_id"));
            }
            write_lcr_global_payload(
                scratch,
                payload,
                info.dependent_xlayers_flag,
                info.global_atlas_id_present,
            )?;
        }
    } else if !info.payloads.is_empty() {
        return Err(non_canonical("payload_count"));
    }

    Ok(())
}

/// Writes `lcr_local_info(xId)` (AV2 v1.0.0 § 5.8.2). The record's `xlayer_id` and any nested
/// PTL `xlayer_id` are the OBU header's `obu_xlayer_id` — parse-context with no bit
/// representation, but decidable once the dispatch threads the header in, so a constructed model
/// whose stored `xlayer_id` disagrees with the header (or whose PTL `xlayer_id` disagrees with
/// the record's) is rejected: the parser passes the one `obu_xlayer_id` into both, so a
/// disagreement is parser-unproducible and would not round-trip.
fn write_lcr_local_info(
    scratch: &mut BitWriter,
    info: &LcrLocalInfo,
    obu_xlayer_id: ExtendedLayerId,
) -> WriteResult<()> {
    if info.xlayer_id != obu_xlayer_id.get() {
        return Err(non_canonical("local_xlayer_id"));
    }

    scratch.write_bits_u8(info.global_id, F3)?;
    scratch.write_bits_u8(info.local_id, F3)?;
    scratch.write_flag(info.profile_tier_level_info_present)?;
    scratch.write_flag(info.local_atlas_id_present)?;

    // § 5.8.2: lcr_seq_profile_tier_level_info(xId) iff lcr_profile_tier_level_info_present_flag.
    // The parser passes this record's xId in, so a PTL targeting a different xlayer is
    // parser-unproducible (its xlayer_id is parse-context, dropped on write, so it must match).
    match (info.profile_tier_level_info_present, &info.seq_ptl_info) {
        (true, Some(ptl)) => {
            if ptl.xlayer_id != info.xlayer_id {
                return Err(non_canonical("local_ptl_xlayer_id"));
            }
            write_lcr_seq_profile_tier_level_info(scratch, *ptl)?;
        }
        (false, None) => {}
        _ => return Err(non_canonical("local_ptl_gate")),
    }

    // § 5.8.2: lcr_local_atlas_id f(3) when present, else lcr_local_reserved_zero_3bits f(3).
    write_atlas_or_reserved_3bits(
        scratch,
        info.local_atlas_id_present,
        info.local_atlas_id,
        info.reserved_zero_3bits,
    )?;
    scratch.write_bits_u8(info.reserved_zero_5bits, F5)?;

    let ctx = XlayerCtx {
        is_global: false,
        global_atlas_id_present: false,
        local_atlas_id_present: info.local_atlas_id_present,
    };
    write_lcr_xlayer_info(scratch, &info.xlayer_info, &ctx)
}

/// Writes the atlas-id-vs-`reserved_zero_3bits` else-branch shared by `lcr_global_info()`
/// and `lcr_local_info()` (AV2 § 5.8.1 / § 5.8.2). When the present flag is set the atlas id
/// must be `Some` and the parser forces `reserved_zero_3bits` to `0`; when clear the atlas id
/// must be `None` and the reserved field is reproduced verbatim within `f(3)`.
fn write_atlas_or_reserved_3bits(
    scratch: &mut BitWriter,
    present: bool,
    atlas_id: Option<u8>,
    reserved_zero_3bits: u8,
) -> WriteResult<()> {
    if present {
        let atlas_id = atlas_id.ok_or_else(|| non_canonical("global_atlas_id_gate"))?;
        if reserved_zero_3bits != 0 {
            // The parser reads no reserved field in the atlas branch, so a non-zero value
            // here is parser-unproducible (it would not round-trip).
            return Err(non_canonical("atlas_reserved_3bits"));
        }
        scratch.write_bits_u8(atlas_id, F3)
    } else {
        if atlas_id.is_some() {
            return Err(non_canonical("global_atlas_id_gate"));
        }
        scratch.write_bits_u8(reserved_zero_3bits, F3)
    }
}

/// Writes `lcr_aggregate_info()` (AV2 v1.0.0 § 5.8.3).
///
/// Kept `&LcrAggregateInfo` rather than by-value so this stays structurally identical
/// to the independent `write_ops_aggregate_info` (§ 5.11.1): the two mirror separate
/// AV2 spec structures whose wire layout coincides today, each with per-section named
/// bit-width constants for spec traceability. Deduplicating them would be a false
/// abstraction across unrelated spec sections (and would erase those named widths).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn write_lcr_aggregate_info(scratch: &mut BitWriter, agg: &LcrAggregateInfo) -> WriteResult<()> {
    scratch.write_bits_u8(agg.config_idc, F6)?;
    scratch.write_bits_u8(agg.aggregate_level_idx, AGG_F5)?;
    scratch.write_flag(agg.max_tier_flag)?;
    scratch.write_bits_u8(agg.max_interop, F4)
}

/// Writes `lcr_seq_profile_tier_level_info(i)` (AV2 v1.0.0 § 5.8.4). The `i` / `xId`
/// argument ([`LcrSeqProfileTierLevelInfo::xlayer_id`]) is parse-context, not a bit field.
/// `lsptli_reserved_2bits` is reproduced verbatim within `f(2)`.
fn write_lcr_seq_profile_tier_level_info(
    scratch: &mut BitWriter,
    ptl: LcrSeqProfileTierLevelInfo,
) -> WriteResult<()> {
    scratch.write_bits_u8(ptl.seq_profile_idc.get(), AGG_F5)?;
    scratch.write_bits_u8(ptl.max_level_idx, AGG_F5)?;
    scratch.write_flag(ptl.tier_flag)?;
    scratch.write_bits_u8(ptl.max_mlayer_count, MLAYER_COUNT_BITS)?;
    scratch.write_bits_u8(ptl.reserved_2bits, F2)
}

/// Writes `lcr_global_payload(n, sz)` (AV2 v1.0.0 § 5.8.5): the `lcr_data_size[i]` `leb128()`
/// prefix, then `lcr_num_dependent_xlayer_map` (iff `lcr_dependent_xlayers_flag && n > 0`),
/// the embedded `lcr_xlayer_info(1, n)`, and the trailing `lcr_remaining_payload_bit` filler.
///
/// The payload is exactly `sz * 8` bits. The writer rejects unless the content it emitted
/// plus the stored [`LcrGlobalPayload::remaining_payload_bits`] equals `sz * 8` — the value
/// the parser re-derives — then emits that many zero filler bits.
fn write_lcr_global_payload(
    scratch: &mut BitWriter,
    payload: &LcrGlobalPayload,
    dependent_xlayers_flag: bool,
    global_atlas_id_present: bool,
) -> WriteResult<()> {
    scratch.write_leb128(payload.data_size)?;

    let content_start = scratch.bit_len();

    // § 5.8.5: lcr_num_dependent_xlayer_map f(n) iff lcr_dependent_xlayers_flag && n > 0.
    let map_present = dependent_xlayers_flag && payload.xlayer_id > 0;
    match (map_present, payload.num_dependent_xlayer_map) {
        (true, Some(map)) => scratch.write_bits(map, u32::from(payload.xlayer_id))?,
        (false, None) => {}
        _ => return Err(non_canonical("num_dependent_gate")),
    }

    let ctx = XlayerCtx {
        is_global: true,
        global_atlas_id_present,
        local_atlas_id_present: false,
    };
    write_lcr_xlayer_info(scratch, &payload.xlayer_info, &ctx)?;

    // § 5.8.5: the payload spans sz * 8 bits; the remainder is reserved filler. data_size is
    // u32 so total_bits fits u64; checked_sub avoids underflow on an over-large content.
    let content_bits = scratch.bit_len() - content_start;
    let total_bits = u64::from(payload.data_size) * 8;
    let expected_remaining = total_bits
        .checked_sub(content_bits)
        .ok_or_else(|| non_canonical("payload_size"))?;
    if expected_remaining != payload.remaining_payload_bits {
        return Err(non_canonical("payload_size"));
    }
    write_zero_bits(scratch, expected_remaining)
}

/// Writes `lcr_xlayer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.6): four present flags, the
/// gated `lcr_rep_info()` / `lcr_xlayer_purpose_id` / `lcr_xlayer_color_info()`, a
/// `byte_alignment()`, then either `lcr_embedded_layer_info()` or the else-branch atlas
/// reference (`isGlobal && lcr_global_atlas_id_present_flag`).
fn write_lcr_xlayer_info(
    scratch: &mut BitWriter,
    info: &LcrXlayerInfo,
    ctx: &XlayerCtx,
) -> WriteResult<()> {
    scratch.write_flag(info.rep_info.is_some())?;
    scratch.write_flag(info.purpose_id.is_some())?;
    scratch.write_flag(info.color_info.is_some())?;
    scratch.write_flag(info.embedded_layer_info.is_some())?;

    if let Some(rep) = &info.rep_info {
        write_lcr_rep_info(scratch, rep)?;
    }
    if let Some(purpose) = info.purpose_id {
        scratch.write_bits_u8(purpose, F7)?;
    }
    if let Some(color) = &info.color_info {
        write_lcr_xlayer_color_info(scratch, color)?;
    }

    // § 5.8.6: byte_alignment() before the embedded-layer / atlas block.
    scratch.align_to_byte();

    if let Some(embedded) = &info.embedded_layer_info {
        if info.xlayer_atlas.is_some() {
            // The parser takes the embedded branch xor the atlas else-branch, never both.
            return Err(non_canonical("embedded_atlas_exclusive"));
        }
        write_lcr_embedded_layer_info(scratch, embedded, ctx.atlas_segment_present())
    } else if ctx.is_global && ctx.global_atlas_id_present {
        // § 5.8.6 else-branch: the f(8) atlas triple for a global record with an atlas id.
        let atlas = info
            .xlayer_atlas
            .as_ref()
            .ok_or_else(|| non_canonical("xlayer_atlas_gate"))?;
        scratch.write_bits_u8(atlas.atlas_segment_id, F8)?;
        scratch.write_bits_u8(atlas.priority_order, F8)?;
        scratch.write_bits_u8(atlas.rendering_method, F8)
    } else {
        // Neither branch is taken; an else-branch atlas reference is parser-unproducible.
        if info.xlayer_atlas.is_some() {
            return Err(non_canonical("embedded_atlas_exclusive"));
        }
        Ok(())
    }
}

/// Writes `lcr_rep_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.7).
fn write_lcr_rep_info(scratch: &mut BitWriter, rep: &LcrRepInfo) -> WriteResult<()> {
    scratch.write_uvlc(rep.max_pic_width)?;
    scratch.write_uvlc(rep.max_pic_height)?;
    scratch.write_flag(rep.format_info.is_some())?;
    scratch.write_flag(rep.cropping_window.is_some())?;

    if let Some(format) = rep.format_info {
        scratch.write_uvlc(format.bit_depth_idc)?;
        scratch.write_uvlc(format.chroma_format_idc)?;
    }
    if let Some(window) = rep.cropping_window {
        scratch.write_uvlc(window.left_offset)?;
        scratch.write_uvlc(window.right_offset)?;
        scratch.write_uvlc(window.top_offset)?;
        scratch.write_uvlc(window.bottom_offset)?;
    }
    Ok(())
}

/// Writes `lcr_xlayer_color_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.9). The
/// `(primaries, transfer, matrix)` triple is read iff `layer_color_description_idc == 0`.
fn write_lcr_xlayer_color_info(
    scratch: &mut BitWriter,
    color: &LcrXlayerColorInfo,
) -> WriteResult<()> {
    scratch.write_rg(color.color_description_idc, F2)?;
    match (color.color_description_idc == 0, color.primaries) {
        (true, Some((primaries, transfer, matrix))) => {
            scratch.write_bits_u8(primaries, F8)?;
            scratch.write_bits_u8(transfer, F8)?;
            scratch.write_bits_u8(matrix, F8)?;
        }
        (false, None) => {}
        _ => return Err(non_canonical("color_primaries_gate")),
    }
    scratch.write_flag(color.full_range_flag)
}

/// Writes `lcr_embedded_layer_info(isGlobal, xId)` (AV2 v1.0.0 § 5.8.8): the `f(8)`
/// `lcr_mlayer_map`, then one layer per set bit `j` (ascending), each closed by a
/// `byte_alignment()`. `atlas_segment_present` gates the per-layer atlas triple.
fn write_lcr_embedded_layer_info(
    scratch: &mut BitWriter,
    embedded: &LcrEmbeddedLayerInfo,
    atlas_segment_present: bool,
) -> WriteResult<()> {
    scratch.write_bits_u8(embedded.mlayer_map, F8)?;

    // The set bits of lcr_mlayer_map are the layer indices, ascending; the stored layers
    // must match them exactly (count and per-element mlayer_index).
    let set_bits: Vec<u8> = (0u8..8)
        .filter(|&j| embedded.mlayer_map & (1u8 << j) != 0)
        .collect();
    if embedded.layers.len() != set_bits.len() {
        return Err(non_canonical("mlayer_layer_count"));
    }

    for (layer, &j) in embedded.layers.iter().zip(&set_bits) {
        if layer.mlayer_index != j {
            return Err(non_canonical("mlayer_index"));
        }

        scratch.write_bits_u8(layer.tlayer_map, MAX_NUM_TLAYERS)?;

        // § 5.8.8: the f(8) atlas triple iff atlasSegmentPresent.
        match (
            atlas_segment_present,
            layer.atlas_segment_id,
            layer.priority_order,
            layer.rendering_method,
        ) {
            (true, Some(seg), Some(prio), Some(method)) => {
                scratch.write_bits_u8(seg, F8)?;
                scratch.write_bits_u8(prio, F8)?;
                scratch.write_bits_u8(method, F8)?;
            }
            (false, None, None, None) => {}
            _ => return Err(non_canonical("embedded_atlas_gate")),
        }

        scratch.write_bits_u8(layer.layer_type, F8)?;
        // § 5.8.8: lcr_auxiliary_type iff lcr_layer_type == AUX_LAYER.
        match (layer.layer_type == AUX_LAYER, layer.auxiliary_type) {
            (true, Some(aux)) => scratch.write_bits_u8(aux, F8)?,
            (false, None) => {}
            _ => return Err(non_canonical("aux_type_gate")),
        }

        scratch.write_bits_u8(layer.view_type, F8)?;
        // § 5.8.8: lcr_view_id iff lcr_view_type == VIEW_EXPLICIT.
        match (layer.view_type == VIEW_EXPLICIT, layer.view_id) {
            (true, Some(view)) => scratch.write_bits_u8(view, F8)?,
            (false, None) => {}
            _ => return Err(non_canonical("view_id_gate")),
        }

        // § 5.8.8: lcr_dependent_layer_map f(j) iff j > 0.
        match (j > 0, layer.dependent_layer_map) {
            (true, Some(map)) => scratch.write_bits(map, u32::from(j))?,
            (false, None) => {}
            _ => return Err(non_canonical("dependent_layer_map_gate")),
        }

        scratch.write_flag(layer.same_sh_max_resolution_flag)?;
        // § 5.8.8: lcr_max_expected_width / _height uvlc() iff !lcr_same_sh_max_resolution_flag.
        match (
            layer.same_sh_max_resolution_flag,
            layer.max_expected_width,
            layer.max_expected_height,
        ) {
            (false, Some(width), Some(height)) => {
                scratch.write_uvlc(width)?;
                scratch.write_uvlc(height)?;
            }
            (true, None, None) => {}
            _ => return Err(non_canonical("max_expected_gate")),
        }

        // § 5.8.8: byte_alignment() at the end of each set-bit iteration.
        scratch.align_to_byte();
    }

    Ok(())
}

/// Derives `LcrXLayerID[]` from `lcr_xlayer_map` (AV2 § 5.8.1): the set bit indices of the
/// 31-bit map, ascending — the order the PTL and payload loops iterate.
fn derive_xlayer_ids(xlayer_map: u32) -> Vec<u8> {
    (0u8..31)
        .filter(|&i| xlayer_map & (1u32 << u32::from(i)) != 0)
        .collect()
}

/// Writes `n` reserved zero bits in 32-bit chunks (the inverse of the
/// `lcr_remaining_payload_bit` read loop in `parse_lcr_global_payload`). `n` is bounded by
/// the model's `lcr_data_size`, so the loop terminates without unbounded allocation.
fn write_zero_bits(scratch: &mut BitWriter, mut n: u64) -> WriteResult<()> {
    while n >= 32 {
        scratch.write_bits(0, 32)?;
        n -= 32;
    }
    if n > 0 {
        // n < 32, so the cast is exact and the f(n) write is in range.
        scratch.write_bits(0, n as u32)?;
    }
    Ok(())
}

/// Helper constructing the layer-config-record-specific non-canonical reject with a stable
/// `what`.
fn non_canonical(what: &'static str) -> WriteError {
    WriteError::NonCanonicalLayerConfigRecord { what }
}

// The round-trip / reject tests live in a sibling file (kept under the advisory source-line
// limit); `include!` pastes them into this module so their `super::*` resolves to the writer
// above.
#[cfg(test)]
include!("layer_config_record_tests.rs");
