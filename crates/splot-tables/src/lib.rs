// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-tables` - dependency-free generated AV2 § 9 spec tables shared across
//! the splot toolkit.
//!
//! This crate is the single dependency-free home for the AV2 § 9 transform-kernel
//! tables — the § 9.6 1D transform and § 9.7 secondary transform tables — that
//! `splot-recon` needs for the § 7.15 inverse transform but cannot reach through
//! `splot-core` (the one-way dependency rule forbids `splot-recon` from depending
//! on `splot-core`). Every other § 9 table stays in `splot-core::tables`.
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
