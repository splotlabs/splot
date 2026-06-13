// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-decode` - scaffold for the future AV2 decode driver.
//!
//! This crate will coordinate parsed AV2 bitstream facts from `splot-core` with
//! reconstruction and output state from `splot-recon`. It intentionally exposes
//! no byte-consuming decode API yet, and current `splot decode` CLI behavior
//! remains the CLI-owned `decode/unsupported-feature` diagnostic.
//!
//! Feature tracking: `INFRA-DECODER-CRATE-SCAFFOLDING`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.
