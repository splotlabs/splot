## Context

The repository is validator-first and currently documents `splot decode` as a
stub. The parser/validator work is tracked by `docs/IMPLEMENTATION-MATRIX.toml`
and generated status docs, but decoder and reconstruction work has no equivalent
canonical support matrix. The mission requires decoder work only where it helps
future encoder roundtrips: frame/plane models, deterministic hashes,
reference-frame state, and eventually scalar reconstruction for a small tier.

Phase 1 audit found:

- `cargo xtask ci` is green on `44ce7bda`.
- `splot decode` exists only in `crates/splot-cli/src/commands/decode.rs`.
- No decoder or reconstruction crate exists yet.
- Local AVM and dav2d checkouts are available for evidence, but must remain
  outside repo code, CI, dependencies, scripts, and required tests.
- Adding `splot-recon` or `splot-decode` later is a dependency-graph change that
  needs explicit maintainer approval under `AGENTS.md`.

## Goals / Non-Goals

**Goals:**

- Establish `docs/DECODER-ROADMAP.md` as the plain-language decoder mission
  scope and non-goals.
- Establish `docs/DECODER-SUPPORT-MATRIX.toml` as the canonical
  decoder/reconstruction support status file.
- Generate a committed decoder support status document from that matrix.
- Make `cargo xtask ci` check decoder support status drift without AVM/dav2d.
- Record that local AVM/dav2d evidence may be cited in docs/manifests but is
  never executable repo plumbing.
- Add implementation-matrix rows for the docs/automation/CLI decode tracking
  introduced by this change.

**Non-Goals:**

- No pixel reconstruction, entropy decoding, tile payload symbol decoding,
  frame output, Y4M writing, or decoded-frame hash verification.
- No new workspace crate and no dependency graph change.
- No AVM/dav2d code, binary, wrapper, build probe, `xtask` command, script, CI
  job, or mandatory test.
- No change to current validator diagnostics or parser behavior.

## Decisions

1. **Use a separate decoder support matrix instead of overloading the feature
   matrix.**

   Rationale: `docs/IMPLEMENTATION-MATRIX.toml` tracks global Feature IDs and the
   parser/validator maturity stages. Decoder support needs tier, support status,
   parser source, reconstruction module, self-contained tests, local reference
   evidence, and unsupported-feature diagnostics. A separate TOML matrix keeps
   those decoder-specific fields explicit while still linking rows back to stable
   Feature IDs.

   Alternative considered: add many `DEC-*` rows to the implementation matrix
   immediately. Rejected for this change because the current matrix schema has no
   decoder category/kind yet, and broad decoder rows would imply more
   implementation commitment than this foundation PR can honestly prove.

2. **Generate a Markdown status document from TOML.**

   Rationale: the existing feature-status pattern works: TOML is canonical,
   Markdown is reviewable, and CI catches drift. `xtask` can implement the
   renderer using the existing `toml` dependency and standard library only.

   Alternative considered: hand-maintain `docs/DECODER-SUPPORT-STATUS.md`.
   Rejected because support status would drift as soon as the decoder backlog
   starts moving.

3. **Keep the first supported tier documented, not implemented.**

   Rationale: the first tier depends on later symbol decoding, frame/plane
   allocation limits, and reconstruction APIs. This change should set the bar:
   supported rows must have self-contained tests and proof before status becomes
   `supported`.

4. **Record local reference evidence as metadata only.**

   Rationale: AVM and dav2d are useful local oracles, but the repo must remain
   self-contained. The support matrix may record commit hashes and command
   summaries as text evidence; `xtask` must not invoke or locate those tools.

5. **Defer crate split approval.**

   Rationale: adding `splot-recon`/`splot-decode` is likely correct, but
   `AGENTS.md` requires asking before dependency graph changes. This change makes
   that future approval explicit instead of smuggling the graph change into a
   docs/status PR.

## Risks / Trade-offs

- Matrix schema too weak -> keep required fields small but checked by `xtask`:
  row id, name, feature id, spec sections, parser source, decode/recon module,
  tier, status, tests, diagnostics, and reference evidence.
- False sense of decoder progress -> roadmap and status docs must say no pixel
  decode exists yet, and no row may be `supported` without committed proof.
- Local reference leakage -> docs and matrix must use portable summaries only;
  no absolute paths or executable reference-tool assumptions.
- Extra automation surface -> keep the renderer narrow and covered by unit tests,
  and run it from `cargo xtask ci`.

## Migration Plan

1. Add OpenSpec delta specs and validate the change.
2. Add docs and decoder support matrix.
3. Add `xtask decoder-support --format markdown --output ...` and
   `xtask check-decoder-support`.
4. Wire `check-decoder-support` into `cargo xtask ci`.
5. Add implementation-matrix rows and regenerate generated docs.
6. Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, and
   `cargo xtask ci`.

## Open Questions

- Which crate split (`splot-recon` plus `splot-decode`, or only
  `splot-decode` initially) should be approved for item #2?
- Whether decoded-frame hash output should use MD5-compatible raw frame bytes,
  a repo-owned SHA-256 digest, or both. This change documents the question but
  does not settle decoder output hashing.
