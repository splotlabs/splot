## Context

The decoder support docs already require future byte-consuming decode entry
points to accept `DecodeOptions { limits: DecodeLimits }` before allocating from
bitstream-derived values. Today that contract is prose only:
`crates/splot-decode` exposes the current unsupported-feature diagnostic
descriptor, but it has no typed options, limit names, units, defaults, checked
comparison helpers, or local errors.

This change source-backs the contract in `splot-decode` without starting a
decode path. The API is repository resource policy layered over AV2-derived
measured values, not AV2 conformance itself. Relevant measured-value surfaces
come from the local AV2 mirror: OBU/input sizing (§ 4.11.6, Annex B.2-B.3,
§ 5.2.1), sequence and per-frame dimensions (§ 6.4.1, § 5.18.4.1,
§ 6.17.4.1, § 5.18.4.4, § 6.17.4.4), tiles (§ 5.18.7.2, § 6.17.7.2, § 5.19,
§ 6.18, § 5.20.1, § 6.19.1), output frames (§ 7.1, § 7.21), and reference
state (§ 6.4.6, § 7.23).

## Goals / Non-Goals

**Goals:**

- Add a dependency-free `splot-decode` runtime API for `DecodeOptions`,
  `DecodeLimits`, typed limit names, typed units, finite defaults, inclusive
  limit checks, checked arithmetic helpers, allocation-size handoff checks, and
  local errors.
- Keep all public limit quantities as `u64` so the policy API is stable across
  32-bit and 64-bit hosts.
- Use tests to pin defaults, builder/update ergonomics, resource names and
  units, limit comparison behavior, arithmetic overflow, and platform handoff
  boundaries where applicable.
- Update decoder support and implementation matrix status to say that the
  limits contract has a runtime API while byte-consuming enforcement remains
  unintegrated.

**Non-Goals:**

- No byte-consuming decode API, parser-to-decoder planner, reconstruction,
  decoded-frame hash, Y4M writer, reference-frame store, CLI option, or CLI
  behavior change.
- No emitted `decode/resource-limit` diagnostic and no new `DecodeDiagnostic`
  descriptor. The new errors are local typed helper errors only.
- No new dependency, serde requirement, AVM/dav2d source read, local reference
  run, wrapper, script, fixture, CI job, or executable reference integration.
- No claim that default thresholds are AV2 conformance limits. AV2 supplies
  measured values; `splot` supplies configured thresholds.

## Decisions

1. **Keep the API in `splot-decode` and dependency-free.**

   Add a focused `crates/splot-decode/src/limits.rs` module and re-export its
   public types from `lib.rs`. This keeps the existing unsupported diagnostic
   descriptor stable and avoids adding a dependency on `splot-core`,
   `splot-recon`, `splot-cli`, `serde`, or `thiserror`.

   Alternative considered: split `error.rs`, `limits.rs`, and `options.rs`.
   That is reasonable later, but the first runtime API is small enough for one
   limits module and fewer public file boundaries.

2. **Expose typed policy names and units, not free-form strings.**

   Use a `DecodeLimitName` enum for resource names and a `DecodeLimitUnit` enum
   for units. Names map to stable field names such as `max_input_bytes`,
   `max_obus`, `max_frames_to_decode`, `max_output_frames`,
   `max_frame_width`, `max_frame_height`, `max_luma_samples_per_frame`,
   `max_decoded_frame_bytes`, `max_reference_slots`,
   `max_reference_store_bytes`, `max_tile_count`,
   `max_tile_payload_bytes`, and `max_output_bytes`.

   `max_reference_slots` is deliberately named as slots rather than frames
   because AV2 reference accounting is slot/state based (§ 6.4.6, § 7.23).
   `max_reference_store_bytes` is separate from decoded-frame bytes because
   § 7.23 storage can include padded frame stores and reference metadata/state.
   `max_tile_payload_bytes` is per tile payload, not per tile group.

   Alternative considered: use the older prose names `max_reference_frames` and
   `max_tile_bytes`. They are less precise and risk future enforcement ambiguity.

3. **Provide finite defaults plus explicit zero and unlimited policies.**

   `DecodeLimits::DEFAULT` and `Default::default()` provide a finite CI-safe
   starting policy. The exact values are implementation constants and tests
   assert them so future changes are deliberate. `DecodeLimits::zero()` and
   `DecodeLimits::unlimited()` exist for tests and explicit caller policy, but
   they are not the default.

   Alternative considered: no default or unlimited default. That would avoid
   policy choices, but the mission requires safe defaults for fuzzing and CI and
   the API request asks for default behavior tests.

4. **Make checks inclusive and local.**

   `DecodeLimits::check(name, actual)` returns comparison metadata and succeeds
   when `actual <= limit`. `DecodeLimits::ensure(name, actual)` adapts failed
   checks into `DecodeLimitError::LimitExceeded`. `DecodeLimits::ensure_add`
   and `DecodeLimits::ensure_mul` report arithmetic overflow before any
   comparison. `DecodeLimits::ensure_allocation_len` checks the configured
   limit first, rejects values above the supported host allocation ceiling, then
   converts to `usize` for allocation handoff and reports a local platform-size
   error if the value cannot fit on the host.

   Alternative considered: return `Option`/`bool` only. Typed errors preserve
   the resource name, limit, actual value, operands, operation, and context for
   future diagnostic adaptation without emitting diagnostics now.

5. **Keep local helper errors distinct from decoder diagnostics.**

   Add `DecodeLimitError` and a crate-local `Result<T>` alias with manual
   `Display` and `std::error::Error` implementations. The error messages should
   describe local helper failures, such as a named limit being exceeded or an
   arithmetic overflow while deriving a value. They must not include the future
   diagnostic rule id or the full stable diagnostic field set.

   Alternative considered: make limit failures `DecodeDiagnostic` immediately.
   That would overclaim user-facing behavior before a byte-consuming planner
   knows spec sections, offsets, matrix rows, and remediation.

## Risks / Trade-offs

- **API naming lock-in** -> Use precise names now (`reference_slots`,
  `reference_store_bytes`, `tile_payload_bytes`) and keep the enums
  `#[non_exhaustive]` so future resource surfaces can be added.
- **Overclaiming decode behavior** -> Keep docs and matrix notes explicit that
  no input bytes are read, no allocations are performed, no CLI behavior
  changes, no hash/Y4M/reconstruction path exists, and no resource diagnostic is
  emitted.
- **Diagnostic registry drift** -> Keep the future rule id out of
  `crates/splot-decode/src` until source intentionally emits a diagnostic.
  Continue running `cargo xtask check-diagnostic-registry`.
- **Default thresholds may need tuning** -> Treat default values as finite
  repository policy, not AV2 normative limits. Tests pin them for deliberate
  review rather than claiming permanence.
- **Reference-store byte coverage is incomplete today** -> Include a dedicated
  `max_reference_store_bytes` field even though no reference store exists yet,
  so future § 7.23 storage charging has a named threshold.
