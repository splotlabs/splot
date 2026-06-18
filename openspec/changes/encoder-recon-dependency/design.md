## Context

`encoder-program-contract` has landed and reserved the next exclusive encoder
flight for the reconstruction dependency decision. Today `splot-encode` depends
only on `splot-core` and `splot-parallel`; future encoder work must reuse
decoder-visible reconstruction math and frame/plane ownership from `splot-recon`
instead of duplicating it in the encoder crate.

The mission explicitly authorizes the single dependency-graph change
`splot-encode -> splot-recon`. Broader graph changes remain out of scope:
`splot-recon` still depends only on `splot-tables`, `splot-encode` still must not
depend on `splot-decode`, `splot-validate`, or `splot-cli`, and the CLI remains
thin.

## Goals / Non-Goals

**Goals:**

- Add `splot-recon` as a direct dependency of `splot-encode`.
- Update the repository dependency-direction policy, documentation, and tests so
  `cargo xtask check-dependency-direction` accepts exactly this new edge.
- Keep the dependency intentional by adding a private compile-time boundary marker
  in `splot-encode` that references `splot-recon` without exposing a public encoder
  API or changing behavior.
- Record a Feature ID and proof for the dependency-boundary change in the
  implementation matrix.
- Preserve zero-copy and concurrency policies.

**Non-Goals:**

- No encoder frame input model, `Frame` redesign, or public recon-backed API.
- No public encode success path; `send_frame`, `receive_packet`, and `flush` keep
  returning `Error::Unimplemented`.
- No reconstruction loop, prediction, transform, quantization, filtering, or
  reference-store integration in `splot-encode`.
- No new third-party dependency, no AV2 syntax/semantics change, and no copied
  external source/prose/tables/constants.
- No dependency from `splot-recon` back to `splot-core` or any decoder/validator
  crate.

## Decisions

1. **Add the direct crate edge now, behavior later.** This PR changes only the
   dependency boundary. Later PRs can design input views and closed-loop
   reconstruction against an already-approved crate graph.

2. **Use a private boundary marker instead of a public API.** A manifest-only
   dependency would be ambiguous and may be flagged as unused. A crate-private
   module or type alias in `splot-encode` will reference stable `splot-recon`
   public types at compile time, proving the dependency is deliberate without
   making any encoder API promise.

3. **Teach `xtask` the exact graph.** The dependency-direction allow-list and its
   tests should accept `splot-encode -> splot-recon` and continue rejecting
   unapproved edges, especially `splot-encode -> splot-decode`,
   `splot-encode -> splot-validate`, and any reverse edge into `splot-recon`.

4. **Keep zero-copy and concurrency unchanged.** `splot-recon` remains the owner
   of decoded/reconstructed media-buffer types, and `splot-parallel` remains the
   only crate allowed to depend on Rayon or `crossbeam-channel`.

## Risks / Trade-offs

- Dependency drift -> mitigate with `cargo xtask check-dependency-direction`,
  dependency-direction unit tests, and `cargo xtask ci`.
- Accidental public API promise -> keep the `splot-recon` reference private and
  document public integration as a future phase.
- Unused-dependency churn -> make the private boundary marker compile-time-visible
  so `cargo machete` remains green without adding runtime behavior.
- Same-file churn in policy docs/matrix -> audit open PRs before ready, keep the
  Flight Manifest current, and merge current `main` if another PR lands first.

## Flight Manifest

- Change ID: `encoder-recon-dependency`
- Feature IDs: new `ENC-RECON-DEPENDENCY`
- Base commit: `ed78cc66`
- Depends on merged changes: `encoder-program-contract` through PR #236
- Exact files/directories owned by this PR:
  - `AGENTS.md`
  - `Cargo.lock`
  - `crates/splot-encode/Cargo.toml`
  - `crates/splot-encode/src/**`
  - `docs/ARCHITECTURE.md`
  - `docs/DECODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `docs/ENCODER-GOAL.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/FEATURE-STATUS.md`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/SPEC-COVERAGE.md`
  - `openspec/changes/encoder-recon-dependency/**`
  - `openspec/specs/encoder-program/spec.md`
  - `openspec/specs/encoder-tools/spec.md`
  - `openspec/specs/process/spec.md`
  - `xtask/src/main.rs`
- Exact files/directories forbidden to this PR:
  - `crates/splot-core/**`
  - `crates/splot-recon/**`
  - `crates/splot-decode/**`
  - `crates/splot-validate/**`
  - `crates/splot-cli/**`
  - `docs/spec/av2/**`
  - `fuzz/**`
  - `tests/**`
- Public APIs/types owned: none
- Open sibling PRs audited: none open at proposal time
- Changed-file intersection with each sibling PR: none
- Semantic overlap with each sibling PR: none
- Can build/test/merge directly onto main without another open PR: yes, subject to
  final local gates and GitHub Claude/Codex acceptance on final HEAD

## Migration Plan

1. Add the dependency and private boundary marker.
2. Update policy/docs/matrix/OpenSpec.
3. Regenerate status outputs.
4. Run dependency, zero-copy, concurrency, OpenSpec, and full CI gates.
5. Archive the OpenSpec change before merge.

Rollback is removing the dependency edge and reverting the policy/docs/spec
updates. No runtime migration exists because no encoder behavior changes.

## Open Questions

- The public input/view API that will borrow or adapt `splot-recon` types belongs
  to `encoder-frame-input-views`.
- The closed-loop reconstruction algorithm and reference-store handoff belong to
  later encoder implementation changes.
