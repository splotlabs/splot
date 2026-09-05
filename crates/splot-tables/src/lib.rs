// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-tables` - dependency-free generated AV2 § 9 spec tables shared across
//! the splot toolkit.
//!
//! Contains the § 9.6 1D transform and § 9.7 secondary transform kernels,
//! § 9.4 quantizer matrices, and § 9.8 loop-restoration tables. Other § 9
//! tables live in `splot-core::tables`.
//!
//! `cargo xtask gen-tables` generates the tables from the committed spec
//! attachment; `cargo xtask ci` checks for drift.
//!
//! Feature tracking: `INFRA-SHARED-SPEC-TABLES`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

pub mod tables;
