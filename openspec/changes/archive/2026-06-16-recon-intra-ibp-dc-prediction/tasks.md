## 1. Spec And Planning

- [x] 1.1 Validate the OpenSpec change with strict mode before implementation.
- [x] 1.2 Record subagent planning decisions for spec mapping, architecture, and safety scope.

## 2. Core Implementation

- [x] 2.1 Add the `splot-recon` IBP DC primitive and public exports.
- [x] 2.2 Add current-frame workspace IBP DC prediction handoff from in-storage edges.
- [x] 2.3 Extend the existing recon intra fuzz target to exercise direct and workspace IBP DC paths.

## 3. Tests

- [x] 3.1 Add focused direct primitive tests for above-only, left-only, both-edge square/wide/tall, no-edge no-op, and typed invalid inputs.
- [x] 3.2 Add workspace tests for interior IBP DC, top-left no-edge DC behavior, missing plane, and out-of-bounds geometry.
- [x] 3.3 Run targeted recon/fuzz verification commands.

## 4. Documentation And Status

- [x] 4.1 Add implementation and decoder-support matrix rows for `RECON-INTRA-IBP-DC-PREDICTION`.
- [x] 4.2 Update generated decoder support/conformance docs and roadmap/status notes while keeping broad rows partial.
- [x] 4.3 Run feature-status, decoder-support, conformance-coverage, and OpenSpec validation gates.

## 5. Review, Gate, And PR

- [x] 5.1 Run independent correctness, security, performance, documentation, and testing reviews and resolve findings.
- [x] 5.2 Run `cargo xtask ci`, archive the OpenSpec change, and rerun required gates.
- [ ] 5.3 Commit, push, open a ready PR, wait for final-head green CI plus final-head approval/thumbs-up and zero unresolved threads, then squash merge.
