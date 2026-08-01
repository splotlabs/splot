// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `splot-simd` - quarantined hand-scheduled SIMD kernels for the splot AV2
//! toolkit.
//!
//! # Why this crate exists
//!
//! Every other crate in this workspace keeps `unsafe_code = "forbid"`. This one
//! does not, and it is the only one that may not. It exists so that the
//! exception has exactly one home, one dependency-free leaf position in the
//! crate graph, and a boundary that is entirely safe to call.
//!
//! The decode-scaling ledger closed the safe-Rust route to the remaining
//! kernel gap in `docs/DECODE-SCALING-MISSION.md`: `core::arch::aarch64`
//! intrinsics are safe `fn`s on this toolchain but carry
//! `#[target_feature(enable = "neon")]`, and rustc will not honour the target's
//! own baseline in place of the attribute, so the first plain-safe caller needs
//! `unsafe` even though nothing about the call is memory-unsafe (SCALE-058).
//! `std::simd` reaches the constant-index `ext` window shape through
//! `simd_swizzle!` (SCALE-059) but cannot express a window base chosen at run
//! time, which is what the AV2 § 7.13.3.18 active-tap-span pruning needs.
//!
//! # Rules for this crate
//!
//! - No `splot-*` dependency and no external dependency: it is a leaf, like
//!   `splot-tables`.
//! - Every public item is safe to call from safe code. Callers pass slices;
//!   this crate does the bounds arithmetic and refuses shapes it cannot serve.
//! - Every `unsafe` block carries a `SAFETY:` comment
//!   (`clippy::undocumented_unsafe_blocks` is `deny` here) and
//!   `unsafe_op_in_unsafe_fn` is `deny`, so a `#[target_feature]` function body
//!   is not an implicit unsafe block.
//! - Every kernel has a portable reference in this crate that is compiled on
//!   every target, is the implementation on targets without the hand-scheduled
//!   path, and is the differential-test oracle for the hand-scheduled path.
//! - A dispatching entry point returns `false` when this build has no kernel
//!   for the shape, so the caller keeps its own portable path and the crate is
//!   never load-bearing for correctness.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.
//!
//! Licensed under PolyForm Noncommercial 1.0.0; commercial use requires a
//! separate written license from Bartosz Tomczyk.
