// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frame-header quantization structures (AV2 v1.0.0 § 5.18.6,
//! `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6`).
//!
//! Models the quantization cluster of the § 5.18.2 intra-path tail:
//!
//! - `quantization_params()` — § 5.18.6.1 (`#s-5-18-6-1`)
//! - `setup_qm_params()` — § 5.18.6.2 (`#s-5-18-6-2`)
//! - `read_delta_q()` — § 5.18.6.3 (`#s-5-18-6-3`)
//! - `delta_q_params()` — § 5.18.7.8 (`#s-5-18-7-8`)
//! - the § 5.18.2 per-segment lossless/QM derivation loop and the
//!   `allow_tcq` / `allow_parity_hiding` reads (`#s-5-18-2`)
//!
//! Every external input is a named field of [`CoreSeqQuantView`] or an explicit
//! parameter — the parsers never look state up implicitly.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::frame::size::ceil_log2;
use crate::headers::sequence::{SequenceHeaderGeneral, SequenceTqEntropyConfig};
use crate::segment::{MAX_SEGMENTS, SegmentFeature};

use super::segmentation::SegmentationParams;

/// `DELTA_DCQUANT_BITS` (AV2 v1.0.0 § 3): number of bits for the sequence
/// `base_y_dc_delta_q` / `base_uv_dc_delta_q` / `base_uv_ac_delta_q` fields.
const DELTA_DCQUANT_BITS: i32 = 5;

/// `DELTA_DCQUANT_MAX = 1 << (DELTA_DCQUANT_BITS - 2)` (AV2 v1.0.0 § 3).
const DELTA_DCQUANT_MAX: i32 = 1 << (DELTA_DCQUANT_BITS - 2);

/// `DELTA_DCQUANT_MIN = DELTA_DCQUANT_MAX - (1 << DELTA_DCQUANT_BITS) + 1`
/// (AV2 v1.0.0 § 3): the offset added to the raw 5-bit base delta-q fields
/// (AV2 § 5.4.8).
const DELTA_DCQUANT_MIN: i32 = DELTA_DCQUANT_MAX - (1 << DELTA_DCQUANT_BITS) + 1;

/// Maximum `qmNum = pic_qm_num_minus_1 + 1` (AV2 v1.0.0 § 5.18.6.2):
/// `pic_qm_num_minus_1` is `f(2)`, so at most 4 QM sets are signalled per frame.
pub const MAX_PIC_QM_NUM: usize = 4;

/// `SEG_LVL_ALT_Q` (AV2 v1.0.0 § 3): index of the quantizer segment feature,
/// consumed by the `get_qindex` derivation (AV2 § 7.14.2).
const SEG_LVL_ALT_Q: usize = 0;

/// `MAXQ_8_BITS = 255` (AV2 v1.0.0 § 3): maximum quantizer when bit depth is 8.
const MAXQ_8_BITS: i64 = 255;

/// `MAXQ_OFFSET = 24` (AV2 v1.0.0 § 3): increase in allowed quantizer for each
/// increase in bit depth.
const MAXQ_OFFSET: i64 = 24;

/// `MAXQ_10_BITS = MAXQ_8_BITS + 2 * MAXQ_OFFSET` (AV2 v1.0.0 § 3): maximum
/// quantizer when bit depth is 10.
const MAXQ_10_BITS: i64 = MAXQ_8_BITS + 2 * MAXQ_OFFSET;

/// Sequence-derived inputs for the § 5.18.6 quantization structures, the
/// § 5.18.7.8 quantizer-index delta parameters, and the § 5.18.2 per-segment
/// lossless/`allow_tcq`/`allow_parity_hiding` tail.
///
/// Gathered from a fully parsed sequence header
/// (`sequence_transform_quant_entropy_config()`, AV2 v1.0.0 § 5.4.8); base delta-q
/// values are stored in their **derived** form (`BaseYDcDeltaQ` /
/// `BaseUVDcDeltaQ` / `BaseUVAcDeltaQ`), exactly as the § 5.18.2 lossless formula
/// consumes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreSeqQuantView {
    /// `BitDepth` (AV2 § 6.4.1 Table 6.3): selects the `base_q_idx` width
    /// `n = BitDepth == 8 ? 8 : 9` (AV2 § 5.18.6.1).
    pub bit_depth: u8,
    /// `NumPlanes = Monochrome ? 1 : 3` (AV2 § 6.4.1), gating the chroma delta
    /// block (§ 5.18.6.1) and the chroma QM reads (§ 5.18.6.2).
    pub num_planes: u8,
    /// `separate_uv_delta_q` (AV2 § 5.4.8), gating `diff_uv_delta` (§ 5.18.6.1)
    /// and the `qm_v` read (§ 5.18.6.2).
    pub separate_uv_delta_q: bool,
    /// `equal_ac_dc_q` (AV2 § 5.4.8): copies the parsed AC deltas onto the DC
    /// deltas (§ 5.18.6.1).
    pub equal_ac_dc_q: bool,
    /// `y_dc_delta_q_enabled` (AV2 § 5.4.8), gating the `DeltaQYDc` read
    /// (§ 5.18.6.1).
    pub y_dc_delta_q_enabled: bool,
    /// `uv_dc_delta_q_enabled` (AV2 § 5.4.8), gating the `DeltaQUDc` / `DeltaQVDc`
    /// reads (§ 5.18.6.1).
    pub uv_dc_delta_q_enabled: bool,
    /// `uv_ac_delta_q_enabled` (AV2 § 5.4.8), gating the `DeltaQUAc` / `DeltaQVAc`
    /// reads (§ 5.18.6.1).
    pub uv_ac_delta_q_enabled: bool,
    /// Derived `BaseYDcDeltaQ` (AV2 § 5.4.8: `0`, or
    /// `DELTA_DCQUANT_MIN + base_y_dc_delta_q` when `!equal_ac_dc_q`), used by the
    /// § 5.18.2 lossless formula.
    pub base_y_dc_delta_q: i32,
    /// Derived `BaseUVDcDeltaQ` (AV2 § 5.4.8), used by the § 5.18.2 lossless formula.
    pub base_uv_dc_delta_q: i32,
    /// Derived `BaseUVAcDeltaQ` (AV2 § 5.4.8), used by the § 5.18.2 lossless formula.
    pub base_uv_ac_delta_q: i32,
    /// `enable_tcq` (AV2 § 5.4.8): the inferred `allow_tcq` value when it is not
    /// chosen per frame (AV2 § 5.18.2).
    pub enable_tcq: bool,
    /// `choose_tcq_per_frame` (AV2 § 5.4.8), gating the `allow_tcq` read
    /// (AV2 § 5.18.2).
    pub choose_tcq_per_frame: bool,
    /// `enable_parity_hiding` (AV2 § 5.4.8), gating the `allow_parity_hiding` read
    /// (AV2 § 5.18.2).
    pub enable_parity_hiding: bool,
}

