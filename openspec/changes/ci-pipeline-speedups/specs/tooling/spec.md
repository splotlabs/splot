# tooling delta: ci-pipeline-speedups

Adds CI-speed constraints to the `tooling` capability. Non-normative repository
tooling: it adds no AV2 conformance coverage. Tracked by
`INFRA-CI-PIPELINE-SPEEDUPS`.

## ADDED Requirements

### Requirement: local CI avoids redundant all-target builds

`cargo xtask ci` SHALL NOT run a separate
`cargo build --workspace --all-targets --locked` pass after the all-target clippy
gate. It SHALL rely on `cargo clippy --workspace --all-targets --all-features
--locked -- -D warnings` and `cargo test --workspace --all-targets --locked` for
workspace compilation coverage, then continue with doctests, rustdoc, optional
external tools, and repository gates. Tracked by `INFRA-CI-PIPELINE-SPEEDUPS`.

#### Scenario: local CI moves from clippy to tests

- **WHEN** `cargo xtask ci` runs
- **THEN** the command sequence runs all-target clippy and then all-target tests
  without an intervening `cargo build --workspace --all-targets --locked`

### Requirement: GitHub CI avoids redundant all-target builds

The GitHub `ci` job SHALL mirror the local gate by omitting a standalone
`cargo build --workspace --all-targets --locked` step. The job SHALL still run
formatting, all-target clippy, all-target tests, doctests, strict rustdoc, and
the existing repository gates. Tracked by `INFRA-CI-PIPELINE-SPEEDUPS`.

#### Scenario: GitHub CI command list has no duplicate build step

- **WHEN** the GitHub `ci` job runs on a pull request or push
- **THEN** it runs all-target clippy and all-target tests without a standalone
  `cargo build --workspace --all-targets --locked` step

### Requirement: GitHub CI caches pinned dupehound installs

The GitHub `ci` job SHALL cache the pinned `dupehound` binary and SHALL accept a
cached binary only when `dupehound --version` reports `dupehound 0.1.2`. When the
binary is missing or has a different version, the job SHALL install
`dupehound@0.1.2` with `--locked --force`. Tracked by
`INFRA-CI-PIPELINE-SPEEDUPS`.

#### Scenario: dupehound cache hit

- **WHEN** the GitHub `ci` job restores a `dupehound` binary reporting version
  `dupehound 0.1.2`
- **THEN** the install step prints the version and skips `cargo install`

#### Scenario: dupehound cache miss or wrong version

- **WHEN** the GitHub `ci` job does not find `dupehound 0.1.2`
- **THEN** it installs `dupehound@0.1.2` with `cargo install --locked --force`

### Requirement: GitHub CI uses non-incremental cargo artifacts

The GitHub Actions workflow SHALL set `CARGO_INCREMENTAL=0` for CI jobs and use a
fresh cargo cache namespace for target artifacts built under that policy. Tracked
by `INFRA-CI-PIPELINE-SPEEDUPS`.

#### Scenario: future target cache is non-incremental

- **WHEN** a GitHub Actions cargo step builds the workspace
- **THEN** cargo sees `CARGO_INCREMENTAL=0`, and target artifacts are restored
  from or saved to the non-incremental cache namespace

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
