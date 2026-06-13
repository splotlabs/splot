# Agent Log: recon-hash-input-serialization

## Orchestrator

- Objective: add dependency-free canonical decoded-frame hash input
  serialization in `splot-recon`.
- Feature ID: `RECON-HASH-INPUT-SERIALIZATION`.
- Branch: `codex/recon-hash-input`.
- Constraints: no AVM/dav2d repo integration, no new dependencies, no new crate
  dependency edges, no byte-consuming decode, no SHA-256 digest computation.

## Plan

1. Scaffold and validate OpenSpec artifacts.
2. Use read-only planning agents for architecture, spec citations, and API
   shape.
3. Implement `DecodedFrameHashInput<'_, T>` in `splot-recon`.
4. Update docs, matrix, generated status, and feature tracking.
5. Run focused checks, full CI, and review agents before PR.

## Agents

| Role | Agent ID | Objective | Status |
|---|---|---|---|
| `@architect` | `019ec29d-ebc4-7382-b3a3-a945ad6f9d50` | Architecture and dependency-boundary plan | complete |
| `@spec-reader` | `019ec29e-001b-7211-a055-061213194058` | AV2 decoded-frame-hash byte-stream citation review | complete |
| `@api-designer` | `019ec29e-1456-7280-bd9a-ccb2d2784547` | Public API shape and tests | complete |
| `@reviewer` | `019ec2aa-5130-7fa1-bd1a-5a56fc614c28` | Final code/API/docs review | complete |
| `@security-reviewer` | `019ec2aa-6755-7451-a714-596ec0819a57` | Security, reliability, panic, dependency, and boundary review | complete |
| `@spec-conformance-reviewer` | `019ec2aa-811b-7e20-a604-ecb390e4b0b2` | AV2 spec-conformance and claim-honesty review | complete |
| `@encoder-impact-reviewer` | `019ec2aa-971e-7f32-ba65-cab629250808` | Future encoder reuse and dependency-boundary review | complete |
| `@test-writer` | `019ec2aa-ae59-7d70-9f00-46283bdee41a` | Test coverage review and re-review | complete |
| `@documenter` | `019ec2aa-c86b-7af0-aa4c-030a5a063013` | Docs, matrix, generated status, and OpenSpec review | complete |

## Findings

- `@api-designer`: implement a byte-stream serializer, not a hasher. Recommended
  `DecodedFrameHashInput<'a, T>` with `BYTE_STREAM_ID`, `VARIANT_ID`,
  infallible `new`, checked `byte_len`, and `write_to<W: std::io::Write>`.
  Keep SHA-256, MD5, AVM/dav2d, and new dependencies out of scope. Cover visible
  rows, plane order, bit-depth byte widths, metadata exclusion, and writer errors
  in unit tests.
- `@spec-reader`: a no-digest serializer may honestly claim only the § 6.16.13
  sample-to-byte conversion input for an already materialized decoded output
  frame. It must not claim MD5/hash verification, output ordering, decode order,
  film-grain support, or AVM/dav2d proof. Use frame bit depth rather than Rust
  storage type, preserve chroma round-up, and exclude stride/padding,
  metadata, OBU/container bytes, timestamps, and color conversion.
- `@architect`: place implementation in `crates/splot-recon/src/hash_input.rs`,
  keep plane serialization private, keep `splot-recon` dependency-empty, and
  avoid `sha2`, `splot-core`, or `splot-decode` edges. The architect suggested a
  free function; the orchestrator chose the API-designer wrapper because it
  gives the byte-stream identifiers and checked byte length a stable home while
  still keeping the byte stream digest-free and plane serialization private.

## Implementation Notes

- Added `crates/splot-recon/src/hash_input.rs` with:
  - `DecodedFrameHashInput<'_, T>`;
  - `BYTE_STREAM_ID == "av2-output-samples-v1"`;
  - `VARIANT_ID == "raw_intermediate_output"`;
  - checked `byte_len()`;
  - writer-based `write_to()` over modeled visible rows;
  - private Y/U/V plane traversal and bit-depth-driven sample serialization.