impl CoreSeqQuantView {
    /// Builds the quantization view from the parsed general header and
    /// `sequence_transform_quant_entropy_config()` (AV2 v1.0.0 § 5.4.1 / § 5.4.8).
    ///
    /// The base delta-q offsets follow § 5.4.8 exactly: each `Base*DeltaQ` starts
    /// at `0` and becomes `DELTA_DCQUANT_MIN + base_*_delta_q` only when the raw
    /// field is signalled (luma: `!equal_ac_dc_q`; chroma: `!Monochrome`, with
    /// `BaseUVDcDeltaQ = BaseUVAcDeltaQ` when `equal_ac_dc_q`, which the parsed
    /// config already mirrors into `base_uv_dc_delta_q`).
    #[must_use]
    pub fn from_sequence_configs(
        general: &SequenceHeaderGeneral,
        tq: &SequenceTqEntropyConfig,
    ) -> Self {
        let monochrome = general.chroma_format_idc.is_monochrome();
        Self {
            bit_depth: general.bit_depth_idc.bit_depth(),
            num_planes: if monochrome { 1 } else { 3 },
            separate_uv_delta_q: tq.separate_uv_delta_q,
            equal_ac_dc_q: tq.equal_ac_dc_q,
            y_dc_delta_q_enabled: tq.y_dc_delta_q_enabled,
            uv_dc_delta_q_enabled: tq.uv_dc_delta_q_enabled,
            uv_ac_delta_q_enabled: tq.uv_ac_delta_q_enabled,
            base_y_dc_delta_q: if tq.equal_ac_dc_q {
                0
            } else {
                DELTA_DCQUANT_MIN + i32::from(tq.base_y_dc_delta_q)
            },
            base_uv_dc_delta_q: if monochrome {
                0
            } else {
                DELTA_DCQUANT_MIN + i32::from(tq.base_uv_dc_delta_q)
            },
            base_uv_ac_delta_q: if monochrome {
                0
            } else {
                DELTA_DCQUANT_MIN + i32::from(tq.base_uv_ac_delta_q)
            },
            enable_tcq: tq.enable_tcq,
            choose_tcq_per_frame: tq.choose_tcq_per_frame,
            enable_parity_hiding: tq.enable_parity_hiding,
        }
    }

    /// `MaxQ = lookup_maxq( bit_depth_idc )` (AV2 v1.0.0 § 6.4.1 Table 6.3,
    /// `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1`):
    /// `MAXQ_8_BITS` when `BitDepth == 8`, else `MAXQ_10_BITS` (`BitDepth == 10`
    /// is the only other non-reserved value).
    const fn max_q(&self) -> i64 {
        if self.bit_depth == 8 {
            MAXQ_8_BITS
        } else {
            MAXQ_10_BITS
        }
    }
}

/// Parsed `quantization_params()` (AV2 v1.0.0 § 5.18.6.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`).
///
/// Deltas not signalled by the gated reads are `0`, exactly as the spec
/// initializes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuantizationParams {
    /// `base_q_idx` (`f(8)` for 8-bit streams, `f(9)` otherwise).
    pub base_q_idx: u32,
    /// `DeltaQYDc`.
    pub delta_q_y_dc: i32,
    /// `DeltaQUDc`.
    pub delta_q_u_dc: i32,
    /// `DeltaQUAc`.
    pub delta_q_u_ac: i32,
    /// `DeltaQVDc`.
    pub delta_q_v_dc: i32,
    /// `DeltaQVAc`.
    pub delta_q_v_ac: i32,
    /// `diff_uv_delta` (inferred `0` unless `separate_uv_delta_q`).
    pub diff_uv_delta: bool,
}

impl QuantizationParams {
    #[doc = "Builds the inferred TIP-as-output quantizer state from AV2 § 5.18.2."]
    #[must_use]
    pub const fn inferred_tip(base_q_idx: u32, delta_q_u_ac: i32, delta_q_v_ac: i32) -> Self {
        Self {
            base_q_idx,
            delta_q_y_dc: 0,
            delta_q_u_dc: 0,
            delta_q_u_ac,
            delta_q_v_dc: 0,
            delta_q_v_ac,
            diff_uv_delta: false,
        }
    }
}

/// One quantizer-matrix level set `(qm_y[i], qm_u[i], qm_v[i])` parsed by
/// `setup_qm_params()` (AV2 v1.0.0 § 5.18.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QmSetLevels {
    /// `qm_y[i]`.
    pub qm_y: u8,
    /// `qm_u[i]` (copied from `qm_y[i]` when `qm_uv_same_as_y`; `0` for monochrome).
    pub qm_u: u8,
    /// `qm_v[i]` (copied from `qm_u[i]` when `!separate_uv_delta_q`).
    pub qm_v: u8,
}

