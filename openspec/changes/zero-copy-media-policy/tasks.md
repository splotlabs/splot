# Tasks

## 1. Policy doc (`docs/ZERO_COPY.md`)

- [ ] Define borrow / mutable view / share / materialize / copy-ok marker.
- [ ] Document the default ownership model (borrowed bitstreams, owned
      reconstruction workspaces, borrowed plane/frame views, disjoint mutable
      regions for parallel work, move/share reference updates, one lookahead
      materialization point, output serialization as a copy boundary).
- [ ] Document banned patterns and the allowed-copy boundaries.
- [ ] Specify the `splot-copy-ok:` marker grammar and the vague-marker rejects.
- [ ] Document the gate scope (which crates/patterns are scanned) and the
      `zerocopy` allowed surface + IVF pattern.
- [ ] Cross-reference `docs/CONCURRENCY.md` and `docs/ARCHITECTURE.md`.

## 2. `splot-recon` view-first ownership

- [ ] Add `PlaneRef<'a, T>` / `PlaneMut<'a, T>` and `FrameRef<'a, T>` /
      `FrameMut<'a, T>` that borrow existing storage, validate geometry, and
      never allocate or copy on construction.
- [ ] Add `SharedFrame<T>` (Arc-backed, `.share()` only, no `Clone` derive, no
      mutable access, no `make_mut`).
- [ ] Remove `Clone` from `Plane`, `FramePlanes`, `DecodedFrame`,
      `ReferenceFrameStore`, `CurrentFrameWorkspace`, `CurrentFramePlane`; keep
      `Clone`/`Copy` on small metadata and borrow iterators.
- [ ] Add owned-type `.as_ref()` / `.as_mut()` view accessors that expose
      borrowed views without copying.
- [ ] Add specific `splot-copy-ok:` markers to every remaining intentional copy.
- [ ] Rewrite the one test that relied on `DecodedFrame: Clone`.

## 3. `xtask check-zero-copy-policy`

- [ ] Add `xtask/src/zero_copy.rs` with a pure `evaluate_*` core and an IO
      `check_*` wrapper (mirror `concurrency_policy.rs`).
- [ ] Implement dependency checks (`zerocopy` placement + banned alt-crates) and
      source checks (media-name `Clone`, suspicious `.clone()`, unmarked sample
      copies, `make_mut`, `unsafe`/`transmute`/`from_raw_parts`, unmarked
      `read_from_bytes`, `include!` bypass) with `splot-copy-ok:` marker support.
- [ ] Add the `CheckZeroCopyPolicy` task variant and wire it into `run_ci()`.
- [ ] Add accept/reject unit tests for every rule plus a real-repo-passes test.

## 4. (optional) `zerocopy` IVF wire

- [ ] If a clean use site exists: add the workspace `zerocopy` dep
      (`default-features = false, features = ["derive"]`), a private IVF wire
      struct in `splot-core`, and parse-by-borrow + validate-into-strong-type,
      preserving all current IVF error kinds/offsets/messages.
- [ ] Otherwise: document `zerocopy` as an approved future dependency with its
      allowed locations and do not add it.

## 5. Matrix and docs

- [ ] Add/update `docs/IMPLEMENTATION-MATRIX.toml` row
      `INFRA-ZERO-COPY-MEDIA-POLICY` with proof.
- [ ] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status
      --format markdown --output docs/FEATURE-STATUS.md`.
- [ ] Update `docs/ARCHITECTURE.md` (zero-copy ownership subsection +
      `zerocopy` dependency-direction) and `docs/CODE_REVIEW.md` (zero-copy
      checklist). Update `docs/references/THIRD-PARTY-NOTICES.md` if `zerocopy`
      is added or approved-future.

## 6. Tests and proof

- [ ] Add view construction-without-allocation tests.
- [ ] Add `*Mut` exclusivity + row-access tests.
- [ ] Add `FrameRef`/`FrameMut` plane-presence/geometry validation tests.
- [ ] Add owned-type borrowed-view-without-copy tests.
- [ ] Add `SharedFrame::share()` pointer-identity tests.
- [ ] Add reference-store move/share-without-`T: Clone` tests.
- [ ] Add gate accept/reject fixtures for every rule.
- [ ] Add IVF wire parsing + invalid magic/version/length/fourcc + preserved
      error tests (only if `zerocopy` is actually used).
- [ ] Add proof commands to the matrix row.

## 7. Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --workspace --all-targets --locked`
- [ ] `cargo xtask check-zero-copy-policy`
- [ ] `cargo xtask check-feature-status`
- [ ] `cargo xtask ci`
