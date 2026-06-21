// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-recon` - AV2 reconstruction model primitives.
//!
//! This crate provides the first decoded output frame and plane model shared by
//! future decoder, frame-hash, Y4M, reference-frame storage, and encoder
//! roundtrip work. The model is limited to immutable owned output frames,
//! plane storage invariants, a safe reference-slot container, and deterministic
//! frame-hash input serialization and digest computation, plus source-backed
//! Y4M writing for caller-supplied decoded frames, plus square DC,
//! rectangular DC, subsampled DC, basic/PAETH, smooth, and H/V cardinal
//! directional intra prediction primitives and a mutable current-frame
//! workspace, plus the AV2 § 7.14.2 dequantization quantizer functions
//! (quantizer-value lookup, quantizer-index resolution, and per-plane DC/AC
//! composition), the § 7.15.2 1D inverse transforms (§ 7.15.2.1 kernel,
//! § 7.15.2.2 Walsh-Hadamard, and § 7.15.2.3 identity), the § 7.14.3
//! residual-addition step, the § 7.14.4 → § 7.15.4 → § 7.14.3 transform-block
//! reconstruction chain composition
//! ([`reconstruct_transform_block_residual`]), the § 7.15.3 secondary inverse
//! transform ([`secondary_inverse_transform`], the § 9.7 IST matrix transform
//! over caller-resolved kernel/transpose), the § 7.15.4.1 2D matrix transform core, the
//! § 7.15.4 outer orchestration (the lossless IDTX shortcut, the DPCM cumulative
//! sum, and adjusted-size sample duplication over caller-resolved transform
//! selections), and the § 7.14.4 dequantization process (the per-coefficient
//! dequant arithmetic, the non-quantization-matrix transform-block helper over
//! caller-resolved quantizers, and the built-in-`Quantizer_Matrix`
//! quantization-matrix weighting over caller-resolved indices), and the
//! § 7.15.4 `Transform_Shift` row/column down-shift lookup keyed on the
//! original `(log2W, log2H)` shape, the § 7.15.4 `get_transform_1d_type`
//! row/column transform-type derivation (the built-in `Transform_1d_Type`
//! table plus the `useDdt` `DDTX`/`FDDT` substitution), the combined § 7.15.4
//! transform-parameter resolve helper ([`InverseTransform2dOuter::resolve`],
//! which derives the per-pass shifts, the per-pass transform types over the
//! adjusted sample sizes, and the `PlaneTxType == IDTX` flag from one
//! `(plane_tx_type, log2W, log2H)` source), the § 7.15.4 DPCM cumulative-sum
//! direction selection ([`dpcm_direction`], `None`/`Vertical`/`Horizontal` from
//! the caller-resolved `useDpcm` and whether the prediction mode is `V_PRED`),
//! and the § 5.20.7.30
//! `get_scan` coefficient scan order (the anti-diagonal 2D scan and the
//! row/column raster scans) with its `get_tx_class` `PlaneTxType`-to-class
//! mapping, the § 7.17.7.1 deblocking sample filter
//! ([`deblock_sample_filter`], the per-edge `deltaM2` ramp over a caller-supplied
//! perpendicular sample line and caller-resolved widths/weights), and the
//! § 7.17.3 deblocking filter-maximum-width derivation
//! ([`deblock_filter_max_width`], the per-side `maxWidthNeg`/`maxWidthPos` from
//! the filter size, plane, and super-block-edge flag), and the § 7.17.5 adaptive
//! filter strength ([`deblock_adaptive_filter_strength`] /
//! [`deblock_side_threshold_index`], the `qThr`/`side` thresholds from the filter
//! level), and the § 7.17.7.2 deblocking filter-choice process
//! ([`deblock_filter_choice`], the chosen filter width from the two perpendicular
//! edge sample lines, the estimated second derivatives, and the `qThr`/`sideThr`
//! threshold cascade over the caller-resolved `Q_First` table); it does not
//! implement the rest of the § 7.17 deblocking edge traversal and § 7.17.6 filter-level derivation,
//! the other loop filters (CDEF, CCSO, loop restoration, GDF), byte-consuming
//! decode, full
//! reconstruction, the § 7.14.4 `shift` derivation
//! or the user-defined `UserQm` matrices,
//! runtime CLI Y4M output, or full AV2
//! reference refresh semantics.
//!
//! The ownership model is view-first ([`docs/ZERO_COPY.md`](../../../docs/ZERO_COPY.md)):
//! owned plane/frame/workspace storage hands out borrowed [`PlaneRef`]/[`PlaneMut`]
//! and [`FrameRef`]/[`FrameMut`] views without copying, immutable frames are shared
//! without copying pixels via [`SharedFrame`], and no media-storage type implements
//! `Clone`.
//!
//! Feature tracking: `INFRA-RECON-FRAME-PLANE-TYPES`,
//! `INFRA-ZERO-COPY-MEDIA-POLICY`,
//! `RECON-REFERENCE-FRAME-STORE`, `RECON-HASH-INPUT-SERIALIZATION`,
//! `RECON-FRAME-HASH-DIGEST`, `RECON-Y4M-OUTPUT-WRITER`,
//! `RECON-INTRA-DC-SQUARE-PREDICTION`,
//! `RECON-INTRA-DC-RECTANGULAR-PREDICTION`,
//! `RECON-INTRA-DC-SUBSAMPLED-PREDICTION`,
//! `RECON-INTRA-IBP-DC-PREDICTION`,
//! `RECON-INTRA-BASIC-PAETH-PREDICTION`,
//! `RECON-INTRA-SMOOTH-PREDICTION`,
//! `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION`,
//! `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION`,
//! `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION`,
//! `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION`,
//! `RECON-CURRENT-FRAME-WORKSPACE`,
//! `RECON-DEQUANT-QUANTIZER-LOOKUP`,
//! `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION`,
//! `RECON-INVERSE-TRANSFORM-1D`,
//! `RECON-INVERSE-TRANSFORM-MATRIX-FREE`,
//! `RECON-RESIDUAL-ADDITION`,
//! `RECON-RECONSTRUCT-TRANSFORM-BLOCK`,
//! `RECON-SECONDARY-INVERSE-TRANSFORM`,
//! `RECON-INVERSE-TRANSFORM-2D`,
//! `RECON-INVERSE-TRANSFORM-2D-OUTER`,
//! `RECON-DEQUANT-PROCESS`,
//! `RECON-DEQUANT-QM-WEIGHT`,
//! `RECON-TRANSFORM-SHIFT-LOOKUP`,
//! `RECON-GET-TRANSFORM-1D-TYPE`,
//! `RECON-RESOLVE-2D-TRANSFORM-PARAMS`,
//! `RECON-DPCM-DIRECTION`,
//! `RECON-COEFFICIENT-SCAN-ORDER`,
//! `RECON-GET-TX-CLASS`,
//! `RECON-DEBLOCK-SAMPLE-FILTER`,
//! `RECON-DEBLOCK-FILTER-MAX-WIDTH`,
//! `RECON-DEBLOCK-ADAPTIVE-STRENGTH`,
//! `RECON-DEBLOCK-FILTER-CHOICE`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod coefficient_scan;
mod deblock_filter;
mod dequant;
mod dequant_process;
mod error;
mod format;
mod frame;
mod geometry;
mod hash_input;
mod intra;
mod intra_basic;
mod intra_dc_math;
mod intra_dc_subsampled;
mod intra_directional;
mod intra_directional_angle;
mod intra_ibp_dc;
mod intra_smooth;
mod inverse_transform;
mod inverse_transform_2d;
mod inverse_transform_2d_outer;
mod plane;
mod reconstruct;
mod reconstruct_block;
mod reference;
mod secondary_transform;
mod transform_params;
mod views;
mod workspace;
mod y4m;

