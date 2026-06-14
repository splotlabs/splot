## Context

`docs/LOCAL-REFERENCE-EVIDENCE.toml` and
`docs/DECODER-SUPPORT-MATRIX.toml` now both carry local reference evidence
metadata. The manifest checker verifies that each manifest entry's
`decoder_support_rows` names known matrix rows. The decoder support checker
renders matrix status and delegates to the manifest checker, but it currently
treats matrix `local_reference_evidence` strings as free-form text after local
path screening.

This leaves one drift path: a matrix row can cite
`docs/LOCAL-REFERENCE-EVIDENCE.toml::<id>` even if `<id>` is missing, or it can
cite an entry that does not list that same row. Manual review caught the first
real entries, but the gate should enforce this after review.

## Goals / Non-Goals

**Goals:**

- Parse checked manifest evidence IDs and row references through reusable xtask
  helpers.
- Make `cargo xtask check-decoder-support` reject stale
  `docs/LOCAL-REFERENCE-EVIDENCE.toml::<id>` pointers.
- Make the same gate reject reciprocal drift when a cited manifest entry does
  not include the citing row in `decoder_support_rows`.
- Add focused unit tests covering valid, missing-ID, and non-reciprocal cases.
- Keep all validation offline and metadata-only.

**Non-Goals:**

- No AVM/dav2d execution, source import, wrapper, script, build probe, CI job,
  dependency, or mandatory local setup.
- No runtime decoder, reconstruction, deterministic hash digest, or Y4M output
  behavior.
- No broad matrix schema redesign or new status values.
- No crate dependency graph change.

## Decisions

1. Keep cross-reference enforcement in `check-decoder-support`.

   Rationale: the decoder support matrix owns `local_reference_evidence`
   pointers. The manifest checker already proves manifest-to-row existence, and
   `check-decoder-support` already calls it, so this is the correct gate for the
   reverse edge.

2. Expose narrow xtask-internal manifest metadata helpers instead of reparsing
   TOML in `decoder_support.rs`.

   Rationale: this keeps manifest parsing rules single-sourced and avoids
   duplicating evidence ID/row extraction logic.

3. Only validate canonical manifest pointers of the form
   `docs/LOCAL-REFERENCE-EVIDENCE.toml::<evidence-id>`.

   Rationale: existing prose entries may remain useful while being screened for
   local paths. The stricter resolution rule applies to machine-addressable
   pointers.

## Risks / Trade-offs

- [Risk] Free-form prose evidence remains possible and cannot be resolved. ->
  Mitigation: only canonical `::id` pointers get strict resolution; prose
  remains path-screened and can be migrated later.
- [Risk] Future manifest path changes would require updating the pointer
  prefix. -> Mitigation: use the existing canonical path constant from the
  manifest checker.
- [Risk] The first implementation could overcouple rendering to validation. ->
  Mitigation: perform validation before rendering and keep rendered status text
  unchanged unless the matrix content changes.
