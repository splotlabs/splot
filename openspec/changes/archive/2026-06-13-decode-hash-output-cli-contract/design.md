## Context

`splot decode` is currently an intentional unsupported CLI entry point. It
parses `--json`, an input path, and required `-o/--output`, then emits the
stable `decode/unsupported-feature` diagnostic without reading input bytes or
touching the output path.

The decoder roadmap now defines deterministic `splot-dfh-sha256-v1` frame
hashes as the first success artifact and Y4M as later work. The existing CLI
shape still implies Y4M output for every valid `decode` invocation, which would
force a later supported hash path to carry an unnecessary Y4M path.

Crate and dependency-graph changes remain blocked on explicit maintainer
approval, so this change stays within `splot-cli`, docs, tests, and OpenSpec.

## Goals / Non-Goals

**Goals:**

- Add a future hash-output CLI contract without claiming runtime hash support.
- Preserve existing `splot decode <input> -o <output>` compatibility.
- Keep valid decode invocations unsupported until runtime decode exists.
- Keep `--json` scoped to diagnostic rendering while decode is unsupported.
- Record self-contained tests for the new parse surface and no-touch behavior.

**Non-Goals:**

- No `splot-recon` or `splot-decode` crate.
- No Cargo manifest, dependency graph, source dependency, `xtask`, script, CI,
  fixture, or diagnostic registry change.
- No input traversal, decode planning, reconstruction, hash computation, Y4M
  writing, file writing, stream validation, or layer selection.
- No AVM/dav2d/ffmpeg integration, local runner, wrapper, mandatory test, or
  committed local reference evidence.
- No final hash output file schema beyond selecting the future `hash` artifact
  kind; the existing deterministic hash contract remains authoritative.

## Decisions

1. Use `--output-format <y4m|hash>` instead of a separate `--hash-output` flag.

   This keeps one output selection concept for future success artifacts. The
   implicit compatibility form is still `-o/--output`, which resolves to Y4M.
   The explicit future hash form is `--output-format hash`, which can be valid
   without a Y4M path.

2. Do not give `--output-format` a clap default.

   `splot decode <input>` must stay a usage error instead of becoming a valid
   unsupported runtime diagnostic. The CLI resolves the implicit Y4M default only
   when `-o/--output` is present without an explicit format.

3. Allow `-o/--output` to mean "selected artifact path".

   With `--output-format y4m`, `-o` is the future Y4M path. With
   `--output-format hash`, `-o` is the future hash-report path. With
   `--output-format hash` and no `-o`, future hash output can use stdout. While
   decode remains unsupported, no valid parse writes to any path.

4. Keep the existing unsupported diagnostic unchanged.

   The current diagnostic registry owns `decode/unsupported-feature`,
   `CLI-DECODE`, and `cli-decode-entrypoint`. Adding output selection should not
   add fields or new rule IDs while no runtime decode path exists.

5. Frame hash output as repository-owned decoded output evidence.

   Future hash output refers to `splot-dfh-sha256-v1` over decoded AV2 output
   samples in repository-owned emission-index order. It is not AV2
   `METADATA_TYPE_DECODED_FRAME_HASH`, not AV2 `hash_type = 0` MD5, and not a
   digest of OBU bytes, metadata payloads, or parser facts.

## Risks / Trade-offs

- User-visible option semantics could be confused with current support ->
  document and test that valid hash-format invocations still emit
  `decode/unsupported-feature`.
- `--json` could be confused with future hash JSON output -> keep this change
  scoped to diagnostic JSON while unsupported and defer success report schema.
- `-o` becomes broader than Y4M -> update help text to say "selected output
  artifact", while preserving old Y4M behavior by default.
- A later runtime path could accidentally read input before rejecting an
  unsupported format -> keep current no-read/no-touch tests covering hash mode.
