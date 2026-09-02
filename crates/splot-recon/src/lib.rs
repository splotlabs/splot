// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![feature(portable_simd)]

//! `splot-recon` owns AV2 reconstruction primitives and frame/workspace storage.
//!
//! The crate exposes checked media buffers, view-first frame access, intra/inter
//! prediction kernels, inverse-transform/dequant helpers, and loop-filter
//! primitives. Byte parsing and runtime scheduling stay outside this crate.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

mod cdef_filter;
mod coefficient_scan;
mod deblock_filter;
mod dequant;
mod dequant_process;
mod error;
mod film_grain;
mod format;
mod frame;
mod geometry;
mod hash_input;
mod intra;
mod intra_basic;
mod intra_dc_math;
mod intra_dc_subsampled;
mod intra_dip;
mod intra_directional;
mod intra_directional_angle;
mod intra_ibp_angular;
mod intra_ibp_dc;
mod intra_smooth;
mod inverse_transform;
mod inverse_transform_2d;
mod inverse_transform_2d_outer;
mod loop_restoration;
pub mod math;
#[doc(hidden)]
pub mod mhccp;
mod optflow;
mod pc_wiener;
mod plane;
mod plane_buffers;
mod reconstruct;
mod reconstruct_block;
mod reference;
mod sample_range;
mod secondary_transform;
mod subpel_mc;
mod transform_params;
mod views;
mod warp_prediction;
mod wienerns_chroma_filter;
mod wienerns_filter;
mod workspace;
mod y4m;

