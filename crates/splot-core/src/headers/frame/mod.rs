// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prefix-only AV2 frame-header parsing (AV2 v1.0.0 § 5.18.1 / § 5.18.2).
//!
//! This is **not** a full `frame_header()` parser. It reads only the head of
//! `frame_header_info()` — the activation/reference fields needed by validator
//! state — and stops before the rest of § 5.18:
//!
//! ```text
//! frame_header_info( ) {
//!     IsBridge = obu_type == OBU_BRIDGE_FRAME
//!     if ( IsBridge ) cur_mfh_id = 0
//!     else            cur_mfh_id                       uvlc()
//!     if ( cur_mfh_id == 0 ) {
//!         seq_header_id_in_frame_header                uvlc()
//!         load_sequence_header( seq_header_id_in_frame_header )
//!     } else {
//!         load_sequence_header( MfhSeqHeaderId[ cur_mfh_id ] )
//!     }
//!     ...                                              // deeper § 5.18, not parsed
//! }
//! ```
//!
//! `frame_header(isFirst)` calls `frame_header_info()` only when `isFirst` is `1`;
//! when `isFirst` is `0` it calls `frame_header_copy()`, a bit copy of the first
//! header that this prefix parser does not model. Callers therefore only reach this
//! parser on the first-header path.

use crate::bitio::BitReader;
use crate::error::Result;
use crate::headers::sequence::SequenceHeaderId;
use crate::hls::MfhId;
use crate::types::ObuType;

mod config;
mod encoder_input;
mod filtering;
mod get_ref_frames;
mod global_motion;
mod info;
mod inter;
mod inter_shared_tail;
mod quant;
mod restoration;
mod segmentation;
mod size;
mod tail;
mod tiling;

pub use config::IntrabcParams;
#[cfg(test)]
pub(crate) use config::{parse_intrabc_params_full, parse_screen_content_params_full};
pub use encoder_input::{
    MinimalIntraCoreError, MinimalIntraIvfError, MinimalIntraSequenceHeaderError,
    MinimalIntraTileGroupError, build_minimal_intra_clk_core, build_minimal_intra_sequence_header,
    encode_minimal_intra_clk_annexb_obu, encode_minimal_intra_clk_ivf,
    encode_minimal_intra_clk_ivf_with_base_q_idx,
    encode_minimal_intra_clk_temporal_unit_with_base_q_idx,
    encode_minimal_intra_clk_tile_group_obu, encode_minimal_intra_sequence_header_obu,
    encode_temporal_delimiter_obu,
};
pub(crate) use filtering::gdf_per_block_is_coded;
pub use filtering::{
    CdefParams, CdefStrengthSet, CoreSeqFilterView, DeblockingFilterParams, GdfGeometry, GdfParams,
    InterpolationFilter, MfhDeblockingView, gdf_block_size, parse_cdef_params,
    parse_deblocking_filter_params, parse_gdf_params, read_interpolation_filter,
};
pub use get_ref_frames::{RESTRICTED_OH, get_relative_dist};
pub use global_motion::{
    GlobalMotionInput, GlobalMotionParams, GlobalMotionRef, GlobalMotionReferenceState,
    GlobalMotionStop, GmType, SavedGlobalMotionOrderHints, SavedGlobalMotionParams,
    decode_signed_subexp_with_ref, decode_subexp, decode_unsigned_subexp_with_ref,
    inverse_recenter, parse_global_motion_params, read_global_param, scale_warp_model,
};
pub use info::{
    CoreSeqInterView, CoreSeqView, FrameHeaderCore, FrameHeaderParseInput, FrameHeaderParseMode,
    FrameHeaderParseStatus, FrameReferenceStateView, FrameType, MfhFrameView, SefTrailingBits,
    parse_frame_header_core,
};
pub(crate) use info::{init_core_from_prefix, parse_core_body};
pub use inter::{
    EXTENDWARP, INTERINTRA, InterControl, InterStop, LOCALWARP, MOTION_MODES, MvPrecision,
    TipFrameMode,
};
pub(crate) use quant::get_qindex_ignore_delta_q;
pub use quant::{
    CoreSeqQuantView, DeltaQParams, LosslessInfo, MAX_PIC_QM_NUM, QmSetLevels, QuantizationParams,
    SetupQmParams, get_qindex, parse_delta_q_params, parse_lossless_info,
    parse_quantization_params, parse_setup_qm_params, read_delta_q,
};
pub use restoration::{
    CCSO_BAND_NUM, CcsoParams, CcsoPlaneParams, CoreSeqCcsoView, CoreSeqRestorationView,
    FrameRestorationType, LrGeometry, LrParams, LrParseOutcome, LrPartialParams, LrPlaneParams,
    LrTemporalReferenceView, SlotFrameFilterTaps, WienerNsFrameFilterBank,
    WienerNsFrameFilterClass, ccso_quant_step, parse_ccso_params, parse_lr_params,
    parse_lr_params_for_inter,
};
/// The § 5.18.7.11 / § 5.18.7.12 helpers and constants the
/// [`crate::write::frame_restoration`] writer shares with the parser so the two never drift:
/// the size-signaling base/default and the `indexToTool` table.
pub(crate) use restoration::{
    CCSO_INPUT_INTERVAL, RESTORATION_TILESIZE_MAX, default_restoration_size, lr_plane_tool_table,
};
pub use segmentation::{CoreSeqSegView, MfhSegView, SegmentationParams, parse_segmentation_params};
pub use size::FrameSize;
pub(crate) use size::ceil_log2;
#[cfg(test)]
pub(crate) use size::parse_frame_size;
pub use tail::{
    FilmGrainConfig, FrameHeaderTail, FrameTailInput, TxMode, parse_film_grain_config,
    parse_intra_tail, read_tx_mode,
};
pub use tiling::{CoreSeqTileView, TileInfo, parse_tile_info};