pub use coefficient_scan::{TransformClass, coefficient_scan_order, tx_class};
pub use deblock_filter::{
    DeblockFilterChoice, DeblockSampleFilter, deblock_adaptive_filter_strength,
    deblock_filter_choice, deblock_filter_max_width, deblock_sample_filter,
    deblock_side_threshold_index,
};
pub use dequant::{
    QuantizerDeltas, ac_quantizer, dc_quantizer, max_quantizer_index, quantizer_index,
    quantizer_value,
};
pub use dequant_process::{
    DequantBlockParams, QmWeightIndex, dequant_coefficient, dequantize_block,
    qm_weighted_quantizer, quantization_matrix_weight,
};
pub use error::{ReconError, Result};
pub use format::{BitDepth, PixelFormat, PlaneId, ReconSample};
pub use frame::{DecodedFrame, DecodedFrameInfo, FramePlanes, SharedFrame};
pub use geometry::{OutputIndex, PlaneRect, PlaneSize};
pub use hash_input::{DecodedFrameHash, DecodedFrameHashInput};
pub use intra::{
    IntraDcEdge, IntraDcEdges, IntraRectBlockSize, IntraSquareBlockSize,
    SquareIntraPredictionBlock, SquareIntraPredictionRows, predict_intra_dc_rect_into,
    predict_intra_dc_rect_value, predict_intra_dc_square, predict_intra_dc_square_into,
    predict_intra_dc_square_value,
};
pub use intra_basic::{IntraPaethEdge, IntraPaethEdges, predict_intra_paeth_rect_into};
pub use intra_dc_subsampled::{
    predict_intra_dc_subsampled_rect_into, predict_intra_dc_subsampled_rect_value,
};
pub use intra_directional::{
    IntraCardinalDirection, IntraCardinalEdge, IntraCardinalEdges,
    predict_intra_cardinal_directional_rect_into,
};
pub use intra_directional_angle::{
    IntraDirectionalAngle, IntraDirectionalAngleEdge, IntraDirectionalAngleEdges,
    IntraDirectionalAngleIdifEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, predict_intra_directional_angle_rect_from_p_angle_into,
    predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_from_p_angle_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_middle_directional_angle_rect_from_p_angle_into,
    predict_intra_middle_directional_angle_rect_idif_from_p_angle_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_into,
};
pub use intra_ibp_dc::apply_intra_ibp_dc_rect;
pub use intra_smooth::{
    IntraSmoothEdge, IntraSmoothEdges, IntraSmoothMode, predict_intra_smooth_rect_into,
};
pub use inverse_transform::{
    InverseTransform1dType, inverse_identity_transform, inverse_transform_1d,
    inverse_walsh_hadamard,
};
pub use inverse_transform_2d::{InverseTransform2d, InverseTransform2dDim, inverse_transform_2d};
pub use inverse_transform_2d_outer::{
    DpcmDirection, InverseTransform2dOuter, inverse_transform_2d_outer,
};
pub use plane::{Plane, VisibleRows};
pub use reconstruct::reconstruct_add_residual;
pub use reconstruct_block::reconstruct_transform_block_residual;
pub use reference::{
    ReferenceFrameEntries, ReferenceFrameEntry, ReferenceFrameReplacement, ReferenceFrameStore,
    ReferenceRefreshMask, ReferenceRefreshOutcome, ReferenceRefreshSlots, ReferenceSlot,
};
pub use secondary_transform::{SecondaryInverseTransform, secondary_inverse_transform};
/// The AV2 4-point DCT kernel (`Dct4` basis, `[[i32; 4]; 4]`), re-exported from the
/// generated `splot-tables` § 9 transform tables. The decoder inverse transform
/// ([`inverse_transform_1d`]) and the encoder forward transform (`splot-encode`)
/// both consume this single source, so the forward transform stays the exact
/// numerical inverse of the decoder kernel and a table correction cannot desync the
/// two directions.
pub use splot_tables::tables::transform_1d::DCT_KERNEL4;
pub use transform_params::{TransformPass, dpcm_direction, get_transform_1d_type, transform_shift};
pub use views::{FrameMut, FrameRef, PlaneMut, PlaneMutRows, PlaneRef, PlaneRefRows};
pub use workspace::{
    CurrentFrameIntraEdges, CurrentFramePlane, CurrentFrameWorkspace, WorkspaceRectRows,
};
pub use y4m::{
    Y4mChromaTag, Y4mError, Y4mFrameFormat, Y4mFrameHeader, Y4mFrameRate, Y4mResult,
    Y4mStreamHeader, Y4mWriter,
};
