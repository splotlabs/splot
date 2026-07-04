# Decoder Architecture

This is the canonical map for where decoder work belongs in `splot-decode`.
Status and coverage details live in `DECODER-SUPPORT-MATRIX.toml` and generated
status docs.

## Module Map

```text
crates/splot-decode/src/
  bitstream/   byte stream planning, stream plans, tile-payload syntax/CDF boundaries
  pipeline/    decode orchestration, frame parsing gates, tile plan handoff, reconstruction bridge
  prediction/  intra/inter prediction selection and chroma prediction helpers
  residual/    residual planning, coefficient handoff, reconstruction ordering
  reference/   §7.23 reference slot metadata and decoded-frame stores
  filters/     deblock, CDEF, CCSO, Wiener-NS restoration ordering
  output/      decoded-frame hash, raw sample, and Y4M writers
  support/     capability messages and resource-limit helpers
  tile/        block context and tile-local plane/band state
  diagnostic/  structured decode diagnostics
```

## Pipeline

```text
input bytes
  -> Annex B / IVF / OBU planning
  -> sequence and frame-header parsing
  -> tile payload boundary and CDF setup
  -> tile/block symbol decode
  -> prediction selection
  -> residual reconstruction
  -> prediction plus residual writeback
  -> loop filters and restoration
  -> reference refresh and output scheduling
  -> hash/raw/Y4M output
```

`pipeline/mod.rs` is the frame-level orchestrator. Domain code belongs in the
domain module first; only cross-stage sequencing and shared diagnostic gates
belong in `pipeline`.

## Crate Boundaries

`splot-core` owns AV2 syntax, containers, headers, bit readers, and generated
tables exposed through syntax models.

`splot-recon` owns scheduler-free decoded-frame storage and reconstruction math.
It must not own decode runtime state, worker pools, tile scheduling, reference
refresh policy, or output publication ordering.

`splot-parallel` owns Rayon/crossbeam integration. Decode stages may use it only
through the approved local worker-pool policy.

`splot-decode` owns decode orchestration, capability gates, diagnostics,
reference state, filter ordering, and output routing.

## Output

`output/hash.rs`, `output/raw.rs`, and `output/y4m.rs` all consume decoded
pipeline frames. They share the same decode path, so output format selection
does not reparse or independently reconstruct the bitstream.

## Unsupported Features

Unsupported features are structured `DecodeError::UnsupportedFeature` values
with stable reason IDs, matrix rows, feature IDs, spec sections when relevant,
and byte offsets when available. Capability text is produced through
`support/capability.rs`; resource limits use `support/pipeline_limits.rs`.

## Adding Decoder Work

Add new code where the AV2 responsibility lives:

- New OBU/tile payload facts: `bitstream/`.
- New block or tile-local state: `tile/`.
- New intra/inter/chroma prediction selection: `prediction/`.
- New residual planning or transform handoff: `residual/`.
- New reference-slot behavior: `reference/`.
- New in-loop filter/restoration orchestration: `filters/`.
- New output representation: `output/`.
- Cross-stage frame sequencing only: `pipeline/`.
