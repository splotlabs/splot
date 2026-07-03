# runtime delta: optimize-decode-serial-hot-paths

Adds the serial hot-path constraints the decode runtime relies on after the
per-call overhead removal. Non-normative codec-runtime infrastructure: it
adds no AV2 conformance coverage and changes no decoded output. Tracked by
`INFRA-DECODE-SERIAL-HOT-PATHS`.

## ADDED Requirements

### Requirement: diagnostic env gates are process-lifetime

A decode-path diagnostic gate backed by an environment variable SHALL be
read at most once per process and cached; hot paths SHALL NOT consult the
environment per block or per symbol. Tracked by
`INFRA-DECODE-SERIAL-HOT-PATHS`.

#### Scenario: traced run still emits

- **WHEN** a diagnostic variable such as `SPLOT_TRACE_LR_SYNTAX` is set at
  process launch
- **THEN** the gated diagnostics emit exactly as before the caching

### Requirement: hot-path restructurings stay bit-exact

A decode hot-path restructuring SHALL preserve byte-identical decoded
output, whether it reuses scratch, hoists validation, coalesces runs,
borrows views, or changes loop shape, and SHALL carry an equivalence test
against the prior computation shape on representative inputs whenever the
kernel's read set, accumulation order, or buffer lifetime changes. Tracked
by `INFRA-DECODE-SERIAL-HOT-PATHS`.

#### Scenario: motivating stream is unchanged

- **WHEN** the motivating 1080p 10-bit stream decodes with
  `--output-format raw --limit=1`
- **THEN** the raw output sha256 equals the pre-change value

#### Scenario: reused scratch leaks nothing

- **WHEN** consecutive blocks decode through a reused per-thread scratch
- **THEN** the results equal a fresh-scratch decode of the same blocks
