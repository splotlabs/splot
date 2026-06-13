// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared helpers for validator-context submodules.

/// Computes a stable 64-bit FNV-1a fingerprint over an OBU payload's bytes.
///
/// Used to compare OBU payloads for bit identity without pulling in a hashing
/// dependency: repeated activated sequence headers (AV2 § 7.3.6) and the non-RAP
/// MSDO identity rule (§ 7.3.8.2).
pub(super) fn payload_fingerprint(payload: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in payload {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
