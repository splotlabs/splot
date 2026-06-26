// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared minimal-tier runtime test helpers.
//!
//! The hash, raw, and Y4M runtime adapters each exercise the same committed
//! minimal fixture; this module hosts the helper they share byte-for-byte to
//! avoid drift between the three test modules.

#![allow(clippy::unwrap_used)]

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
