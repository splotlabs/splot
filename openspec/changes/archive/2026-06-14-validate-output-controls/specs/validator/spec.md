# validator delta: validate-output-controls

Tracks `CLI-VALIDATE-OUTPUT-CONTROLS`. This adds presentation-only controls to the
`splot validate` CLI; it does not change which diagnostics are produced or the
conformance/exit-code decision.

## ADDED Requirements

### Requirement: validate diagnostic-count cap

`splot validate` SHALL accept `--max-diagnostics N` to show at most `N` diagnostics
in both text and JSON output, with the remainder summarized by a truncation notice
(text) and a `truncation` object (JSON) whose counts are computed from the full
report. The cap SHALL NOT change which diagnostics are computed, the summary
counts, or the exit code (which always derives from the full report via
`Validator::is_acceptable`). The truncation notice SHALL NOT be a `Diagnostic` and
SHALL carry no rule id.

#### Scenario: cap omits some diagnostics

- **WHEN** a report with more than `N` diagnostics is rendered with `--max-diagnostics N`
- **THEN** only `N` diagnostics are shown, an omitted-count notice / `truncation`
  object is emitted, the summary counts reflect the full report, and the exit code
  equals the uncapped run

#### Scenario: cap absent or non-truncating

- **WHEN** `--max-diagnostics` is not supplied (or `N` is at least the diagnostic count)
- **THEN** the output is byte-compatible with the previous full output (no notice,
  no `truncation` object)

### Requirement: validate summary-only output

`splot validate` SHALL accept `--summary-only` to print only the summary counts and
the conformance line (text) or a `summary` object with an empty `diagnostics` array
(JSON), suppressing the per-diagnostic lines. It SHALL take precedence over
`--max-diagnostics`, SHALL preserve the exit code, and SHALL be distinct from the
global `--quiet` flag (which controls logging only).

#### Scenario: summary-only suppresses per-diagnostic lines

- **WHEN** a report is rendered with `--summary-only`
- **THEN** no per-diagnostic line is shown, the summary counts and conformance
  result are still emitted, and the exit code is unchanged
