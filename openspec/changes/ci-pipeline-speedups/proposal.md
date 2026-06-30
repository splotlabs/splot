# Change: ci-pipeline-speedups

## Feature IDs

- `INFRA-CI-PIPELINE-SPEEDUPS`

## Why

The local `cargo xtask ci` gate and the GitHub `ci` job both run an explicit
`cargo build --workspace --all-targets --locked` after
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
and before `cargo test --workspace --all-targets --locked`. On a cold local run,
that build pass took 20.60 seconds while adding no independent behavioral gate:
all-target clippy type-checks the targets with warnings denied, and all-target
tests compile and execute the test surface.

Recent GitHub Actions timing showed the same duplicated build pass taking 39
seconds, while the pinned `dupehound` install took 58 seconds and the restored
target cache took more than two minutes. The workflow can avoid the redundant
build, keep future target caches smaller by disabling incremental compilation in
CI, and skip rebuilding `dupehound` on cache-warm runs.

## Scope

- Spec sections: none (infrastructure only).
- Crates/modules: `xtask/src/main.rs` (`run_ci` command list).
- CI/docs/tests: `.github/workflows/ci.yml`, `README.md`,
  `docs/agents/commands.md`, `docs/IMPLEMENTATION-MATRIX.toml`, and generated
  `docs/FEATURE-STATUS.md`.

## Non-goals

- No decoder, validator, reconstruction, or encoder behavior changes.
- No change to AV2 conformance status, diagnostics, dependency direction, or
  test selection.
- No new Rust dependency or GitHub Action.
- No same-run build-artifact sharing between GitHub jobs; this keeps the workflow
  simple and avoids large artifact upload/download churn.

## Acceptance criteria

- [ ] `cargo xtask ci` no longer runs the redundant explicit `cargo build`
      pass.
- [ ] `.github/workflows/ci.yml` no longer runs that explicit build pass.
- [ ] GitHub CI disables incremental compilation and uses a fresh cargo cache
      namespace for smaller future target caches.
- [ ] GitHub CI caches the pinned `dupehound` binary and skips `cargo install`
      when it is present.
- [ ] Documentation reflects the local gate contents.
- [ ] `cargo xtask check-feature-status` passes.
- [ ] `cargo xtask ci` passes.
