# Agent Log: decode-limits-runtime-api

## Orchestrator Plan

- Start the next decoder mission slice after `decoded-frame-plane-runtime-types`
  merged.
- Keep one OpenSpec change in flight and do not create a branch until
  `openspec validate decode-limits-runtime-api --strict` passes.
- Implement only a dependency-free runtime limits/options API in
  `splot-decode`; do not start byte-consuming decode, change CLI behavior, emit
  resource diagnostics, or introduce AVM/dav2d integration.
- Use `DECODE-LIMITS-RUNTIME-API` for the source-backed API row while keeping
  `DOC-DECODE-LIMITS-CONTRACT` as the docs/contract umbrella until real decode
  enforcement exists.

## Planning Agents

### @api-designer

- Agent: `019ec246-d881-73b0-b3b0-eb2bc8b05453` (`Raman the 2nd`)
- Prompt: design a small dependency-free Rust API in `splot-decode` for
  `DecodeLimits`, `DecodeOptions`, typed resource names/units, checked limit
  helpers, and local errors; do not edit files.
- Output:
  - Recommended `DecodeOptions`, `DecodeLimits`, typed resources/units,
    checked add/mul, inclusive checks, and allocation handoff checks.
  - Recommended public `u64` values rather than public `usize`.
  - Recommended local errors with manual `Display`/`Error`, no `thiserror`, no
    `serde`, no diagnostic emission, no CLI behavior.
  - Recommended tests for defaults, builder/update ergonomics, stable names and
    units, limit equality, over-limit failures, overflow, and allocation handoff.

### @architect

- Agent: `019ec24d-48bb-7250-82f0-b61c5b5113b8` (`Curie the 2nd`)
- Prompt: design crate/module boundaries, dependency flow, docs/matrix updates,
  tests, gates, and overclaiming boundaries; do not edit files.
- Output:
  - Recommended keeping `splot-decode` dependency-free and adding a focused
    limits module re-exported from `lib.rs`.
  - Recommended precise names: reference slots rather than reference frames,
    and tile payload bytes rather than generic tile bytes.
  - Recommended a new `DECODE-LIMITS-RUNTIME-API` implementation-matrix row,
    a decoder support matrix row, roadmap updates, generated status updates,
    and no emitted diagnostic registry change.
  - Flagged overclaiming and diagnostic registry drift as the primary risks.

### @spec-reader

- Agent: `019ec24d-4bc4-7281-beb6-b67e098aa532` (`Bernoulli the 2nd`)
- Prompt: read the local AV2 spec mirror and current decoder support docs to
  extract resource-limit surfaces; do not edit files.
- Output:
  - Identified input/OBU byte surfaces from § 4.11.6, Annex B.2-B.3, and
    § 5.2.1.
  - Identified dimensions from § 6.4.1, § 5.18.4.1, § 6.17.4.1,
    § 5.18.4.4, and § 6.17.4.4.
  - Identified tile count/payload surfaces from § 5.18.7.2, § 6.17.7.2,
    § 5.19, § 6.18, § 5.20.1, and § 6.19.1.
  - Identified output surfaces from § 7.1 and § 7.21.
  - Identified reference slot/storage surfaces from § 6.4.6, § 3, and § 7.23.
  - Recommended adding or explicitly accounting for `max_reference_store_bytes`.

### @reference-oracle

- Agent: `019ec24d-4f74-7fd0-95ad-a7ed02c3c6e8` (`Carver the 2nd`)
- Prompt: decide whether local-only AVM/dav2d reading or runs are necessary and
  identify reference boundary risks; do not edit files.
- Output:
  - AVM/dav2d evidence is not required for this API-only slice.
  - The change only source-backs configured `DecodeLimits` and pure local
    comparison/arithmetic helpers. It does not read input bytes, traverse OBUs,
    allocate, reconstruct, hash, write output, emit a resource diagnostic, or
    map failures to user-facing decoder diagnostics.
  - No AVM/dav2d source was read or run.
  - Boundary checks before merge should confirm no AVM/dav2d source, snippets,
    binaries, submodules, dependencies, build probes, wrappers, scripts, CI
    jobs, mandatory tests, fixtures, manifest evidence, or local paths were
    added.

## Decisions So Far

- Use finite defaults despite one architect alternative recommending no default,
  because the mission requires safe defaults and the API request explicitly asks
  for default tests.
- Add `max_reference_store_bytes` to the runtime API because § 7.23 reference
  storage is distinct from visible decoded-frame bytes.
- Keep limit helper errors local and distinct from emitted decoder diagnostics.
- Keep the API names precise: `max_reference_slots`,
  `max_reference_store_bytes`, and `max_tile_payload_bytes`.
- Reject allocation handoff values above `isize::MAX` before converting to
  `usize`, even when the configured policy is `unlimited`.

## Implementation

- Added `crates/splot-decode/src/limits.rs` with `DecodeOptions`,
  `DecodeLimits`, `DecodeLimitName`, `DecodeLimitUnit`,
  `DecodeLimitThreshold`, `DecodeLimitOp`, `DecodeLimitCheck`,
  `DecodeLimitError`, and `DecodeLimitResult`.