/// Parsed `setup_qm_params()` (AV2 v1.0.0 § 5.18.6.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SetupQmParams {
    /// `using_qmatrix`.
    pub using_qmatrix: bool,
    /// `pic_qm_num_minus_1` (`f(2)` when `segmentation_enabled`, else inferred `0`;
    /// meaningful only when `using_qmatrix`).
    pub pic_qm_num_minus_1: u8,
    /// The `qmNum = pic_qm_num_minus_1 + 1` parsed level sets; entries beyond
    /// `qmNum` (and all entries when `!using_qmatrix`) are zeroed.
    pub levels: [QmSetLevels; MAX_PIC_QM_NUM],
}

/// Parsed `delta_q_params()` (AV2 v1.0.0 § 5.18.7.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeltaQParams {
    /// `delta_q_present` (read only when `base_q_idx > 0`, else inferred `0`).
    pub delta_q_present: bool,
    /// `delta_q_res` (read only when `delta_q_present`, else inferred `0`).
    pub delta_q_res: u8,
}

/// The § 5.18.2 per-segment lossless/QM derivation and the `allow_tcq` /
/// `allow_parity_hiding` reads (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LosslessInfo {
    /// `LosslessArray[segmentId]`; entries at and beyond `MaxSegments` stay `false`.
    pub lossless_array: [bool; MAX_SEGMENTS],
    /// `CodedLossless`.
    pub coded_lossless: bool,
    /// `HasLosslessSegment`.
    pub has_lossless_segment: bool,
    /// `SegQMLevel[plane][segmentId]`, indexed here as `[segmentId][plane]`
    /// (planes `0..3` = Y/U/V). Meaningful only when `using_qmatrix`; lossless
    /// segments hold the spec-assigned level `15`.
    pub seg_qm_levels: [[u8; 3]; MAX_SEGMENTS],
    /// `allow_tcq` (`0` when `CodedLossless`; read when `choose_tcq_per_frame`;
    /// else inferred `enable_tcq`).
    pub allow_tcq: bool,
    /// `allow_parity_hiding` (`0` when
    /// `CodedLossless || !enable_parity_hiding || allow_tcq`, else read).
    pub allow_parity_hiding: bool,
}

/// `Clip3( low, high, value )` (AV2 v1.0.0 § 4.8 mathematical functions).
const fn clip3(low: i64, high: i64, value: i64) -> i64 {
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }
}

/// `get_qindex( 1, segmentId )` (AV2 v1.0.0 § 7.14.2,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2`): the `ignoreDeltaQ == 1`
/// form used by the § 5.18.2 lossless derivation.
///
/// Per § 7.14.2 with `ignoreDeltaQ == 1`:
///
/// - If `seg_feature_active_idx( segmentId, SEG_LVL_ALT_Q )` is `1`
///   (§ 5.20.5.12: `segmentation_enabled && FeatureEnabled[ idx ][ feature ]`):
///   `data = FeatureData[ segmentId ][ SEG_LVL_ALT_Q ]`,
///   `qindex = base_q_idx + data` (the `CurrentQIndex` step requires
///   `ignoreDeltaQ == 0` and does not apply), return `Clip3( 0, MaxQ, qindex )`.
/// - Otherwise return `base_q_idx` (the second § 7.14.2 bullet also requires
///   `ignoreDeltaQ == 0`).
///
/// Pure function of parsed data; no decoder state is consulted.
pub(crate) fn get_qindex_ignore_delta_q(
    quant: &CoreSeqQuantView,
    base_q_idx: u32,
    segmentation: &SegmentationParams,
    segment_id: usize,
) -> i64 {
    let feature = segmentation
        .features
        .get(segment_id)
        .and_then(|features| features.get(SEG_LVL_ALT_Q))
        .copied()
        .unwrap_or(SegmentFeature::DISABLED);
    if segmentation.segmentation_enabled && feature.enabled {
        let qindex = i64::from(base_q_idx) + i64::from(feature.data);
        clip3(0, quant.max_q(), qindex)
    } else {
        i64::from(base_q_idx)
    }
}

/// `get_qindex( 0, segmentId )` (AV2 v1.0.0 § 7.14.2): the decode-time per-block
/// qindex with `ignoreDeltaQ == 0`, i.e. the `SEG_LVL_ALT_Q` feature offset is added
/// to `CurrentQIndex` (the delta-q-adjusted qindex) rather than `base_q_idx`, then
/// `Clip3( 0, MaxQ, · )`. Reuses the base-parameterized `get_qindex_ignore_delta_q`
/// with `CurrentQIndex` as the base.
#[must_use]
pub fn get_qindex(
    quant: &CoreSeqQuantView,
    current_qindex: u32,
    segmentation: &SegmentationParams,
    segment_id: usize,
) -> u32 {
    let qindex = get_qindex_ignore_delta_q(quant, current_qindex, segmentation, segment_id);
    u32::try_from(qindex).unwrap_or(current_qindex)
}

/// Parses `read_delta_q()` (AV2 v1.0.0 § 5.18.6.3,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-3`): `delta_coded` `f(1)`,
/// then `delta_q` `su(7)` when coded (else `0`).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn read_delta_q(reader: &mut BitReader<'_>) -> Result<i32> {
    let delta_coded = reader.read_flag()?;
    if delta_coded {
        reader.read_su(7)
    } else {
        Ok(0)
    }
}

