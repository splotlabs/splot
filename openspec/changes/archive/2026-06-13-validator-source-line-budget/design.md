# Design: validator-source-line-budget

## Context

The public validator entry point is small, but `crates/splot-validate/src/validator.rs`
also contains more than twenty thousand lines of tests and fixtures. That makes
behavior-preserving changes difficult to review and increases the risk that
future validator work piles onto the same file.

The repository already has deterministic `xtask` checks for license headers,
dependency direction, feature tracking, generated tables, and diagnostic registry
drift. A source-line budget belongs in that same offline automation layer.

## Goals / Non-Goals

**Goals:**

- Preserve `splot_validate::Validator` and the current validator behavior.
- Split validator production code by responsibility: public facade, runner flow,
  and parse/IVF diagnostic conversion.
- Split validator tests by validation domain while keeping shared builders in a
  fixture module.
- Add a soft 1000-line Rust source-file budget and an `xtask` report/check that
  fails only for hard-cap violations.

**Non-Goals:**

- Do not add, remove, rename, or reorder validator diagnostics.
- Do not change AV2 syntax, parsing, context state, or conformance semantics.
- Do not refactor unrelated large files unless the hard cap requires a narrow
  exemption.
- Do not add third-party dependencies.

## Decisions

- Replace `src/validator.rs` with a directory module. `validator/mod.rs` keeps
  the public `Validator` type and methods; `validator/runner.rs` owns stream
  orchestration and check execution; `validator/diagnostics.rs` owns
  `parse_error_diagnostic`, `ivf_error_diagnostic`, and the IVF rule-id list.
  This preserves `pub mod validator;` in `lib.rs` and keeps internals private to
  the validator module where possible.
- Keep tests under `validator/tests/` as children of `validator`. This lets tests
  continue to access private validator helpers without making implementation
  details public.
- Use one shared `validator/tests/fixtures.rs` for common bit writers, Annex B/IVF
  builders, sequence-header builders, and assertion helpers. Domain-specific
  builders stay in the domain test files that use them so the fixture file does
  not become a replacement monster.
- Implement `cargo xtask check-source-lines` using only `std` and existing
  `anyhow` support. The check uses git's index plus non-ignored new files so it
  inspects Rust files deterministically before staging while ignoring `.git`,
  `target`, and generated build output by construction.
- Set the soft limit to 1000 physical lines and the hard cap to 2500 physical
  lines. Files over the soft limit print advisory warnings; files over the hard
  cap fail unless listed in a small hard-cap exception table with a reason.

## Risks / Trade-offs

- Risk: Moving tests into many modules can accidentally hide or duplicate helper
  dependencies. Mitigation: keep modules as children of `validator`, run focused
  `splot-validate` tests, and verify no test deletions.
- Risk: A strict line check could block existing cohesive large files. Mitigation:
  warn at the soft limit and fail only above the hard cap, with explicit
  exceptions only for pre-existing files that are out of scope for this change.
- Risk: Splitting by arbitrary line ranges would not improve maintainability.
  Mitigation: group tests by validation domain and keep production modules aligned
  with responsibility boundaries.

## Migration Plan

1. Add the OpenSpec change and `XTASK-VALIDATOR-MODULE-SPLIT` /
   `XTASK-SOURCE-LINES` matrix rows.
2. Move validator production code into the new module tree.
3. Move tests into domain modules and shared fixtures without changing test bodies
   beyond import paths and visibility.
4. Add the source-line budget documentation and xtask command, then wire it into
   `cargo xtask ci`.
5. Run formatting, focused validator tests, targeted xtask checks, and the full CI
   gate.