- Split the limit tests into `crates/splot-decode/src/limits/tests.rs` so the
  production module stays below the repository's 1000-line soft source budget.
- Re-exported the runtime limits API from `crates/splot-decode/src/lib.rs`
  without changing the existing unsupported diagnostic descriptor.
- Updated `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/IMPLEMENTATION-MATRIX.toml`, and
  generated status docs for `DECODE-LIMITS-RUNTIME-API`.
- No `Cargo.toml`, `Cargo.lock`, CLI behavior, byte-consuming decode path,
  emitted `decode/resource-limit` diagnostic, AVM/dav2d integration, fixture,
  script, wrapper, or CI change was made.

## Local Reference Evidence

No AVM or dav2d commands were run. No local reference evidence is needed for
this slice.

## Review Notes

### Initial Review Agents

- `@reviewer`: `019ec265-9737-71a2-94b3-73278f0da6dc` (`Franklin the 2nd`)
  found incomplete spec-section coverage in matrices/status docs, stale
  OpenSpec method names, and the initial `limits.rs` source-line advisory.
- `@security-reviewer`: `019ec265-9a93-79f2-b7b9-3df2e762819d` (`Bohr the
  2nd`) found that `ensure_allocation_len` accepted values above Rust's
  practical allocation ceiling on 64-bit hosts.
- `@spec-conformance-reviewer`: `019ec265-9d48-7ee3-8ca5-e99b41061191`
  (`Averroes the 2nd`) found missing citations/coverage for § 4.11.6,
  Annex B.2-Annex B.3, § 5.2.1, § 6.18, and § 6.19.1.
- `@encoder-impact-reviewer`: `019ec265-a027-7c12-98cb-5911e41bf92c`
  (`Epicurus the 2nd`) found stale OpenSpec helper names and noted that
  future direct `splot-encode` reuse cannot depend on `splot-decode` under the
  current dependency rules.
- `@test-writer`: `019ec265-a345-75f0-9e08-69453590a8ed` (`Ptolemy the 2nd`)
  found default tests were not pinned to literals and local error text was not
  checked for absence of the planned resource diagnostic id. Its first
  re-review found that only `LimitExceeded` text was checked, so the final test
  also renders `ArithmeticOverflow` and `HostAllocationTooLarge`.
- `@documenter`: `019ec265-a642-7022-9f95-611456d5579d` (`Cicero the 2nd`)
  found the same missing spec coverage, stale OpenSpec helper names, and stale
  agent-log review status.

### Fixes From Review

- Added the missing spec sections/sources to decoder support and implementation
  matrix rows, then regenerated `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.
- Updated the roadmap decode-limits surface paragraph to include input/OBU byte
  surfaces and tile semantics.
- Updated `design.md` to match the implemented API:
  `check`, `ensure`, `ensure_add`, `ensure_mul`, and `ensure_allocation_len`.
- Added `MAX_HOST_ALLOCATION_LEN = isize::MAX as u64`; allocation handoff now
  rejects values above that ceiling before returning `usize`.
- Changed default-threshold tests to assert literal values rather than the
  implementation constants.
- Added a test that rendered `DecodeLimitError` text does not contain the
  planned resource diagnostic id for `LimitExceeded`, `ArithmeticOverflow`, and
  `HostAllocationTooLarge`.
- Split tests into `limits/tests.rs`, leaving both new source files below the
  1000-line soft source budget.

### Final Verification

- `openspec validate decode-limits-runtime-api --strict` passed.
- `cargo test -p splot-decode --locked` passed: 12 tests.
- `cargo clippy -p splot-decode --all-targets --locked -- -D warnings` passed.
- `cargo xtask check-diagnostic-registry` passed: decoder registry still has
  only `decode/unsupported-feature`.
- `cargo xtask check-dependency-direction` passed.
- `cargo xtask check-decoder-support` passed.
- `cargo xtask check-feature-status` passed.
- `cargo xtask ci` passed after review fixes; remaining source-line advisories
  are pre-existing files outside this change.

### Final Re-review

- `@reviewer` final re-review: LGTM; prior coverage, design-name, and
  source-line findings resolved.
- `@security-reviewer` final re-review: LGTM; allocation handoff now rejects
  values above `isize::MAX` before returning `usize`.
- `@spec-conformance-reviewer` final re-review: LGTM; roadmap, decoder support
  rows, implementation matrix, and generated coverage include the missing
  input/OBU and tile-semantics sections.
- `@encoder-impact-reviewer` final re-review: LGTM; API names and coverage are
  fixed, with no dependency or CLI/runtime behavior change.
- `@test-writer` final re-review: LGTM; default literals, allocation tests, and
  all local error variants' text checks are covered.
- `@documenter` final re-review: LGTM for docs/OpenSpec fixes; agent-log status
  was updated by the orchestrator afterward.
