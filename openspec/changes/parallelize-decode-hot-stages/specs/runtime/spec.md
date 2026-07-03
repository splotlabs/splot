# runtime delta: parallelize-decode-hot-stages

Adds the parallel-stage constraints the decode runtime relies on after the
hot post-reconstruction filter stages move onto the context-owned worker
pool. Non-normative codec-runtime infrastructure: it adds no AV2 conformance
coverage and changes no decoded output. Tracked by
`INFRA-DECODE-PARALLEL-STAGES`.

## ADDED Requirements

### Requirement: parallel filter stages are deterministic and disjoint

A decode filter stage that runs on the context-owned worker pool SHALL
produce byte-identical output for every resolved thread count, and SHALL
write only disjoint plane regions or task-local buffers published in
bitstream order, so no two workers ever write the same sample. A stage that
cannot express its work as disjoint regions SHALL keep the serial path.
Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: thread sweep is byte-identical

- **WHEN** the same stream decodes with `--threads 1`, `2`, `4`, `8`, `10`,
  and `auto`
- **THEN** the raw output bytes are identical across all runs

### Requirement: banded publication fails loudly or falls back, never corrupts

Banded publication of filtered rectangles MUST fail loudly or fall back,
never corrupt: when a rectangle does not fit the disjoint plane row band that
owns its rows, the stage surfaces a typed error or returns control to a
serial in-order write that rewrites every rectangle. Because filter outputs
are disjoint, a partial parallel write before such a fallback leaves the same
final plane as a pure serial write. Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: mis-banded write never yields silent corruption

- **WHEN** a rectangle would fall outside the band that owns its rows
- **THEN** the stage errors or takes the serial fallback, and the decoded
  output stays byte-identical to the serial decode

### Requirement: parallel stages report scaling attribution

When `SPLOT_DECODE_TIMING` is set, a pool-parallel decode stage SHALL
report its work-unit count and the number of distinct workers that executed
stage work, and a stage that takes its serial fallback SHALL be
attributable from the report. Timing SHALL stay fully disabled otherwise.
Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: timing explains thread usage

- **WHEN** a stream decodes with `SPLOT_DECODE_TIMING=1 --threads 10`
- **THEN** stderr reports per-stage times and worker counts that show which
  stages used the pool and which stayed serial
