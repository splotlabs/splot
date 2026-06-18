// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Private compile-time boundary to keep the `splot-recon` dependency explicit.

#[inline]
pub(crate) fn dependency_marker() -> usize {
    core::mem::size_of::<splot_recon::DecodedFrameInfo>()
}