- Exported `DecodedFrameHashInput` from `crates/splot-recon/src/lib.rs` and
  updated crate docs/feature tracking comments.
- Kept `crates/splot-recon/Cargo.toml` unchanged; no digest dependency or crate
  dependency graph change was added.
- Added unit tests covering stable identifiers, visible-row serialization that
  excludes stride/padding, 8-bit `u16` storage emitting one byte per sample,
  10-bit little-endian output, YUV420 odd-dimension Y/U/V order, YUV422/YUV444
  byte lengths, metadata/coded-padding exclusion, and writer error propagation.
- After `@test-writer` review, tightened the YUV422/YUV444 coverage to assert
  full emitted byte vectors with distinct Y/U/V values, proving `write_to()`
  plane order as well as `byte_len()`.
- Updated decoder roadmap, decoder support matrix, feature matrix, generated
  decoder support status, feature status, and spec coverage.

## Verification

- `openspec validate recon-hash-input-serialization --strict`: passed.
- `openspec validate --all --no-interactive`: passed, 15 items.
- `cargo test -p splot-recon --locked`: passed, 40 tests plus doctests.
- `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`:
  passed.
- `cargo xtask check-dependency-direction`: passed.
- `cargo xtask check-feature-status`: passed, 153 features.
- `cargo xtask check-decoder-support`: passed, 22 rows.
- `cargo xtask ci`: passed.
- Post-review fix verification:
  - `cargo fmt --all -- --check`: passed.
  - `cargo test -p splot-recon --locked hash_input`: passed, 8 tests.
  - `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`:
    passed.

## Review

- `@reviewer`: LGTM. Re-ran focused recon tests, clippy, dependency direction,
  feature status, decoder support, OpenSpec validation, and diff whitespace
  checks.
- `@security-reviewer`: LGTM. No unsafe code, dependency changes, AVM/dav2d
  integration, local path leaks, library panics, unchecked arithmetic, or writer
  error handling blockers found.
- `@spec-conformance-reviewer`: LGTM. Confirmed the implementation and docs stay
  limited to § 6.16.13 hash-input byte serialization over § 7.21.2 modeled
  visible output rows, with no SHA/MD5 verification, output ordering, or
  film-grain synthesis claim.
- `@encoder-impact-reviewer`: LGTM. Confirmed the API remains useful for future
  encoder roundtrip work without dependency cycles, digest lock-in, or reference
  tool integration.
- `@test-writer`: initially found that YUV422/YUV444 tests checked only
  `byte_len()` with zero-filled planes. Fixed by asserting full serialized
  vectors with distinct Y/U/V values; re-review LGTM.
- `@documenter`: no docs/content blockers for generated-status drift, local path
  leaks, AVM/dav2d boundary language, or hash-input-vs-digest overclaims. The
  only finding was to record final sign-offs and complete task 5.6, addressed
  here.

## PR Review Follow-up

- Claude review initially recommended merge with one non-blocking test-coverage
  nit: add a 10-bit multi-plane case. Fixed with
  `ten_bit_yuv_samples_emit_little_endian_y_then_u_then_v`; Claude re-review
  confirmed the nit was resolved and again recommended merge.
- Codex review suggested adding pinned spec mirror paths to public AV2 citations.
  Fixed the `DecodedFrameHashInput` type docs and `VARIANT_ID` docs to include
  the corresponding mirror anchors.
- Codex review suggested batching sample writes before calling the caller's
  writer. Fixed `write_to()` to reuse a row buffer and write one visible row at a
  time, with a regression test covering YUV row batching.
- Follow-up verification after Codex review fixes:
  - `cargo fmt --all -- --check`: passed.
  - `cargo test -p splot-recon --locked hash_input`: passed, 10 tests.
  - `cargo clippy -p splot-recon --all-targets --locked -- -D warnings`:
    passed.
  - `openspec validate --all --no-interactive`: passed, 14 items.
  - `git diff --check`: passed.
  - `cargo xtask ci`: passed.
