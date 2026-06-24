# Agent Coding Standards

Use this file for implementation rules that apply across Rust crates. The review
checklist is [../CODE_REVIEW.md](../CODE_REVIEW.md).

## Library First

`splot-cli` parses arguments, sets up logging, reads/writes files, and calls
library APIs. Codec logic, validation logic, parsing, and diagnostics live in
library crates.

## Error Handling

- Libraries use typed errors with `thiserror`.
- `anyhow` is allowed only in `splot-cli` and `xtask`.
- Malformed input returns errors or diagnostics; it must not panic.
- Recognized but unimplemented functionality returns
  `Error::Unimplemented { feature }` or a structured `Diagnostic`.

## Panic Policy

No `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` may be reachable in
library code.

Tests may use `unwrap` or `expect` only inside test-only code with the local
allowance pattern described in [../TESTING.md](../TESTING.md).

## Types and Public API

- Use newtypes and enums at public boundaries, for example `ObuType`,
  `TemporalLayerId`, and `ByteOffset`.
- Avoid bare integers in public APIs when a domain type exists or should exist.
- Every public item has a doc comment.
- Every crate has crate-level `//!` docs.

## SPDX Headers

Every `.rs` file starts with:

```rust
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
```

Enforcement:

```bash
cargo xtask check-license-headers
```

## Source Size

Rust source files should stay at or below 1000 physical lines. The hard cap is
2500 lines unless `xtask/src/source_lines.rs` records a temporary allowance and
rationale.

Enforcement:

```bash
cargo xtask check-source-lines
```

Split large files by responsibility before adding more code. Do not grow an
allowlisted file past its recorded cap without deliberately updating the
allowance and rationale.

## Unsafe and SIMD

`unsafe_code = "forbid"` across the workspace. Future SIMD or FFI must live
behind narrowly scoped, documented, tested modules and requires maintainer
approval.

## Concurrency and Zero-Copy

For concurrency and media-buffer ownership, follow
[architecture.md](./architecture.md), [../CONCURRENCY.md](../CONCURRENCY.md),
and [../ZERO_COPY.md](../ZERO_COPY.md).