/// Parses `quantization_params()` (AV2 v1.0.0 § 5.18.6.1,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1`).
///
/// `tip_frame_as_output` is `TipFrameMode == TIP_FRAME_AS_OUTPUT` decoder state; the
/// intra path always passes `false` (`TipFrameMode = TIP_FRAME_DISABLED`,
/// AV2 § 5.18.2). It gates the `DeltaQYDc` and `DeltaQUDc` / `DeltaQVDc` reads.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn parse_quantization_params(
    reader: &mut BitReader<'_>,
    quant: &CoreSeqQuantView,
    tip_frame_as_output: bool,
) -> Result<QuantizationParams> {
    let n = if quant.bit_depth == 8 { 8 } else { 9 };
    let base_q_idx = reader.read_bits(n)?;
    let mut params = QuantizationParams {
        base_q_idx,
        delta_q_y_dc: 0,
        delta_q_u_dc: 0,
        delta_q_u_ac: 0,
        delta_q_v_dc: 0,
        delta_q_v_ac: 0,
        diff_uv_delta: false,
    };
    if !tip_frame_as_output && quant.y_dc_delta_q_enabled {
        params.delta_q_y_dc = read_delta_q(reader)?;
    }
    if quant.num_planes > 1
        && (quant.uv_ac_delta_q_enabled || (!tip_frame_as_output && quant.uv_dc_delta_q_enabled))
    {
        if quant.separate_uv_delta_q {
            params.diff_uv_delta = reader.read_flag()?;
        }
        if !tip_frame_as_output && quant.uv_dc_delta_q_enabled {
            params.delta_q_u_dc = read_delta_q(reader)?;
        }
        if quant.uv_ac_delta_q_enabled {
            params.delta_q_u_ac = read_delta_q(reader)?;
        }
        if quant.equal_ac_dc_q {
            params.delta_q_u_dc = params.delta_q_u_ac;
        }
        if params.diff_uv_delta {
            if !tip_frame_as_output && quant.uv_dc_delta_q_enabled {
                params.delta_q_v_dc = read_delta_q(reader)?;
            }
            if quant.uv_ac_delta_q_enabled {
                params.delta_q_v_ac = read_delta_q(reader)?;
            }
            if quant.equal_ac_dc_q {
                params.delta_q_v_dc = params.delta_q_v_ac;
            }
        } else {
            params.delta_q_v_dc = params.delta_q_u_dc;
            params.delta_q_v_ac = params.delta_q_u_ac;
        }
    }
    Ok(params)
}

/// Parses `setup_qm_params()` (AV2 v1.0.0 § 5.18.6.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-2`).
///
/// `segmentation_enabled` is the frame's parsed `segmentation_enabled`
/// (§ 5.18.7.1); per § 5.18.2 call order, `setup_qm_params()` runs **after**
/// `segmentation_params()`.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn parse_setup_qm_params(
    reader: &mut BitReader<'_>,
    quant: &CoreSeqQuantView,
    segmentation_enabled: bool,
) -> Result<SetupQmParams> {
    let using_qmatrix = reader.read_flag()?;
    let mut pic_qm_num_minus_1 = 0u8;
    let mut levels = [QmSetLevels::default(); MAX_PIC_QM_NUM];
    if using_qmatrix {
        if segmentation_enabled {
            pic_qm_num_minus_1 = reader.read_bits_u8(2)?;
        }
        let qm_num = usize::from(pic_qm_num_minus_1) + 1;
        for level in levels.iter_mut().take(qm_num) {
            level.qm_y = reader.read_bits_u8(4)?;
            if quant.num_planes > 1 {
                let qm_uv_same_as_y = reader.read_flag()?;
                if qm_uv_same_as_y {
                    level.qm_u = level.qm_y;
                    level.qm_v = level.qm_y;
                } else {
                    level.qm_u = reader.read_bits_u8(4)?;
                    if quant.separate_uv_delta_q {
                        level.qm_v = reader.read_bits_u8(4)?;
                    } else {
                        level.qm_v = level.qm_u;
                    }
                }
            }
        }
    }
    Ok(SetupQmParams {
        using_qmatrix,
        pic_qm_num_minus_1,
        levels,
    })
}

/// Parses `delta_q_params()` (AV2 v1.0.0 § 5.18.7.8,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8`).
///
/// `base_q_idx` is the value parsed by `quantization_params()` (§ 5.18.6.1); it
/// gates the `delta_q_present` read.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn parse_delta_q_params(reader: &mut BitReader<'_>, base_q_idx: u32) -> Result<DeltaQParams> {
    let mut delta_q_present = false;
    let mut delta_q_res = 0u8;
    if base_q_idx > 0 {
        delta_q_present = reader.read_flag()?;
    }
    if delta_q_present {
        delta_q_res = reader.read_bits_u8(2)?;
    }
    Ok(DeltaQParams {
        delta_q_present,
        delta_q_res,
    })
}

