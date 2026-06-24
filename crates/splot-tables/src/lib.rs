// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-tables` - dependency-free generated AV2 § 9 spec tables shared across
//! the splot toolkit.
//!
//! This crate is the single dependency-free home for the AV2 § 9 tables that
//! `splot-recon` needs but cannot reach through `splot-core` (the one-way
//! dependency rule forbids `splot-recon` from depending on `splot-core`): the
//! § 9.6 1D transform and § 9.7 secondary transform kernel tables (for the
//! § 7.15 inverse transform) and the § 9.4 quantizer-matrix tables (for the
//! § 7.14.4 dequantization), plus the § 9.8 loop-restoration tables needed by
//! § 7.20.4 pixel-classified Wiener classification. Other § 9 tables stay in
//! `splot-core::tables`.
//!
//! The tables are generated verbatim from the committed spec attachment by
//! `cargo xtask gen-tables` (drift-checked in `cargo xtask ci`); this crate is
//! never hand-edited. It depends on no other `splot-*` crate and no external
//! crate, so any crate may depend on it without affecting the dependency
//! direction.
//!
//! Feature tracking: `INFRA-SHARED-SPEC-TABLES`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

pub mod tables;
