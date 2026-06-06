// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-validate` — turns [`splot_core`] parser output into structured AV2
//! conformance diagnostics.
//!
//! Diagnostics are the product: every finding has a stable [`Diagnostic::rule_id`],
//! a [`Severity`], an optional spec section, an optional byte/bit offset, and a
//! human-readable message. A malformed bitstream is a [`ValidationReport`], never
//! a process failure.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate license.

pub mod checks;
pub mod diagnostic;
pub mod validator;

pub use diagnostic::{Diagnostic, Severity, ValidationReport};
pub use validator::Validator;