/// Parses the § 5.18.2 per-segment lossless/QM derivation loop and the
/// `allow_tcq` / `allow_parity_hiding` reads (AV2 v1.0.0 § 5.18.2,
/// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`).
///
/// Uses the minimal `get_qindex(1, segmentId)` form (AV2 § 7.14.2,
/// `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-14-2`) — a pure function of
/// `base_q_idx` and the parsed segmentation `SEG_LVL_ALT_Q` feature data — plus
/// the parsed quantizer deltas and the sequence base DC/AC offsets. When
/// `using_qmatrix`, reads `qm_index` `f(CeilLog2(pic_qm_num_minus_1 + 1))` for
/// each non-lossless segment in `0..max_segments` (`MaxSegments`, AV2 § 5.4.4).
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) if the
/// payload ends mid-field.
pub fn parse_lossless_info(
    reader: &mut BitReader<'_>,
    quant: &CoreSeqQuantView,
    quantization: &QuantizationParams,
    qm: &SetupQmParams,
    delta_q: &DeltaQParams,
    segmentation: &SegmentationParams,
    max_segments: u8,
) -> Result<LosslessInfo> {
    let mut coded_lossless = true;
    let mut has_lossless_segment = false;
    let mut lossless_array = [false; MAX_SEGMENTS];
    let mut seg_qm_levels = [[0u8; 3]; MAX_SEGMENTS];
    let count = usize::from(max_segments).min(MAX_SEGMENTS);
    for (segment_id, (lossless, qm_levels)) in lossless_array
        .iter_mut()
        .zip(seg_qm_levels.iter_mut())
        .enumerate()
        .take(count)
    {
        let qindex =
            get_qindex_ignore_delta_q(quant, quantization.base_q_idx, segmentation, segment_id);
        *lossless = qindex == 0
            && !delta_q.delta_q_present
            && i64::from(quantization.delta_q_y_dc) + i64::from(quant.base_y_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_u_dc) + i64::from(quant.base_uv_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_v_dc) + i64::from(quant.base_uv_dc_delta_q) <= 0
            && i64::from(quantization.delta_q_u_ac) + i64::from(quant.base_uv_ac_delta_q) <= 0
            && i64::from(quantization.delta_q_v_ac) + i64::from(quant.base_uv_ac_delta_q) <= 0;
        if *lossless {
            has_lossless_segment = true;
        } else {
            coded_lossless = false;
        }
        if qm.using_qmatrix {
            if *lossless {
                *qm_levels = [15, 15, 15];
            } else {
                let qm_num = u32::from(qm.pic_qm_num_minus_1) + 1;
                let qm_index_bits = ceil_log2(qm_num);
                let qm_index = reader.read_bits(qm_index_bits)?;
                let level = qm
                    .levels
                    .get(qm_index as usize)
                    .copied()
                    .unwrap_or_default();
                *qm_levels = [level.qm_y, level.qm_u, level.qm_v];
            }
        }
    }
    let allow_tcq = if coded_lossless {
        false
    } else if quant.choose_tcq_per_frame {
        reader.read_flag()?
    } else {
        quant.enable_tcq
    };
    let allow_parity_hiding = if coded_lossless || !quant.enable_parity_hiding || allow_tcq {
        false
    } else {
        reader.read_flag()?
    };
    Ok(LosslessInfo {
        lossless_array,
        coded_lossless,
        has_lossless_segment,
        seg_qm_levels,
        allow_tcq,
        allow_parity_hiding,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;

    use crate::span::ByteOffset;
    use crate::test_support::{base_quant, seg_params};

    use crate::test_bits::Bits;

    /// `quantization_params()` output with the given `base_q_idx` and zero deltas.
    fn quant_params(base_q_idx: u32) -> QuantizationParams {
        QuantizationParams {
            base_q_idx,
            delta_q_y_dc: 0,
            delta_q_u_dc: 0,
            delta_q_u_ac: 0,
            delta_q_v_dc: 0,
            delta_q_v_ac: 0,
            diff_uv_delta: false,
        }
    }

    #[test]
    fn inferred_tip_quantization_retains_independent_ac_deltas() {
        let params = QuantizationParams::inferred_tip(91, -3, 4);
        assert_eq!(params.base_q_idx, 91);
        assert_eq!(params.delta_q_u_ac, -3);
        assert_eq!(params.delta_q_v_ac, 4);
        assert!(!params.diff_uv_delta);
    }

    /// `setup_qm_params()` output with QM disabled.
    fn qm_disabled() -> SetupQmParams {
        SetupQmParams {
            using_qmatrix: false,
            pic_qm_num_minus_1: 0,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        }
    }

    fn no_delta_q() -> DeltaQParams {
        DeltaQParams {
            delta_q_present: false,
            delta_q_res: 0,
        }
    }

    #[test]
    fn read_delta_q_not_coded_is_zero() {
        let mut bits = Bits::default();
        bits.bit(0); // delta_coded = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert_eq!(read_delta_q(&mut reader).unwrap(), 0);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn read_delta_q_coded_positive() {
        let mut bits = Bits::default();
        bits.bit(1); // delta_coded = 1
        bits.su(5, 7); // delta_q su(7) = 5
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert_eq!(read_delta_q(&mut reader).unwrap(), 5);
        assert_eq!(reader.consumed_bits(), 8);
    }

    #[test]
    fn read_delta_q_coded_most_negative() {
        let mut bits = Bits::default();
        bits.bit(1);
        bits.su(-64, 7); // su(7) minimum: 0b1000000
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert_eq!(read_delta_q(&mut reader).unwrap(), -64);
    }

    #[test]
    fn read_delta_q_eof_on_delta_coded() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            read_delta_q(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn read_delta_q_eof_inside_su() {
        let data = [0b0010_0000];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bits(2).unwrap();
        assert!(matches!(
            read_delta_q(&mut reader),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn quantization_params_base_only_8bit() {
        let mut bits = Bits::default();
        bits.f(100, 8); // base_q_idx f(8)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &base_quant(), false).unwrap();
        assert_eq!(params.base_q_idx, 100);
        assert_eq!(params.delta_q_y_dc, 0);
        assert_eq!(params.delta_q_u_dc, 0);
        assert_eq!(params.delta_q_u_ac, 0);
        assert_eq!(params.delta_q_v_dc, 0);
        assert_eq!(params.delta_q_v_ac, 0);
        assert!(!params.diff_uv_delta);
        assert_eq!(reader.consumed_bits(), 8);
    }

    #[test]
    fn quantization_params_9_bit_base_q_idx_for_high_bit_depth() {
        let quant = CoreSeqQuantView {
            bit_depth: 10,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(300, 9); // base_q_idx f(9)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert_eq!(params.base_q_idx, 300);
        assert_eq!(reader.consumed_bits(), 9);
    }

    #[test]
    fn quantization_params_monochrome_reads_no_chroma() {
        let quant = CoreSeqQuantView {
            num_planes: 1,
            separate_uv_delta_q: true,
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(77, 8);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert_eq!(params.base_q_idx, 77);
        assert!(!params.diff_uv_delta);
        assert_eq!(params.delta_q_u_dc, 0);
        assert_eq!(params.delta_q_v_ac, 0);
        assert_eq!(reader.consumed_bits(), 8);
    }

    #[test]
    fn quantization_params_y_dc_delta_read_when_enabled() {
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(50, 8);
        bits.bit(1); // delta_coded
        bits.su(-3, 7); // DeltaQYDc
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert_eq!(params.delta_q_y_dc, -3);
        assert_eq!(reader.consumed_bits(), 16);
    }

    #[test]
    fn quantization_params_tip_frame_as_output_skips_dc_reads() {
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            uv_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(60, 8);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, true).unwrap();
        assert_eq!(params.base_q_idx, 60);
        assert_eq!(params.delta_q_y_dc, 0);
        assert_eq!(params.delta_q_u_dc, 0);
        assert_eq!(reader.consumed_bits(), 8);
    }

    #[test]
    fn quantization_params_shared_uv_delta_copies_v() {
        let quant = CoreSeqQuantView {
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(40, 8);
        bits.bit(1);
        bits.su(2, 7); // DeltaQUDc = 2
        bits.bit(1);
        bits.su(-5, 7); // DeltaQUAc = -5
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert!(!params.diff_uv_delta);
        assert_eq!(params.delta_q_u_dc, 2);
        assert_eq!(params.delta_q_u_ac, -5);
        assert_eq!(params.delta_q_v_dc, 2);
        assert_eq!(params.delta_q_v_ac, -5);
        assert_eq!(reader.consumed_bits(), 24);
    }

    #[test]
    fn quantization_params_separate_uv_with_diff_reads_v() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            uv_dc_delta_q_enabled: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(40, 8);
        bits.bit(1); // diff_uv_delta = 1
        bits.bit(1);
        bits.su(1, 7); // DeltaQUDc = 1
        bits.bit(0); // DeltaQUAc not coded -> 0
        bits.bit(1);
        bits.su(-2, 7); // DeltaQVDc = -2
        bits.bit(0); // DeltaQVAc not coded -> 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert!(params.diff_uv_delta);
        assert_eq!(params.delta_q_u_dc, 1);
        assert_eq!(params.delta_q_u_ac, 0);
        assert_eq!(params.delta_q_v_dc, -2);
        assert_eq!(params.delta_q_v_ac, 0);
        assert_eq!(reader.consumed_bits(), 8 + 1 + 8 + 1 + 8 + 1);
    }

    #[test]
    fn quantization_params_equal_ac_dc_q_copies_ac_to_dc() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            equal_ac_dc_q: true,
            uv_ac_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(30, 8);
        bits.bit(1); // diff_uv_delta = 1
        bits.bit(1);
        bits.su(-4, 7); // DeltaQUAc = -4
        bits.bit(1);
        bits.su(6, 7); // DeltaQVAc = 6
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_quantization_params(&mut reader, &quant, false).unwrap();
        assert_eq!(params.delta_q_u_ac, -4);
        assert_eq!(params.delta_q_u_dc, -4);
        assert_eq!(params.delta_q_v_ac, 6);
        assert_eq!(params.delta_q_v_dc, 6);
        assert_eq!(reader.consumed_bits(), 8 + 1 + 8 + 8);
    }

    #[test]
    fn quantization_params_eof_on_base_q_idx() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_quantization_params(&mut reader, &base_quant(), false),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn quantization_params_eof_inside_delta_read() {
        let quant = CoreSeqQuantView {
            y_dc_delta_q_enabled: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.f(50, 8); // exactly one byte: read_delta_q hits EOF.
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_quantization_params(&mut reader, &quant, false),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn setup_qm_params_disabled_reads_one_bit() {
        let mut bits = Bits::default();
        bits.bit(0); // using_qmatrix = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let qm = parse_setup_qm_params(&mut reader, &base_quant(), true).unwrap();
        assert!(!qm.using_qmatrix);
        assert_eq!(qm.pic_qm_num_minus_1, 0);
        assert_eq!(qm.levels, [QmSetLevels::default(); MAX_PIC_QM_NUM]);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn setup_qm_params_no_segmentation_infers_single_set() {
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(9, 4); // qm_y[0]
        bits.bit(1); // qm_uv_same_as_y
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let qm = parse_setup_qm_params(&mut reader, &base_quant(), false).unwrap();
        assert!(qm.using_qmatrix);
        assert_eq!(qm.pic_qm_num_minus_1, 0);
        assert_eq!(
            qm.levels[0],
            QmSetLevels {
                qm_y: 9,
                qm_u: 9,
                qm_v: 9
            }
        );
        assert_eq!(qm.levels[1], QmSetLevels::default());
        assert_eq!(reader.consumed_bits(), 1 + 4 + 1);
    }

    #[test]
    fn setup_qm_params_segmentation_three_sets_mixed_gating() {
        let quant = CoreSeqQuantView {
            separate_uv_delta_q: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(2, 2); // pic_qm_num_minus_1
        bits.f(1, 4);
        bits.bit(1);
        bits.f(2, 4);
        bits.bit(0);
        bits.f(3, 4); // qm_u[1]
        bits.f(4, 4); // qm_v[1]
        bits.f(5, 4);
        bits.bit(0);
        bits.f(6, 4);
        bits.f(7, 4);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let qm = parse_setup_qm_params(&mut reader, &quant, true).unwrap();
        assert_eq!(qm.pic_qm_num_minus_1, 2);
        assert_eq!(
            qm.levels[0],
            QmSetLevels {
                qm_y: 1,
                qm_u: 1,
                qm_v: 1
            }
        );
        assert_eq!(
            qm.levels[1],
            QmSetLevels {
                qm_y: 2,
                qm_u: 3,
                qm_v: 4
            }
        );
        assert_eq!(
            qm.levels[2],
            QmSetLevels {
                qm_y: 5,
                qm_u: 6,
                qm_v: 7
            }
        );
        assert_eq!(qm.levels[3], QmSetLevels::default());
        assert_eq!(reader.consumed_bits(), 1 + 2 + 5 + 13 + 13);
    }

    #[test]
    fn setup_qm_params_shared_uv_copies_qm_v() {
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(8, 4); // qm_y[0]
        bits.bit(0); // qm_uv_same_as_y = 0
        bits.f(2, 4); // qm_u[0]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let qm = parse_setup_qm_params(&mut reader, &base_quant(), false).unwrap();
        assert_eq!(
            qm.levels[0],
            QmSetLevels {
                qm_y: 8,
                qm_u: 2,
                qm_v: 2
            }
        );
        assert_eq!(reader.consumed_bits(), 1 + 4 + 1 + 4);
    }

    #[test]
    fn setup_qm_params_monochrome_reads_only_qm_y() {
        let quant = CoreSeqQuantView {
            num_planes: 1,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1); // using_qmatrix
        bits.f(12, 4); // qm_y[0]
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let qm = parse_setup_qm_params(&mut reader, &quant, false).unwrap();
        assert_eq!(
            qm.levels[0],
            QmSetLevels {
                qm_y: 12,
                qm_u: 0,
                qm_v: 0
            }
        );
        assert_eq!(reader.consumed_bits(), 1 + 4);
    }

    #[test]
    fn setup_qm_params_eof_cases() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_setup_qm_params(&mut reader, &base_quant(), false),
            Err(Error::UnexpectedEof { .. })
        ));
        let data = [0b1110_0000_u8];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_setup_qm_params(&mut reader, &base_quant(), true),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn delta_q_params_zero_base_q_idx_reads_nothing() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let params = parse_delta_q_params(&mut reader, 0).unwrap();
        assert!(!params.delta_q_present);
        assert_eq!(params.delta_q_res, 0);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn delta_q_params_absent_reads_one_bit() {
        let mut bits = Bits::default();
        bits.bit(0); // delta_q_present = 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_delta_q_params(&mut reader, 10).unwrap();
        assert!(!params.delta_q_present);
        assert_eq!(params.delta_q_res, 0);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn delta_q_params_present_reads_res() {
        let mut bits = Bits::default();
        bits.bit(1); // delta_q_present = 1
        bits.f(2, 2); // delta_q_res
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let params = parse_delta_q_params(&mut reader, 10).unwrap();
        assert!(params.delta_q_present);
        assert_eq!(params.delta_q_res, 2);
        assert_eq!(reader.consumed_bits(), 3);
    }

    #[test]
    fn delta_q_params_eof_cases() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_delta_q_params(&mut reader, 1),
            Err(Error::UnexpectedEof { .. })
        ));
        let data = [0b0000_0001_u8];
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        reader.read_bits(7).unwrap();
        assert!(matches!(
            parse_delta_q_params(&mut reader, 1),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn lossless_all_segments_coded_lossless_reads_no_bits() {
        let quant = CoreSeqQuantView {
            enable_tcq: true,
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(info.lossless_array[..8].iter().all(|&l| l));
        assert!(info.lossless_array[8..].iter().all(|&l| !l));
        assert!(info.coded_lossless);
        assert!(info.has_lossless_segment);
        assert!(!info.allow_tcq);
        assert!(!info.allow_parity_hiding);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn lossless_segment_via_alt_q_skips_qm_index_and_forces_level_15() {
        let quant = CoreSeqQuantView {
            choose_tcq_per_frame: true,
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut segmentation = seg_params(true);
        segmentation.features[0][SEG_LVL_ALT_Q] = SegmentFeature {
            enabled: true,
            data: -40,
        };
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 1,
            levels: [
                QmSetLevels {
                    qm_y: 1,
                    qm_u: 2,
                    qm_v: 3,
                },
                QmSetLevels {
                    qm_y: 4,
                    qm_u: 5,
                    qm_v: 6,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        let mut bits = Bits::default();
        for i in 1..8 {
            bits.bit((i % 2) as u8); // qm_index for segments 1..8: 1,0,1,0,1,0,1
        }
        bits.bit(0); // allow_tcq = 0 (choose_tcq_per_frame)
        bits.bit(1); // allow_parity_hiding = 1
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quant_params(40),
            &qm,
            &no_delta_q(),
            &segmentation,
            8,
        )
        .unwrap();
        assert!(info.lossless_array[0]);
        assert!(!info.lossless_array[1]);
        assert!(!info.coded_lossless);
        assert!(info.has_lossless_segment);
        assert_eq!(info.seg_qm_levels[0], [15, 15, 15]);
        assert_eq!(info.seg_qm_levels[1], [4, 5, 6]);
        assert_eq!(info.seg_qm_levels[2], [1, 2, 3]);
        assert_eq!(info.seg_qm_levels[3], [4, 5, 6]);
        assert_eq!(info.seg_qm_levels[7], [4, 5, 6]);
        assert_eq!(info.seg_qm_levels[8], [0, 0, 0]);
        assert!(!info.allow_tcq);
        assert!(info.allow_parity_hiding);
        assert_eq!(reader.consumed_bits(), 7 + 1 + 1);
    }

    #[test]
    fn lossless_blocked_by_positive_base_uv_ac_offset() {
        let quant = CoreSeqQuantView {
            base_uv_ac_delta_q: 2,
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut bits = Bits::default();
        bits.bit(1); // allow_parity_hiding (allow_tcq inferred enable_tcq = 0)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(!info.has_lossless_segment);
        assert!(!info.coded_lossless);
        assert!(info.lossless_array.iter().all(|&l| !l));
        assert!(!info.allow_tcq);
        assert!(info.allow_parity_hiding);
        assert_eq!(reader.consumed_bits(), 1);
    }

    #[test]
    fn lossless_blocked_by_delta_q_present() {
        let mut segmentation = seg_params(true);
        for segment in &mut segmentation.features {
            segment[SEG_LVL_ALT_Q] = SegmentFeature {
                enabled: true,
                data: -100,
            };
        }
        let delta_q = DeltaQParams {
            delta_q_present: true,
            delta_q_res: 1,
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &base_quant(),
            &quant_params(100),
            &qm_disabled(),
            &delta_q,
            &segmentation,
            8,
        )
        .unwrap();
        assert!(!info.has_lossless_segment);
        assert!(!info.coded_lossless);
    }

    #[test]
    fn lossless_formula_hand_computed_delta_sums() {
        let quant = CoreSeqQuantView {
            base_y_dc_delta_q: 1,
            base_uv_dc_delta_q: -5,
            ..base_quant()
        };
        let mut quantization = quant_params(0);
        quantization.delta_q_y_dc = -1;
        quantization.delta_q_u_dc = 5;
        quantization.delta_q_v_dc = -7; // -7 + (-5) = -12 <= 0
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quantization,
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(info.coded_lossless);
        quantization.delta_q_v_ac = 1;
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quantization,
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(!info.coded_lossless);
        assert!(!info.has_lossless_segment);
    }

    #[test]
    fn lossless_alt_q_feature_makes_zero_base_segment_non_lossless() {
        let mut segmentation = seg_params(true);
        segmentation.features[0][SEG_LVL_ALT_Q] = SegmentFeature {
            enabled: true,
            data: 5,
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &base_quant(),
            &quant_params(0),
            &qm_disabled(),
            &no_delta_q(),
            &segmentation,
            8,
        )
        .unwrap();
        assert!(!info.lossless_array[0]); // qindex = 5
        assert!(info.lossless_array[1]); // no feature -> base_q_idx = 0
        assert!(!info.coded_lossless);
        assert!(info.has_lossless_segment);
    }

    #[test]
    fn lossless_qm_num_one_reads_zero_bit_qm_index() {
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 0,
            levels: [
                QmSetLevels {
                    qm_y: 7,
                    qm_u: 8,
                    qm_v: 9,
                },
                QmSetLevels::default(),
                QmSetLevels::default(),
                QmSetLevels::default(),
            ],
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &base_quant(),
            &quant_params(40),
            &qm,
            &no_delta_q(),
            &seg_params(false),
            16,
        )
        .unwrap();
        assert!(info.seg_qm_levels.iter().all(|l| *l == [7, 8, 9]));
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn lossless_allow_tcq_inferred_from_enable_tcq() {
        let quant = CoreSeqQuantView {
            enable_tcq: true,
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        let info = parse_lossless_info(
            &mut reader,
            &quant,
            &quant_params(40),
            &qm_disabled(),
            &no_delta_q(),
            &seg_params(false),
            8,
        )
        .unwrap();
        assert!(info.allow_tcq);
        assert!(!info.allow_parity_hiding);
        assert_eq!(reader.consumed_bits(), 0);
    }

    #[test]
    fn lossless_eof_cases() {
        let qm = SetupQmParams {
            using_qmatrix: true,
            pic_qm_num_minus_1: 1,
            levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_lossless_info(
                &mut reader,
                &base_quant(),
                &quant_params(40),
                &qm,
                &no_delta_q(),
                &seg_params(false),
                8,
            ),
            Err(Error::UnexpectedEof { .. })
        ));
        let quant = CoreSeqQuantView {
            choose_tcq_per_frame: true,
            ..base_quant()
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_lossless_info(
                &mut reader,
                &quant,
                &quant_params(40),
                &qm_disabled(),
                &no_delta_q(),
                &seg_params(false),
                8,
            ),
            Err(Error::UnexpectedEof { .. })
        ));
        let quant = CoreSeqQuantView {
            enable_parity_hiding: true,
            ..base_quant()
        };
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_lossless_info(
                &mut reader,
                &quant,
                &quant_params(40),
                &qm_disabled(),
                &no_delta_q(),
                &seg_params(false),
                8,
            ),
            Err(Error::UnexpectedEof { .. })
        ));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod proptests {
    use super::*;
    use crate::segment::SEG_LVL_MAX;
    use crate::span::ByteOffset;
    use proptest::prelude::*;

    use crate::test_support::arbitrary_quant_view;

    proptest! {
        /// `read_delta_q()` never panics and never reads past the payload.
        #[test]
        fn read_delta_q_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4)) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = read_delta_q(&mut reader);
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }

        /// `quantization_params()` never panics for any sequence view.
        #[test]
        fn quantization_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..16),
            quant in arbitrary_quant_view(),
            tip in any::<bool>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_quantization_params(&mut reader, &quant, tip);
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }

        /// `setup_qm_params()` never panics for any sequence view.
        #[test]
        fn setup_qm_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..16),
            quant in arbitrary_quant_view(),
            segmentation_enabled in any::<bool>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_setup_qm_params(&mut reader, &quant, segmentation_enabled);
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }

        /// `delta_q_params()` never panics for any `base_q_idx`.
        #[test]
        fn delta_q_params_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..2),
            base_q_idx in any::<u32>(),
        ) {
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_delta_q_params(&mut reader, base_q_idx);
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }

        /// The § 5.18.2 lossless/QM derivation never panics (or overflows) for
        /// arbitrary parsed inputs, including hostile `max_segments` values and
        /// extreme delta/offset magnitudes.
        #[test]
        fn lossless_info_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..8),
            quant in arbitrary_quant_view(),
            base_q_idx in any::<u32>(),
            deltas in any::<[i32; 5]>(),
            using_qmatrix in any::<bool>(),
            pic_qm_num_minus_1 in 0u8..4,
            segmentation_enabled in any::<bool>(),
            alt_q in any::<(bool, i32)>(),
            delta_q_present in any::<bool>(),
            max_segments in any::<u8>(),
        ) {
            let quantization = QuantizationParams {
                base_q_idx,
                delta_q_y_dc: deltas[0],
                delta_q_u_dc: deltas[1],
                delta_q_u_ac: deltas[2],
                delta_q_v_dc: deltas[3],
                delta_q_v_ac: deltas[4],
                diff_uv_delta: false,
            };
            let qm = SetupQmParams {
                using_qmatrix,
                pic_qm_num_minus_1,
                levels: [QmSetLevels::default(); MAX_PIC_QM_NUM],
            };
            let delta_q = DeltaQParams {
                delta_q_present,
                delta_q_res: 0,
            };
            let mut segmentation = SegmentationParams {
                segmentation_enabled,
                reuse_seg_info: false,
                features: [[SegmentFeature::DISABLED; SEG_LVL_MAX]; MAX_SEGMENTS],
                segmentation_update_map: segmentation_enabled,
                segmentation_temporal_update: false,
                seg_id_pre_skip: false,
                last_active_seg_id: 0,
            };
            segmentation.features[0][0] = SegmentFeature {
                enabled: alt_q.0,
                data: alt_q.1,
            };
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_lossless_info(
                &mut reader,
                &quant,
                &quantization,
                &qm,
                &delta_q,
                &segmentation,
                max_segments,
            );
            prop_assert!(reader.consumed_bits() <= (data.len() as u64) * 8);
        }
    }
}