/// How much of `frame_header()` the prefix parser consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameHeaderPrefixStatus {
    /// The parser reached the activation/reference fields of `frame_header_info()`
    /// and intentionally stopped; the rest of § 5.18 was **not** consumed. A
    /// full-payload trailing-bits check must not be inferred from this prefix.
    ActivationFieldsOnly,
    /// Reserved for a future, fully-consumed special-case frame-header path. The
    /// current prefix parser never produces it.
    CompleteForSpecialCase,
}

impl FrameHeaderPrefixStatus {
    /// Returns a stable snake-case label for tools and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ActivationFieldsOnly => "activation_fields_only",
            Self::CompleteForSpecialCase => "complete_for_special_case",
        }
    }
}

/// A prefix-only parse of the AV2 frame header (AV2 v1.0.0 § 5.18.1 / § 5.18.2).
///
/// Only the activation/reference fields are modeled. The `is_*` flags are derived
/// from `obu_type` (AV2 § 5.18.2), and `seq_header_id_in_frame_header` keeps the raw
/// `uvlc` value so an out-of-range id can be reported distinctly from an absent one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameHeaderPrefix {
    /// The OBU type whose payload carried this frame header.
    pub obu_type: ObuType,
    /// `isFirst`: this prefix is only produced for the first-header path
    /// (`frame_header_info()`), so it is always `true`.
    pub is_first: bool,
    /// `keyFrame`: `obu_type` is `OBU_CLOSED_LOOP_KEY` or `OBU_OPEN_LOOP_KEY`.
    pub is_key_frame: bool,
    /// `IsBridge`: `obu_type` is `OBU_BRIDGE_FRAME`.
    pub is_bridge: bool,
    /// `IsRegular`: derived from `obu_type` per AV2 § 5.18.2.
    pub is_regular: bool,
    /// `startCVS`: `obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU`
    /// (AV2 § 5.18.2). `FirstPictureInTU` is decoder state the caller supplies.
    /// `None` only when the carried type is `OBU_CLOSED_LOOP_KEY` *and* the caller
    /// withheld `FirstPictureInTU` (the stateless dispatch front door): startCVS is
    /// then genuinely unknown rather than fabricated `false`. For every non-CLK type
    /// the derivation does not consult `FirstPictureInTU`, so it is always
    /// `Some(false)` regardless of the input.
    pub starts_cvs: Option<bool>,
    /// `cur_mfh_id` (inferred `0` for bridge frames). The raw value may be out of
    /// range; gate on [`MfhId::in_range`].
    pub cur_mfh_id: MfhId,
    /// `seq_header_id_in_frame_header` raw `uvlc` value, present only when
    /// `cur_mfh_id == 0`. May be out of range (`>= MAX_SEQ_NUM`).
    pub seq_header_id_in_frame_header: Option<u32>,
    /// The directly referenced sequence header when `cur_mfh_id == 0` and
    /// `seq_header_id_in_frame_header` is in range. `None` when out of range or when
    /// `cur_mfh_id > 0` (the validator resolves that path through the MFH record).
    pub referenced_sequence_header_id: Option<SequenceHeaderId>,
    /// Bits consumed by this prefix parse (not the whole frame header).
    pub consumed_bits: u64,
    /// How much of § 5.18 was consumed (always [`FrameHeaderPrefixStatus::ActivationFieldsOnly`]).
    pub status: FrameHeaderPrefixStatus,
}

