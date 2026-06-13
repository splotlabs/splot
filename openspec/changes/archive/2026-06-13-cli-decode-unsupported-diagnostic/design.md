## Context

`splot decode` currently accepts an input path and `-o/--output`, then prints a
generic "not yet implemented" message and exits `1`. The decoder roadmap now
requires unsupported decoder behavior to be explicit, structured, and tied to
the decoder support matrix. The dependency graph has not been approved for a
decoder or reconstruction crate, so this change must remain inside the existing
thin CLI surface.

## Goals / Non-Goals

**Goals:**

- Replace the generic decode stub with a stable unsupported-feature diagnostic.
- Make text output useful to humans and JSON output useful to tests/automation.
- Cite AV2 § 7.1, Feature ID `CLI-DECODE`, and support row
  `cli-decode-entrypoint`.
- Keep exit codes aligned with existing CLI policy: `1` for intentional
  unsupported behavior and `2` for operational errors.
- Update matrix/docs/OpenSpec evidence so the shipped state is honest.

**Non-Goals:**

- No `splot-decode` or `splot-recon` crate.
- No decoded frames, frame hashes, Y4M output, stream traversal, or input byte
  parsing.
- No dependency graph changes or new third-party dependencies.
- No AVM/dav2d/ffmpeg lookup, invocation, wrapper, build probe, test
  requirement, or CI integration.

## Decisions

1. Keep the unsupported diagnostic in `splot-cli` for this change.
   - Rationale: no decoder library crate has been approved yet, and the current
     behavior is CLI-only.
   - Alternative considered: create `splot-decode` with a stub API. Rejected
     because it changes the crate graph and requires maintainer approval.

2. Add `--json` to `splot decode` rather than changing global CLI behavior.
   - Rationale: `validate --json` already uses command-local JSON rendering, and
     this keeps behavior predictable.
   - Alternative considered: always emit JSON. Rejected because the default CLI
     should remain human-readable.

3. Do not read the input or create the output file before returning the
   unsupported diagnostic.
   - Rationale: the command cannot decode any supported stream yet, so touching
     user files only creates unnecessary I/O risk and weaker unsupported
     semantics.
   - Alternative considered: read input to prove the path exists. Rejected
     because nonexistent input would mask the decoder support state with an
     operational error, and later decoder work can add real input handling.

4. Use a small serializable diagnostic type local to the decode command.
   - Rationale: the required fields differ slightly from validator diagnostics
     and no shared decoder diagnostic crate exists yet.
   - Alternative considered: reuse `splot_validate::Diagnostic`. Rejected
     because decode diagnostics need `matrix_row`, `feature_id`, and
     `remediation`, and making validator diagnostics carry decoder fields would
     blur crate responsibilities.

## Risks / Trade-offs

- [Risk] The diagnostic type may move when a decoder crate is approved.
  → Mitigation: keep the JSON field names stable and test them at the CLI
  boundary.
- [Risk] Not reading input means `splot decode missing.ivf -o out.y4m` reports
  unsupported decode instead of missing input.
  → Mitigation: document this as intentional until decode support exists; real
  input I/O belongs to the future decode driver.
- [Risk] A text-only assertion would be too weak for automation.
  → Mitigation: add JSON tests that parse the full diagnostic object.
