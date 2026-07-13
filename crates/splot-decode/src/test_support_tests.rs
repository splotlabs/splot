// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared minimal-tier fixture test helpers.
//!
//! The hash, raw, and Y4M runtime adapters each exercise the same committed
//! minimal fixture; this module hosts the helper they share byte-for-byte to
//! avoid drift between the three test modules.

#![allow(clippy::unwrap_used)]

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect,
    PlaneSize, ReconSample,
};

/// The committed conformant luma-skip fixture exercised by every minimal-tier
/// runtime adapter test.
pub(crate) const MINIMAL_FIXTURE: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");

/// Returns [`MINIMAL_FIXTURE`] with its IVF time base rewritten to
/// `numerator / denominator`.
pub(crate) fn minimal_fixture_with_timebase(numerator: u32, denominator: u32) -> Vec<u8> {
    let mut bytes = MINIMAL_FIXTURE.to_vec();
    bytes[16..20].copy_from_slice(&denominator.to_le_bytes());
    bytes[20..24].copy_from_slice(&numerator.to_le_bytes());
    bytes
}

/// Returns the header-only IVF emitted by AVM when no input frames are encoded.
pub(crate) fn empty_avmenc_ivf() -> Vec<u8> {
    include_bytes!("../../../tests/conformance/vectors/valid/syn-empty-avmenc-64x64.ivf").to_vec()
}

pub(crate) fn yuv420_workspace_with<T: ReconSample>(
    bit_depth: BitDepth,
    width: usize,
    height: usize,
    fill: T,
) -> CurrentFrameWorkspace<T> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    CurrentFrameWorkspace::<T>::new(info, fill).unwrap()
}

pub(crate) fn yuv420_workspace(width: usize, height: usize, fill: u8) -> CurrentFrameWorkspace<u8> {
    yuv420_workspace_with(BitDepth::Eight, width, height, fill)
}
