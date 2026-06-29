// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Byte-for-byte round-trip cover for every [`ReconError`] `Display` rendering.
//!
//! The `Display` impl is derived by `thiserror`; these goldens were captured
//! from the previous hand-written `Display` match, so the derive must reproduce
//! each message exactly.

use super::*;

fn sz(w: usize, h: usize) -> PlaneSize {
    PlaneSize::new(w, h).unwrap()
}

fn rc(x: usize, y: usize, w: usize, h: usize) -> PlaneRect {
    PlaneRect::new(x, y, w, h).unwrap()
}

/// One instance of every [`ReconError`] variant, in enum-declaration order,
/// each built with distinct asymmetric field values so a field swap or an
/// argument-order regression in an `#[error(..)]` attribute surfaces as a
/// different rendered message. Keep this exhaustive: a new variant must gain a
/// case here (and an [`EXPECTED`] entry) or [`variant_count_is_locked`] fails.
fn all_variants() -> Vec<ReconError> {
    vec![
        ReconError::UnsupportedBitDepthIdc { idc: 7 },
        ReconError::UnsupportedChromaFormatIdc { idc: 9 },
        ReconError::ZeroDimension {
            field: "plane width",
        },
        ReconError::ArithmeticOverflow {
            context: "chroma width",
        },
        ReconError::StrideTooSmall {
            stride_samples: 3,
            storage_width: 5,
        },
        ReconError::BufferLengthMismatch {
            expected: 11,
            actual: 13,
        },
        ReconError::VisibleRectOutOfBounds {
            storage: sz(20, 30),
            rect: rc(1, 2, 3, 4),
        },
        ReconError::CropOriginNotAligned {
            x: 5,
            y: 7,
            subsampling_x: 1,
            subsampling_y: 0,
        },
        ReconError::MissingChromaPlane { plane: PlaneId::U },
        ReconError::UnexpectedChromaPlane { plane: PlaneId::V },
        ReconError::PlaneSizeMismatch {
            plane: PlaneId::U,
            expected: sz(8, 16),
            actual: sz(9, 17),
        },
        ReconError::SampleTypeUnsupportedBitDepth {
            sample_type: "u8",
            bit_depth: BitDepth::Ten,
        },
        ReconError::SampleOutOfRange {
            plane: PlaneId::V,
            sample_index: 42,
            value: 1000,
            max: 1023,
        },
        ReconError::SampleValueUnsupportedStorage {
            sample_type: "u8",
            value: 300,
            max: 255,
        },
        ReconError::WorkspaceAllocationFailed {
            plane: PlaneId::U,
            context: "samples",
        },
        ReconError::MissingWorkspacePlane { plane: PlaneId::V },
        ReconError::WorkspaceRectOutOfBounds {
            plane: PlaneId::U,
            storage: sz(64, 48),
            rect: rc(1, 2, 3, 4),
        },
        ReconError::WorkspaceWriteStrideTooSmall {
            plane: PlaneId::U,
            stride_samples: 3,
            width: 7,
        },
        ReconError::WorkspaceWriteLengthMismatch {
            plane: PlaneId::V,
            expected: 100,
            actual: 90,
        },
        ReconError::WorkspaceCopyShapeMismatch {
            plane: PlaneId::U,
            source_rect: rc(0, 0, 4, 8),
            target_rect: rc(0, 0, 5, 9),
        },
        ReconError::InvalidIntraSquareBlockLog2 {
            log2_size: 7,
            min: 2,
            max: 6,
        },
        ReconError::InvalidIntraRectBlockLog2 {
            log2_width: 7,
            log2_height: 8,
            min: 2,
            max: 6,
        },
        ReconError::IntraPredictionEdgeLengthMismatch {
            edge: IntraDcEdge::Above,
            expected: 8,
            actual: 9,
        },
        ReconError::IntraPredictionSampleOutOfRange {
            edge: IntraDcEdge::Left,
            sample_index: 3,
            value: 500,
            max: 255,
        },
        ReconError::IntraPredictionOutputSampleOutOfRange {
            sample_index: 5,
            value: 600,
            max: 255,
        },
        ReconError::IntraPaethEdgeLengthMismatch {
            edge: IntraPaethEdge::TopLeft,
            expected: 4,
            actual: 5,
        },
        ReconError::IntraPaethSampleOutOfRange {
            edge: IntraPaethEdge::Above,
            sample_index: 2,
            value: 700,
            max: 255,
        },
        ReconError::IntraCardinalEdgeUnavailable {
            direction: IntraCardinalDirection::Vertical,
            edge: IntraCardinalEdge::Above,
        },
        ReconError::IntraCardinalEdgeLengthMismatch {
            edge: IntraCardinalEdge::Left,
            expected: 6,
            actual: 7,
        },
        ReconError::IntraCardinalSampleOutOfRange {
            edge: IntraCardinalEdge::Above,
            sample_index: 1,
            value: 800,
            max: 255,
        },
        ReconError::UnsupportedIntraDirectionalAngle { p_angle: 99 },
        ReconError::IntraDirectionalAngleEdgeUnavailable {
            angle: IntraDirectionalAngle::D45,
            edge: IntraDirectionalAngleEdge::Above,
        },
        ReconError::IntraDirectionalAngleEdgeLengthMismatch {
            edge: IntraDirectionalAngleEdge::Left,
            expected: 10,
            actual: 11,
        },
        ReconError::IntraDirectionalAngleSampleOutOfRange {
            edge: IntraDirectionalAngleEdge::Above,
            sample_index: 4,
            value: 900,
            max: 255,
        },
        ReconError::UnsupportedIntraMiddleDirectionalAngle { p_angle: 88 },
        ReconError::IntraMiddleDirectionalAngleEdgeUnavailable {
            angle: IntraMiddleDirectionalAngle::D113,
            edge: IntraDirectionalAngleEdge::Above,
        },
        ReconError::IntraMiddleDirectionalAngleEdgeLengthMismatch {
            edge: IntraDirectionalAngleEdge::Left,
            expected: 12,
            actual: 13,
        },
        ReconError::IntraMiddleDirectionalAngleSampleOutOfRange {
            edge: IntraDirectionalAngleEdge::Above,
            sample_index: 6,
            value: 950,
            max: 255,
        },
        ReconError::IntraSmoothEdgeLengthMismatch {
            edge: IntraSmoothEdge::BottomLeft,
            expected: 14,
            actual: 15,
        },
        ReconError::IntraSmoothSampleOutOfRange {
            edge: IntraSmoothEdge::TopRight,
            sample_index: 7,
            value: 999,
            max: 255,
        },
        ReconError::IntraSmoothPredictionOutOfRange {
            row: 3,
            column: 5,
            value: -7,
            min: 0,
            max: 255,
        },
        ReconError::IntraPredictionAllocationFailed {
            context: "prediction block",
        },
        ReconError::IntraPredictionStrideTooSmall {
            stride_samples: 3,
            width: 8,
        },
        ReconError::IntraPredictionOutputTooSmall {
            expected: 64,
            actual: 60,
        },
        ReconError::WorkspaceIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            edge: IntraPaethEdge::TopLeft,
            rect: rc(1, 2, 3, 4),
        },
        ReconError::WorkspaceSmoothIntraPredictionEdgeUnavailable {
            plane: PlaneId::V,
            edge: IntraSmoothEdge::BottomLeft,
            rect: rc(2, 3, 4, 5),
        },
        ReconError::WorkspaceCardinalIntraPredictionEdgeUnavailable {
            plane: PlaneId::U,
            edge: IntraCardinalEdge::Left,
            rect: rc(3, 4, 5, 6),
        },
        ReconError::WorkspaceDirectionalAngleIntraPredictionEdgeUnavailable {
            plane: PlaneId::V,
            p_angle: 67,
            edge: IntraDirectionalAngleEdge::Above,
            rect: rc(4, 5, 6, 7),
        },
        ReconError::WorkspaceDirectionalAngleIntraPredictionLumaIdifUnsupported {
            plane: PlaneId::Y,
            p_angle: 45,
            rect: rc(5, 6, 7, 8),
        },
        ReconError::InvalidReferenceStoreCapacity {
            capacity: 20,
            max_slots: 16,
        },
        ReconError::InvalidReferenceSlotIndex {
            index: 18,
            max_slots: 16,
        },
        ReconError::InvalidReferenceRefreshMask {
            mask: 0x0001_0001,
            max_slots: 16,
        },
        ReconError::ReferenceSlotOutOfBounds {
            slot: ReferenceSlot::new(5).unwrap(),
            capacity: 4,
        },
        ReconError::ReferenceRefreshMaskOutOfBounds {
            mask: 0x0000_00ff,
            capacity: 4,
        },
        ReconError::InvalidInverseTransformSize { size: 7 },
        ReconError::InverseTransformLengthMismatch {
            src_len: 16,
            out_len: 15,
        },
        ReconError::ReconstructLengthMismatch {
            prediction_len: 16,
            residual_len: 17,
            out_len: 18,
        },
        ReconError::ReconstructPredictionOutOfRange {
            sample_index: 9,
            value: 1100,
            max: 1023,
        },
        ReconError::InvalidInverseTransform2dShape {
            log2_w: 7,
            log2_h: 8,
            lossless: true,
        },
        ReconError::InvalidTransformShiftShape {
            log2_width: 7,
            log2_height: 8,
        },
        ReconError::InvalidPlaneTxType { plane_tx_type: 20 },
        ReconError::InvalidScanShape { w: 5, h: 6 },
        ReconError::ScanLengthMismatch {
            expected: 64,
            out_len: 60,
        },
        ReconError::InverseTransform2dBufferMismatch {
            expected: 64,
            dequant_len: 60,
            residual_len: 62,
        },
        ReconError::InverseTransform2dOuterBufferMismatch {
            dequant_expected: 64,
            residual_expected: 32,
            dequant_len: 60,
            residual_len: 30,
        },
        ReconError::SecondaryTransformInvalidShape { w: 5, h: 6 },
        ReconError::SecondaryTransformBufferMismatch {
            expected: 64,
            actual: 60,
        },
        ReconError::SecondaryTransformInvalidParams {
            n: 3,
            kernel: 5,
            sec_tx_type: 7,
        },
        ReconError::DeblockFilterInvalidWidth {
            max_width_neg: 9,
            max_width_pos: 10,
        },
        ReconError::DeblockFilterLineTooShort {
            boundary: 2,
            max_width_neg: 3,
            width: 4,
            len: 5,
        },
        ReconError::WienerNsFilterOutputStrideTooSmall {
            stride_samples: 3,
            width: 8,
        },
        ReconError::WienerNsFilterOutputTooSmall {
            expected: 64,
            actual: 60,
        },
        ReconError::WienerNsFilterMissingClasses,
        ReconError::WienerNsFilterSubclassMapTooShort {
            expected: 64,
            actual: 60,
        },
        ReconError::WienerNsFilterSubclassOutOfRange {
            sample_index: 5,
            subclass: 9,
            classes: 4,
        },
        ReconError::WienerNsFilterSourceSampleOutOfRange {
            x: -3,
            y: 7,
            value: 1100,
            max: 1023,
        },
        ReconError::WienerNsFilterInvalidCflDsFilterIndex { index: 9 },
        ReconError::PcWienerInvalidBounds { field: "row range" },
        ReconError::PcWienerSourceSampleOutOfRange {
            x: -5,
            y: 9,
            value: 1200,
            max: 1023,
        },
        ReconError::PcWienerInvalidTxSkip {
            x: 3,
            y: 5,
            row: 7,
            col: 9,
            value: 2,
        },
        ReconError::LoopRestorationSourceInvalidBounds {
            field: "luma height",
        },
        ReconError::LoopRestorationSourceInvalidSubsampling {
            subsampling_x: 2,
            subsampling_y: 3,
        },
        ReconError::LoopRestorationSourceSubsamplingMismatch {
            plane: PlaneId::U,
            subsampling_x: 1,
            subsampling_y: 0,
            expected_x: 1,
            expected_y: 1,
        },
        ReconError::LoopRestorationSourceFrameMismatch { field: "width" },
        ReconError::LoopRestorationSourceSampleOutOfBounds {
            plane: PlaneId::V,
            x: 3,
            y: 5,
            width: 7,
            height: 9,
        },
        ReconError::InvalidDequantBlockShape {
            tx_width: 5,
            tx_height: 6,
        },
        ReconError::DequantBlockLengthMismatch {
            expected: 64,
            quant_len: 60,
            out_len: 62,
        },
        ReconError::InvalidQuantizerMatrixIndex {
            seg_level: 3,
            qm_offset: 5,
            position: 7,
        },
        ReconError::SubpelReferencePlaneMismatch {
            expected: 64,
            actual: 60,
        },
        ReconError::SubpelBlockDimensionUnsupported { w: 130, h: 140 },
        ReconError::SubpelNegativeStep {
            step_x: -3,
            step_y: -5,
        },
        ReconError::SubpelIntermediateOutOfRange {
            base: 9,
            intermediate_height: 8,
        },
        ReconError::CompoundBlendLengthMismatch {
            left_len: 16,
            right_len: 17,
        },
    ]
}

