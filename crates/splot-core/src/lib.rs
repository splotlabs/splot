// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-core` — the AV2 (AOMedia Video 2) bitstream model and parsing
//! foundation for the [`splot`](https://github.com/splotlabs/splot) toolkit.
//!
//! This crate models the AV2 v1.0.0 bitstream envelope and OBU headers with
//! strong types and panic-free parsers:
//!
//! - [`leb128`] — LEB128 unsigned integers (AV2 § 4.11.6)
//! - [`obu`] — OBU headers (AV2 § 5.2.2)
//! - [`annexb`] — Annex B length-delimited envelopes (AV2 Annex B)
//! - [`ivf`] — IVF container envelopes around Annex B payloads
//!   (`AV2-IVF-CONTAINER`)
//! - [`types`] — strongly-typed `obu_type` and layer ids (AV2 Table 6.1, § 6.2.2)
//!
//! Design rules enforced here:
//!
//! - `splot-core` depends on no other `splot-*` crate.
//! - Library code never panics on malformed input; failures are [`Error`]s.
//! - This is the **AV2** OBU header (§ 5.2.2), not the AV1 OBU header.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.

pub mod annexb;
pub mod bitio;
pub mod error;
pub mod headers;
pub mod hls;
pub mod ivf;
pub mod leb128;
pub mod obu;
pub mod segment;
pub mod span;
pub mod stream;
pub mod tables;
pub mod tile;
pub mod types;

pub use error::{Error, Result};
