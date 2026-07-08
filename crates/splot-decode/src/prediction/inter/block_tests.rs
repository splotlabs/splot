// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::headers::sequence::ChromaFormatIdc;
use splot_core::span::ByteOffset;

use super::{
    chroma_smooth_grid_dimensions, ensure_intra_leaf_quantizer_delta_scope,
    inter_residual_geometry_supported_flags,
};
use crate::error::DecodeError;
use crate::prediction::inter::SPEC_MODE_INFO;

#[test]
fn inter_residual_geometry_allows_shared_leaves() {
    assert!(inter_residual_geometry_supported_flags(false, false));
}

#[test]
fn inter_residual_geometry_rejects_chroma_partitioned_leaves() {
    assert!(!inter_residual_geometry_supported_flags(true, false));
    assert!(!inter_residual_geometry_supported_flags(false, true));
}

#[test]
fn inter_frame_intra_leaf_rejects_nonzero_quantizer_deltas() {
    let result = ensure_intra_leaf_quantizer_delta_scope(false, false, ByteOffset::new(13));
    assert!(matches!(
        &result,
        Err(DecodeError::UnsupportedFeature { unsupported })
            if unsupported.reason() == "inter_block_intra_leaf_nonzero_quantizer_delta"
                && unsupported.spec_section() == SPEC_MODE_INFO
                && unsupported.byte_offset() == Some(ByteOffset::new(13))
    ));
}

#[test]
fn intra_leaf_quantizer_delta_guard_allows_installed_or_zero_delta_scope() {
    assert!(ensure_intra_leaf_quantizer_delta_scope(true, false, ByteOffset::new(0)).is_ok());
    assert!(ensure_intra_leaf_quantizer_delta_scope(false, true, ByteOffset::new(0)).is_ok());
}

#[test]
fn chroma_smooth_grid_dimensions_follow_chroma_sampling() {
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv420),
        (9, 10)
    );
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv422),
        (17, 10)
    );
    assert_eq!(
        chroma_smooth_grid_dimensions(17, 19, ChromaFormatIdc::Yuv444),
        (17, 19)
    );
}
