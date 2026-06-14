## ADDED Requirements

### Requirement: DecodeContext tile-payload handoff

The decoder support model SHALL route the crate-private tile-payload boundary
through `DecodeContext` before any runtime tile syntax traversal or
reconstruction work is added. This handoff is tracked by Feature ID
`DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` and SHALL use the context-owned
`splot_parallel::WorkerPool` to execute the existing `tile-payload-decode`
boundary. The handoff SHALL NOT expose public tile-payload API support, bypass
`DecodeContext`, add a second worker pool, use direct Rayon/crossbeam/global
pool/thread primitives, add a `splot-decode -> splot-recon` dependency, or claim
runtime `splot decode` success.

#### Scenario: Tile boundary runs through DecodeContext

- **WHEN** crate-internal decoder code asks `DecodeContext` to plan an already
  framed minimal tile-payload boundary
- **THEN** the context executes that boundary inside its single owned
  `splot_parallel::WorkerPool`
- **AND** it returns the same deterministic tile work-unit metadata and
  structured unsupported `decode_tile()` stop as the direct crate-private
  boundary

#### Scenario: Thread policy does not change tile boundary output

- **WHEN** the same tile-payload boundary input is planned through
  `DecodeContext` configured with `auto`, `1`, and a fixed positive worker count
- **THEN** the returned plan metadata is identical across those thread policies
- **AND** no global pool, nested pool, direct Rayon/crossbeam API, ad-hoc
  thread, or queue is used outside `splot_parallel`

#### Scenario: Runtime decode remains unsupported

- **WHEN** `splot decode` is run after this handoff exists
- **THEN** it still follows the existing plan-only unsupported behavior until a
  later OpenSpec change derives tile-payload inputs from parsed frame state and
  implements tile syntax/reconstruction/output
- **AND** repo code, tests, `xtask`, and CI do not locate or invoke AVM, dav2d,
  ffmpeg, or any external decoder
