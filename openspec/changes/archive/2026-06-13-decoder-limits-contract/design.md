## Context

`splot decode` currently exits through the intentional
`decode/unsupported-feature` diagnostic and does not read input bytes or touch
the requested output. The decoder roadmap identifies explicit decode limits as
the next foundation item before any bitstream-derived pixel allocation.

This change is contract-only. It does not add `splot-decode`, `splot-recon`,
library APIs, CLI flags, new dependencies, or AVM/dav2d integration. It defines
the resource model that a future byte-consuming decode planner must implement
once the crate/dependency graph is approved.

Relevant AV2 anchors from the committed mirror:

- § 6.4.1 sequence header semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1`
- § 6.4.6 sequence inter config semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6`
- § 6.17.4.1 frame size semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1`
- § 6.17.4.3 frame size with refs semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-3`
- § 6.17.7.2 tile info semantics:
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2`
- § 5.19 tile group OBU syntax:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`
- § 5.20 and § 5.20.2.1 tile payload and decode tile syntax:
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20`
- § 7.1 general decoding process:
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-1`
- § 7.21 output processes:
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-21`
- § 7.23 reference frame update process:
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-23`

## Goals / Non-Goals

**Goals:**

- Define the conceptual `DecodeOptions { limits: DecodeLimits }` contract for
  future decoder planning.
- Require limits to be checked before input-driven allocation, tile traversal,
  reference-frame storage, output buffering, or frame hashing.
- Define the planned `decode/resource-limit` diagnostic fields and ownership
  while keeping it out of the emitted diagnostic registry until source emits it.
- Update decoder support and feature tracking docs so this contract is visible
  in the same generated status paths as other decoder foundation work.

**Non-Goals:**

- No new workspace crates, dependency edges, or public Rust API.
- No change to `splot decode`; it remains an unsupported entry point.
- No Y4M output, frame hash output, tile decode, reconstruction, or reference
  frame allocation.
- No AVM/dav2d source, binary, wrapper, script, `xtask` command, CI job, or
  required local setup.

## Decisions

1. **Use a docs Feature ID for this slice.**

   The Feature ID is `DOC-DECODE-LIMITS-CONTRACT` because this PR defines the
   contract only. Future implementation can add runtime Feature IDs once the
   decoder/reconstruction crate boundary is approved.

2. **Treat `DecodeLimits` as repository policy, not AV2 conformance.**

   AV2 defines syntax-derived values such as sequence maximum frame dimensions,
   per-frame dimensions, tile grids, output arrays, and reference frame update
   state. `splot` limits are caller/resource policy layered over those values.
   Diagnostics must cite the AV2 section that supplied the measured value, while
   the actual threshold comes from `DecodeLimits`.

3. **Define the budget categories before defining code.**

   The contract names these future limit fields:
   `max_input_bytes`, `max_obus`, `max_frames_to_decode`,
   `max_output_frames`, `max_frame_width`, `max_frame_height`,
   `max_luma_samples_per_frame`, `max_decoded_frame_bytes`,
   `max_reference_frames`, `max_tile_count`, `max_tile_bytes`, and
   `max_output_bytes`.

   This set covers sequence and frame dimensions (§ 6.4.1, § 6.17.4.1),
   reference-frame count (§ 6.4.6), inter-reference dimension constraints
   (§ 6.17.4.3), tile grid/count derivation (§ 6.17.7.2, § 5.19), tile payload
   traversal (§ 5.20), decoded output arrays (§ 7.21), and reference storage
   (§ 7.23).

4. **Reserve `decode/resource-limit` without emitting it.**

   The emitted decoder registry is exact: documenting an ID inside its enforced
   marker region before source emits it would make
   `cargo xtask check-diagnostic-registry` fail. This change documents
   `decode/resource-limit` as planned contract text only. When emitted later, it
   must use the stable fields already shared by decoder diagnostics plus
   `limit_name`, `limit`, `actual`, `unit`, `byte_offset`, and `bit_offset`.

5. **Preserve the current CLI boundary.**

   Future library decode planning should own resource-limit detection; the CLI
   should only translate flags into options and render the resulting diagnostic.
   This PR does not add those flags or reports. `splot decode --json` remains a
   single diagnostic object for the current unsupported entry point.

## Risks / Trade-offs

- **Contract overreach** -> Keep status `partial`, not `supported`, until
  runtime code enforces limits before allocation.
- **Diagnostic registry drift** -> Keep `decode/resource-limit` outside the
  enforced emitted-diagnostics table until source emits it.
- **Spec ambiguity around memory** -> Cite AV2 only for syntax-derived values
  and describe allocation thresholds as `splot` policy.
- **Dependency graph pressure** -> Do not add crates or public APIs in this
  change; the roadmap already records that crate scaffolding requires explicit
  maintainer approval.

## Migration Plan

1. Add the OpenSpec delta and docs describing the limit contract.
2. Mark `decode-limits-budget` partial in decoder support status with
   self-contained proof from OpenSpec and drift checks.
3. Add `DOC-DECODE-LIMITS-CONTRACT` to the implementation matrix and regenerate
   generated status docs.
4. Archive the OpenSpec change before PR, so `main` carries no completed
   unarchived decoder contract delta.

Rollback is a normal revert of documentation/matrix/OpenSpec files because no
runtime behavior or dependency graph changes are introduced.

## Open Questions

- Exact numeric defaults for future `DecodeLimits` remain open until the
  runtime API lands and can be tested with fuzz/fixture workloads.
- Whether `DecodeLimits` lives first in `splot-decode` or a temporary CLI-owned
  planning layer is blocked on explicit crate/dependency graph approval.