/// Returns `true` if `obu_type` is `keyFrame` per AV2 § 5.18.2.
#[must_use]
pub(crate) fn derive_key_frame(obu_type: ObuType) -> bool {
    matches!(obu_type, ObuType::ClosedLoopKey | ObuType::OpenLoopKey)
}

/// Returns `IsRegular` per AV2 § 5.18.2.
#[must_use]
pub(crate) fn derive_is_regular(obu_type: ObuType) -> bool {
    matches!(
        obu_type,
        ObuType::OpenLoopKey
            | ObuType::RegularTileGroup
            | ObuType::RegularTip
            | ObuType::RegularSef
            | ObuType::Switch
            | ObuType::RasFrame
            | ObuType::BridgeFrame
    )
}

/// Parses the `frame_header_info()` activation prefix (AV2 v1.0.0 § 5.18.2).
///
/// `obu_type` is the OBU whose payload this frame header belongs to, and
/// `first_picture_in_tu` is the decoder-state `FirstPictureInTU` used only to derive
/// `startCVS`. Pass `Some(known)` on a stateful path that tracks `FirstPictureInTU`;
/// pass `None` on the stateless dispatch front door that does not. Per AV2 § 5.18.2,
/// `startCVS = obu_type == OBU_CLOSED_LOOP_KEY && FirstPictureInTU`, so the derivation
/// consults the input *only* for `OBU_CLOSED_LOOP_KEY`: a non-CLK type is always
/// `Some(false)`, and a CLK with a withheld input is `None` (genuinely unknown, not a
/// fabricated `false`). The parser reads `cur_mfh_id` (unless this is a bridge frame,
/// where it is inferred `0`) and, when `cur_mfh_id == 0`, `seq_header_id_in_frame_header`.
/// It stops immediately afterward.
///
/// # Errors
/// Returns [`Error::UnexpectedEof`](crate::error::Error::UnexpectedEof) or
/// [`Error::InvalidUvlc`](crate::error::Error::InvalidUvlc) if the payload ends or is
/// malformed before the activation fields can be read.
pub fn parse_frame_header_prefix(
    reader: &mut BitReader<'_>,
    obu_type: ObuType,
    first_picture_in_tu: Option<bool>,
) -> Result<FrameHeaderPrefix> {
    let start_bits = reader.consumed_bits();

    let is_key_frame = derive_key_frame(obu_type);
    let is_bridge = obu_type == ObuType::BridgeFrame;
    let is_regular = derive_is_regular(obu_type);
    let starts_cvs = if obu_type == ObuType::ClosedLoopKey {
        first_picture_in_tu
    } else {
        Some(false)
    };

    let cur_mfh_id = if is_bridge {
        MfhId::zero()
    } else {
        MfhId::from_raw(reader.read_uvlc()?)
    };

    let (seq_header_id_in_frame_header, referenced_sequence_header_id) = if cur_mfh_id.is_zero() {
        let raw = reader.read_uvlc()?;
        (Some(raw), SequenceHeaderId::try_new(raw))
    } else {
        (None, None)
    };

    Ok(FrameHeaderPrefix {
        obu_type,
        is_first: true,
        is_key_frame,
        is_bridge,
        is_regular,
        starts_cvs,
        cur_mfh_id,
        seq_header_id_in_frame_header,
        referenced_sequence_header_id,
        consumed_bits: reader.consumed_bits().saturating_sub(start_bits),
        status: FrameHeaderPrefixStatus::ActivationFieldsOnly,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::span::ByteOffset;

    use crate::test_bits::Bits;

    #[test]
    fn frame_header_prefix_reads_cur_mfh_id_zero_and_seq_header_id() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0 -> direct sequence-header reference
        bits.uvlc(1); // seq_header_id_in_frame_header
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        assert!(prefix.is_first);
        assert!(prefix.is_key_frame);
        assert!(!prefix.is_bridge);
        assert_eq!(prefix.starts_cvs, Some(true)); // CLK + FirstPictureInTU
        assert!(prefix.cur_mfh_id.is_zero());
        assert_eq!(prefix.seq_header_id_in_frame_header, Some(1));
        assert_eq!(
            prefix.referenced_sequence_header_id,
            SequenceHeaderId::try_new(1)
        );
        assert_eq!(prefix.status, FrameHeaderPrefixStatus::ActivationFieldsOnly);
    }

    #[test]
    fn frame_header_prefix_clk_not_first_picture_does_not_start_cvs() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(false)).unwrap();
        assert_eq!(prefix.starts_cvs, Some(false));
    }

    #[test]
    fn frame_header_prefix_clk_unknown_first_picture_leaves_start_cvs_none() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, None).unwrap();
        assert_eq!(prefix.starts_cvs, None);
    }

    #[test]
    fn frame_header_prefix_non_clk_unknown_first_picture_is_some_false() {
        let mut bits = Bits::default();
        bits.uvlc(0);
        bits.uvlc(0);
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix = parse_frame_header_prefix(&mut reader, ObuType::OpenLoopKey, None).unwrap();
        assert_eq!(prefix.starts_cvs, Some(false));
    }

    #[test]
    fn frame_header_prefix_reads_nonzero_cur_mfh_id() {
        let mut bits = Bits::default();
        bits.uvlc(2); // cur_mfh_id == 2 -> sequence header resolved via MFH record
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::RegularTileGroup, Some(false)).unwrap();
        assert!(!prefix.is_key_frame);
        assert!(prefix.is_regular);
        assert_eq!(prefix.starts_cvs, Some(false)); // not a CLK
        assert_eq!(prefix.cur_mfh_id.get(), 2);
        assert!(prefix.cur_mfh_id.in_range());
        assert_eq!(prefix.seq_header_id_in_frame_header, None);
        assert_eq!(prefix.referenced_sequence_header_id, None);
    }

    #[test]
    fn frame_header_prefix_bridge_infers_cur_mfh_id_zero() {
        let mut bits = Bits::default();
        bits.uvlc(3); // seq_header_id_in_frame_header (cur_mfh_id inferred 0)
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::BridgeFrame, Some(true)).unwrap();
        assert!(prefix.is_bridge);
        assert!(prefix.is_regular);
        assert!(!prefix.is_key_frame);
        assert_eq!(prefix.starts_cvs, Some(false)); // bridge frame is not a CLK
        assert!(prefix.cur_mfh_id.is_zero());
        assert_eq!(prefix.seq_header_id_in_frame_header, Some(3));
    }

    #[test]
    fn frame_header_prefix_out_of_range_seq_header_id_is_surfaced_raw() {
        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        bits.uvlc(16); // seq_header_id_in_frame_header == MAX_SEQ_NUM -> out of range
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::OpenLoopKey, Some(true)).unwrap();
        assert_eq!(prefix.seq_header_id_in_frame_header, Some(16));
        assert_eq!(prefix.referenced_sequence_header_id, None);
    }

    #[test]
    fn frame_header_prefix_eof_is_structured_error() {
        let mut reader = BitReader::new(&[], ByteOffset::new(0));
        assert!(matches!(
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)),
            Err(Error::UnexpectedEof { .. })
        ));

        let mut bits = Bits::default();
        bits.uvlc(0); // cur_mfh_id == 0
        let data = bits.into_bytes();
        let mut reader = BitReader::new(&data, ByteOffset::new(0));
        assert!(matches!(
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)),
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
        /// The frame-header prefix parser must never panic on arbitrary input.
        #[test]
        fn parse_frame_header_prefix_never_panics(
            data in proptest::collection::vec(any::<u8>(), 0..64),
            raw_type in 0u8..=31,
            first_picture in any::<Option<bool>>(),
        ) {
            let obu_type = ObuType::from_raw(raw_type);
            let mut reader = BitReader::new(&data, ByteOffset::new(0));
            let _ = parse_frame_header_prefix(&mut reader, obu_type, first_picture);
        }
    }
}
