// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <contact@splotlabs.io>

//! AV2 normative tables.
//!
//! Large tables (CDFs, quantizer matrices, scan orders, ...) are intentionally
//! not hand-transcribed. They will be generated from the AV2 v1.0.0 additional
//! tables (`all_tables.h`) by `cargo xtask gen-tables` so that they stay faithful
//! to the spec.
//
// TODO(spec): code-generate tables from AV2 v1.0.0 § 9 (Additional tables).
