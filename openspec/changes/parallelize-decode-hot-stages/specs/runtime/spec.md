# runtime delta: parallelize-decode-hot-stages

Adds the parallel-stage constraints the decode runtime relies on after the
hot decode/filter stages move onto the context-owned worker pool.
Non-normative codec-runtime infrastructure: it adds no AV2 conformance
coverage and changes no decoded output. Tracked by
`INFRA-DECODE-PARALLEL-STAGES`.

## ADDED Requirements

### Requirement: parallel decode stages are deterministic and disjoint

A decode or filter stage that runs on the context-owned worker pool SHALL
produce byte-identical output for every resolved thread count, SHALL write
only disjoint plane regions or task-local buffers merged in bitstream
order, and SHALL keep every entropy-decode symbol read on the serial parse
path. Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: thread sweep is byte-identical

- **WHEN** the same stream decodes with `--threads 1`, `2`, `4`, `8`, `10`,
  and `auto`
- **THEN** the raw output bytes are identical across all runs

### Requirement: deferred reconstruction replays in parse order

A reconstruction stage that defers work captured during the serial entropy
parse SHALL replay ordered commits in exactly the original parse order, so
neighbor-pixel dependencies (intra prediction edges, CfL luma reads,
IntraBC source coverage) observe the same state as the interleaved decode.
Deferred work computed on the pool SHALL depend only on captured descriptor
data and immutable inputs. Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: staged intra equals interleaved intra

- **WHEN** a key-frame tile decodes through descriptor capture, parallel
  residual reconstruction, and ordered replay
- **THEN** the reconstructed frame equals the interleaved single-pass
  decode bit-for-bit

### Requirement: parallel stages report scaling attribution

When `SPLOT_DECODE_TIMING` is set, a pool-parallel decode stage SHALL
report its work-unit count and the number of distinct workers that executed
stage work, and a stage that chooses its serial fallback SHALL be
attributable from the report. Tracked by `INFRA-DECODE-PARALLEL-STAGES`.

#### Scenario: timing explains thread usage

- **WHEN** a stream decodes with `SPLOT_DECODE_TIMING=1 --threads 10`
- **THEN** stderr reports per-stage times and worker counts that show which
  stages used the pool
