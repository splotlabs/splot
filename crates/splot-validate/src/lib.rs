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
//! separate written license from Bartosz Tomczyk.

pub mod checks;
pub mod diagnostic;
pub mod options;
pub mod render;
pub mod validator;

mod annex_a;
mod celu;
mod context;
mod error_location;
mod frame_unit;
mod metadata_lifetime;
mod reference_state;

pub use diagnostic::{Diagnostic, Severity, ValidationReport};
pub use options::{ExternalHlsMode, ExternalHlsSet, ValidationOptions};
pub use render::{RenderOptions, RenderedReport, ReportSummary, Truncation};
pub use validator::Validator;