/// Golden `Display` rendering of each [`all_variants`] entry, in the same order,
/// captured from the pre-`thiserror` hand-written `Display` impl.
const EXPECTED: [&str; 93] = [
    "unsupported AV2 bit_depth_idc 7; expected 0 or 1",
    "unsupported AV2 chroma_format_idc 9; expected 0 through 3",
    "plane width must be greater than zero",
    "arithmetic overflow while deriving chroma width",
    "plane stride 3 samples is smaller than storage width 5",
    "plane buffer length mismatch: expected 11 samples, got 13",
    "visible rectangle x=1 y=2 width=3 height=4 is outside storage 20x30",
    "luma crop origin (5, 7) is not aligned to subsampling (1, 0)",
    "missing required chroma plane U",
    "unexpected chroma plane V for monochrome output",
    "plane U visible size mismatch: expected 8x16, got 9x17",
    "sample type u8 cannot represent 10-bit decoded output",
    "plane V sample 42 value 1000 exceeds maximum 1023",
    "sample value 300 cannot be represented by u8; maximum is 255",
    "failed to allocate current-frame workspace U plane samples",
    "current-frame workspace plane V is not present",
    "current-frame workspace U rectangle x=1 y=2 width=3 height=4 is outside storage 64x48",
    "current-frame workspace U write stride 3 samples is smaller than write width 7",
    "current-frame workspace V write buffer is too small: expected at least 100 samples, got 90",
    "current-frame workspace U copy source 4x8 does not match target 5x9",
    "unsupported square intra block log2 size 7; expected 2 through 6",
    "unsupported rectangular intra block log2 size 7x8; expected each dimension 2 through 6",
    "intra prediction above edge length mismatch: expected 8 samples, got 9",
    "intra prediction left edge sample 3 value 500 exceeds maximum 255",
    "intra prediction output sample 5 value 600 exceeds maximum 255",
    "PAETH intra prediction top-left edge length mismatch: expected 4 samples, got 5",
    "PAETH intra prediction above edge sample 2 value 700 exceeds maximum 255",
    "cardinal directional intra prediction vertical mode requires above edge",
    "cardinal directional intra prediction left edge length mismatch: expected 6 samples, got 7",
    "cardinal directional intra prediction above edge sample 1 value 800 exceeds maximum 255",
    "unsupported one-sided directional intra prediction pAngle 99; expected 45, 67, or 203",
    "directional angle intra prediction pAngle 45 requires above edge",
    "directional angle intra prediction left edge length mismatch: expected 10 samples, got 11",
    "directional angle intra prediction above edge sample 4 value 900 exceeds maximum 255",
    "unsupported middle directional intra prediction pAngle 88; expected 113, 135, or 157",
    "middle directional angle intra prediction pAngle 113 requires above edge",
    "middle directional angle intra prediction left edge length mismatch: expected 12 samples, got 13",
    "middle directional angle intra prediction above edge sample 6 value 950 exceeds maximum 255",
    "smooth intra prediction bottom-left edge length mismatch: expected 14 samples, got 15",
    "smooth intra prediction top-right edge sample 7 value 999 exceeds maximum 255",
    "smooth intra prediction sample at row 3 column 5 value -7 is outside 0..=255",
    "failed to allocate prediction block",
    "intra prediction output stride 3 samples is smaller than prediction width 8",
    "intra prediction output buffer is too small: expected at least 64 samples, got 60",
    "current-frame workspace U intra prediction requires top-left edge for rectangle x=1 y=2 width=3 height=4",
    "current-frame workspace V smooth intra prediction requires bottom-left edge for rectangle x=2 y=3 width=4 height=5",
    "current-frame workspace U cardinal directional intra prediction requires left edge for rectangle x=3 y=4 width=5 height=6",
    "current-frame workspace V directional angle intra prediction pAngle 67 requires above edge for rectangle x=4 y=5 width=6 height=7",
    "current-frame workspace Y directional angle intra prediction pAngle 45 requires luma IDIF for rectangle x=5 y=6 width=7 height=8",
    "reference frame store capacity 20 is outside 1..=16",
    "reference slot index 18 is outside 0..16",
    "reference refresh mask 0x00010001 contains bits outside 0..16",
    "reference slot 5 is outside store capacity 4",
    "reference refresh mask 0x000000ff selects a slot outside store capacity 4",
    "unsupported inverse transform length 7; expected 4, 8, 16, or 32",
    "inverse transform output length 15 does not match source length 16",
    "reconstruct length mismatch: prediction 16, residual 17, output 18",
    "reconstruct prediction sample 9 value 1100 exceeds maximum 1023",
    "unsupported 2D inverse transform log2 shape 7x8 (lossless=true); expected each log2 in 2..=6, and 2x2 (4x4) when lossless",
    "no Transform_Shift entry for log2 shape 7x8; expected one of the 25 AV2 TX_SIZES_ALL shapes",
    "invalid PlaneTxType 20; expected a TX_TYPES index in 0..16",
    "unsupported get_scan shape 5x6; expected each of w/h in 4/8/16/32",
    "get_scan output length mismatch: expected 64, got 60",
    "2D inverse transform buffer mismatch: expected 64, dequant 60, residual 62",
    "2D outer inverse transform buffer mismatch: dequant expected 64 (got 60), residual expected 32 (got 30)",
    "unsupported secondary transform shape 5x6; expected each side 4, 8, 16, or 32",
    "secondary transform buffer mismatch: expected 64, got 60",
    "invalid secondary transform params: n 3, kernel 5, sec_tx_type 7",
    "invalid deblocking filter widths: neg 9, pos 10; expected each in 1..=8",
    "deblocking filter line too short: boundary 2, max_width_neg 3, width 4, len 5",
    "Wiener NS filter output stride 3 samples is smaller than block width 8",
    "Wiener NS filter output buffer too small: expected at least 64 samples, got 60",
    "Wiener NS filter requires at least one coefficient class",
    "Wiener NS subclass map too short: expected at least 64 entries, got 60",
    "Wiener NS subclass 9 at sample 5 is outside 4 coefficient classes",
    "Wiener NS source sample at (-3, 7) has value 1100, exceeding active bit-depth max 1023",
    "invalid Wiener NS chroma cfl_ds_filter_index 9; expected 0..=3",
    "invalid PC-Wiener classification row range",
    "PC-Wiener source sample at (-5, 9) has value 1200, exceeding active bit-depth max 1023",
    "invalid PC-Wiener LrTxSkip value 2 at sample (3, 5) grid (7, 9); expected 0 or 1",
    "invalid loop-restoration source-sample luma height",
    "invalid loop-restoration source-sample subsampling (2, 3); expected each in 0..=1",
    "loop-restoration source-sample U subsampling (1, 0) does not match frame format (1, 1)",
    "loop-restoration source-sample width mismatch between CurrFrame and CdefFrame",
    "loop-restoration source-sample V coordinate (3, 5) is outside coded plane 7x9",
    "unsupported dequantization block shape 5x6; expected each side 4, 8, 16, or 32",
    "dequantization block buffer mismatch: expected 64, quant 60, out 62",
    "quantization-matrix index out of range: seg_level 3, qm_offset 5, position 7",
    "subpel reference plane buffer mismatch: expected 64 samples, got 60",
    "unsupported subpel block dimension 130x140; each side must be at most 128",
    "subpel motion-compensation step must be non-negative: step_x -3, step_y -5",
    "subpel vertical-pass base row 9 exceeds intermediate height 8",
    "compound blend predictor length mismatch: left 16 samples, right 17 samples",
];

#[test]
fn display_matches_pre_migration_messages() {
    let variants = all_variants();
    assert_eq!(
        variants.len(),
        EXPECTED.len(),
        "all_variants() and EXPECTED disagree on length"
    );
    for (error, expected) in variants.iter().zip(EXPECTED) {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn variant_count_is_locked() {
    assert_eq!(all_variants().len(), 93);
}
