# runtime delta: optimize-decode-first-frame-latency

Adds first-frame latency constraints for measured decode-runtime hot-path work.
This is non-normative codec-runtime infrastructure: it adds no AV2 conformance
coverage and changes no decoded output. Tracked by
`INFRA-DECODE-FIRST-FRAME-LATENCY`.

## ADDED Requirements

### Requirement: first-frame latency optimizations are measured and bit-exact

Decode-runtime first-frame latency work SHALL start from `SPLOT_DECODE_TIMING`
attribution and a local profile or equivalent hotspot evidence. A hot-path
optimization SHALL preserve decoded raw bytes, hash output, structured
diagnostics, resource-limit behavior, deterministic output ordering, and
thread-policy-independent output. Tracked by
`INFRA-DECODE-FIRST-FRAME-LATENCY`.

#### Scenario: optimized first frame stays byte-identical

- **WHEN** the motivating IVF stream decodes with
  `--quiet --output-format raw --limit=1` before and after the optimization
- **THEN** the output sha256 MUST be identical

#### Scenario: measured attribution remains available

- **WHEN** `SPLOT_DECODE_TIMING=1 splot decode --quiet --output-format hash --limit=1` runs
- **THEN** stderr SHALL include compact `splot.decode_timing` phase lines
- **AND** normal stdout output SHALL remain the hash report

### Requirement: hot-path allocation reductions preserve fail-atomic output

Filter or reconstruction helpers MUST preserve caller-visible success output when reducing scratch allocation, collection, or copying.
Such changes MUST NOT commit partial caller-visible output on an error path that
was previously fail-atomic. Tracked by
`INFRA-DECODE-FIRST-FRAME-LATENCY`.

#### Scenario: successful filter output is unchanged

- **WHEN** a rewritten filter helper runs on representative valid luma/chroma
  blocks
- **THEN** its output MUST match the pre-change helper output bit-for-bit

#### Scenario: validation failures do not leak partial rows

- **WHEN** a rewritten filter helper encounters malformed source samples or an
  invalid subclass map
- **THEN** it MUST return the same typed error class as before
- **AND** it MUST NOT leave partially committed caller output where the prior
  helper was fail-atomic

### Requirement: deterministic owned-pool scheduling is preserved

First-frame decode optimization SHALL keep the existing local owned worker-pool
model. Parallel filter work SHALL keep deterministic serial publication order
and SHALL NOT use the Rayon global pool, nested pools, ad-hoc threads, or
scheduling-dependent reductions. Tracked by
`INFRA-DECODE-FIRST-FRAME-LATENCY`.

#### Scenario: thread policies produce identical output

- **WHEN** the same bitstream decodes with `--threads 1` and default thread mode
- **THEN** raw output and hash output SHALL be identical

#### Scenario: concurrency gate still passes

- **WHEN** `cargo xtask check-concurrency-policy` runs
- **THEN** it SHALL pass without new allowlist entries unless the entry
  documents an existing `WorkerPool::install` scope