pub use cdef_filter::{
    CDEF_DIRECTIONS, CDEF_PADDED_AREA, CDEF_PADDED_SIDE, CDEF_PAIR_OUTPUT, CDEF_PAIR_STRIDE,
    CDEF_UNAVAILABLE, CDEF_UV_DIR, CdefBlockFilter, CdefSampleTaps, CdefTap, cdef_constrain,
    cdef_direction, cdef_direction_padded, cdef_filter_block_boundary_to_valid_stride,
    cdef_filter_block_chroma_pair, cdef_filter_block_interior, cdef_filter_block_interior_to,
    cdef_filter_block_interior_to_valid_stride, cdef_filter_sample,
};
pub use coefficient_scan::{
    TransformClass, coefficient_scan_order, coefficient_scan_slice, tx_class,
};
pub use deblock_filter::{
    DeblockFilterChoice, DeblockSampleFilter, deblock_adaptive_filter_strength,
    deblock_filter_choice, deblock_filter_choice_and_sample_strided_4,
    deblock_filter_choice_and_sample_strided_4_fast_validated, deblock_filter_choice_strided,
    deblock_filter_max_width, deblock_sample_filter, deblock_sample_filter_strided,
    deblock_sample_filter_strided_4, deblock_side_threshold_index,
};
pub use dequant::{
    QuantizerDeltas, ac_quantizer, dc_quantizer, max_quantizer_index, quantizer_index,
    quantizer_value,
};
pub use dequant_process::{
    DequantBlockParams, QmDequant, QmFrameLevels, QmUserPlane, QmWeightIndex, dequant_coefficient,
    dequantize_block, qm_weighted_quantizer, quantization_matrix_weight,
};
pub use error::{ReconError, Result};
pub use film_grain::apply_film_grain;
pub use format::{BitDepth, PixelFormat, PlaneId, ReconSample};
pub use frame::{DecodedFrame, DecodedFrameInfo, FramePlanes, SharedFrame};
pub use geometry::{OutputIndex, PlaneRect, PlaneSize};
pub use hash_input::{DecodedFrameHash, DecodedFrameHashInput, visible_byte_len};
pub use intra::{
    IntraDcEdge, IntraDcEdges, IntraRectBlockSize, IntraSquareBlockSize,
    SquareIntraPredictionBlock, SquareIntraPredictionRows, predict_intra_dc_rect_into,
    predict_intra_dc_rect_value, predict_intra_dc_square, predict_intra_dc_square_into,
    predict_intra_dc_square_value,
};
pub use intra_basic::{IntraPaethEdge, IntraPaethEdges, predict_intra_paeth_rect_into};
pub use intra_dc_math::resolve_divisor;
pub use intra_dc_subsampled::{
    predict_intra_dc_subsampled_rect_into, predict_intra_dc_subsampled_rect_value,
};
pub use intra_dip::{IntraDipEdge, IntraDipEdges, predict_intra_dip_rect_into};
pub use intra_directional::{IntraCardinalDirection, predict_intra_cardinal_directional_rect_into};
pub use intra_directional_angle::{
    IntraDirectionalAngle, IntraDirectionalAngleEdge, IntraDirectionalAngleEdges,
    IntraDirectionalAngleIdifEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraMiddleDirectionalAngleIdifMrlEdges,
    apply_intra_edge_filter, filter_intra_edge_corner, predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_into,
};
pub use intra_ibp_angular::{apply_ibp_dr_blend_rect, ibp_blend_fires};
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
    inverse_transform_2d_outer_adjusted,
};
pub use loop_restoration::{
    LoopRestorationSource, LoopRestorationSourceBounds, LoopRestorationSourceSample,
    LoopRestorationSourceSampleValue, loop_restoration_source_sample,
    loop_restoration_source_sample_value,
};
pub use optflow::{
    OptflowScratch, derive_optflow_mv_delta_8x8_strided_into, derive_optflow_mv_deltas,
    derive_optflow_mv_deltas_into,
};
pub use pc_wiener::{
    PC_WIENER_CLASSIFY_READ_RADIUS, PC_WIENER_FEATURE_WINDOW_SIDE, PC_WIENER_FILTER_TAP_RADIUS,
    PC_WIENER_FULL_CLASSES, PC_WIENER_LUT_CLASSES, PC_WIENER_LUT_INPUTS, PC_WIENER_NUM_FEATURES,
    PcWienerClassification, PcWienerClassifyPaddedSource, PcWienerClassifyParams,
    PcWienerClassifyScratch, PcWienerFilter, PcWienerPaddedSource, PcWienerTxSkipLookup,
    pc_wiener_classify, pc_wiener_classify_grid, pc_wiener_classify_grid_padded,
    pc_wiener_classify_grid_padded_classes_into, pc_wiener_classify_grid_padded_into,
    pc_wiener_filter_block, pc_wiener_filter_block_padded, pc_wiener_filter_block_padded_u16_into,
    pc_wiener_filter_set_index, pc_wiener_subclass_table,
};
pub use plane::{Plane, VisibleRows};
pub use reconstruct::reconstruct_add_residual;
pub use reconstruct_block::{
    reconstruct_transform_block_residual, reconstruct_transform_block_residual_with_secondary,
};
pub use reference::{
    ReferenceFrameEntries, ReferenceFrameEntry, ReferenceFrameReplacement, ReferenceFrameStore,
    ReferenceRefreshMask, ReferenceRefreshOutcome, ReferenceRefreshSlots, ReferenceSlot,
};
pub use secondary_transform::{SecondaryInverseTransform, secondary_inverse_transform};
/// Re-export of the § 9.4 `Qm_Offset[txSz]` table so decode crates can resolve a
/// transform block's built-in quantizer-matrix region without depending on
/// `splot-tables` directly.
pub use splot_tables::tables::quantizer::QM_OFFSET;
pub use subpel_mc::{
    InterpolationFilter, ReferencePlaneView, SUBPEL_FILTERS, SubpelPredictParams,
    blend_compound_average_equal, blend_compound_average_weighted,
    blend_compound_average_weighted_sample, subpel_predict_16x16_bilinear_horizontal_overlap_into,
    subpel_predict_block, subpel_predict_block_compound_average_fast_validated_strided_into,
    subpel_predict_block_compound_average_fullpel_strided_into_u8,
    subpel_predict_block_compound_average_into, subpel_predict_block_compound_average_strided_into,
    subpel_predict_block_compound_average_strided_into_u8,
    subpel_predict_block_compound_intermediate, subpel_predict_block_compound_intermediate_into,
    subpel_predict_block_into, subpel_predict_block_strided_into,
    subpel_predict_block_strided_into_u8,
};
pub use transform_params::{
    TransformPass, dpcm_direction, get_transform_1d_type, transform_shift, tx_size_index,
};
pub use views::{FrameMut, FrameRef, PlaneMut, PlaneMutRows, PlaneRef, PlaneRefRows};
pub use warp_prediction::{
    IDENTITY_WARP_PARAMS, PreparedWarpPrediction, WARPED_BLOCK_SIZE, WarpPredictBlockParams,
    ext_warp_predict_unit, warp_predict_block, warp_predict_block_into, warp_shear_is_valid,
};
pub use wienerns_chroma_filter::{
    WIENER_NS_CHROMA_COEFFS, WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_CHROMA_TAPS,
    WienerNsChromaFilter, WienerNsChromaPaddedSource, WienerNsChromaScratch,
    wiener_ns_filter_chroma_block, wiener_ns_filter_chroma_block_padded_into,
    wiener_ns_filter_chroma_block_padded_u8_into, wiener_ns_filter_chroma_block_padded_u16_into,
};
pub use wienerns_filter::{
    WIENER_NS_LUMA_COEFFS, WIENER_NS_LUMA_TAP_RADIUS, WIENER_NS_LUMA_TAPS, WienerNsLumaFilter,
    WienerNsLumaPaddedSource, WienerNsLumaScratch, wiener_ns_filter_luma_block,
    wiener_ns_filter_luma_block_padded, wiener_ns_filter_luma_block_padded_cells_into,
    wiener_ns_filter_luma_block_padded_cells_u8_into,
    wiener_ns_filter_luma_block_padded_cells_u16_into, wiener_ns_filter_luma_block_padded_into,
    wiener_ns_filter_luma_block_padded_u8_into, wiener_ns_filter_luma_block_padded_u16_into,
};
pub use workspace::{
    CurrentFrameIntraEdges, CurrentFramePlane, CurrentFramePlaneRect, CurrentFrameRect,
    CurrentFrameRectRows, CurrentFrameRectRowsMut, CurrentFrameSurface, CurrentFrameWorkspace,
    InterIntraMode, IntraPredictionScratch, IntraPredictionScratchBuffer, OwnedFrameRect,
    OwnedFrameRectRows, WorkspaceRectRows, wedge_mask_plane_sample,
};
pub use y4m::{
    Y4mChromaTag, Y4mError, Y4mFrameFormat, Y4mFrameHeader, Y4mFrameRate, Y4mResult,
    Y4mStreamHeader, Y4mWriter,
};
