# process delta: openspec-matrix-hygiene

Completes the `XTASK-CI-QUALITY-GATES` local/CI parity: OpenSpec validation
joins the local acceptance gate. No AV2 syntax change; the bitstream/validator
main-spec edits are non-normative reflows.

## ADDED Requirements

### Requirement: OpenSpec validation in the local gate

`cargo xtask ci` SHALL run `openspec validate --all --no-interactive` when the
`openspec` binary is available, under the same run-if-present policy as the
other external-tool checks (skip with an install hint when absent), so the
local gate and the CI workflow's conditional OpenSpec step enforce the same
validation.

#### Scenario: openspec installed

- **WHEN** `cargo xtask ci` runs on a machine with `openspec` on PATH and a
  spec or active change fails validation
- **THEN** the gate fails at the OpenSpec step

#### Scenario: openspec absent

- **WHEN** `cargo xtask ci` runs on a machine without `openspec`
- **THEN** the step is skipped with an install hint and the gate continues

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
