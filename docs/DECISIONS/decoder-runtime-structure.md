# Decoder Runtime Structure

Date: 2026-07-04

Feature: `DECODE-RUNTIME-STRUCTURE`

## Decision

Production decode code is organized by decoder responsibility rather than by a
minimal-runtime bootstrap name. The active `splot-decode` layout is:

```text
bitstream/    byte/container planning and tile-payload syntax boundaries
pipeline/     frame-level orchestration, tile-plan handoff, reconstruction bridge
prediction/   intra, inter, edge, and chroma prediction selection/glue
residual/     decode-side residual planning and reconstruction orchestration
reference/    reference-frame slots and refresh metadata
filters/      deblock, CDEF, CCSO, and Wiener-NS loop-restoration ordering
output/       hash, raw, and Y4M serialization over decoded pipeline frames
support/      capability diagnostics and resource-limit helpers
tile/         block/tile-local context state
diagnostic/   structured decode diagnostic reporting
```

The old `runtime_minimal` and `runtime_minimal_recon` module names are retired
from production decode organization. “Minimal” remains valid only for named
support tiers and status rows such as `minimal-intra-8bit420-hash-v1`.

## Ownership Rules

`splot-decode` owns stream planning, frame orchestration, tile/symbol decode
handoffs, prediction scheduling, reference state, filter ordering, diagnostics,
and output routing. `splot-recon` remains scheduler-free reconstruction math and
decoded-frame storage primitives. `splot-parallel` remains the only concurrency
backend owner.

Hash, raw, and Y4M output modules consume decoded pipeline frames; they do not
own independent parsing or decode paths.

## Migration Policy

New decode modules must use AV2/decoder-domain names: `bitstream`, `tile`,
`prediction`, `residual`, `reference`, `filters`, `output`, `pipeline`,
`support`, or `diagnostic`. Do not introduce `runtime2`, `new_runtime`, `misc`,
or fixture-named runtime modules. Fixture names may appear only in isolated
tests, local evidence, or stable feature/status identifiers that still document
their historical frontier.

Temporary oversized-module allowances remain only for active frontier files and
must keep their source-line budget entries explicit.
