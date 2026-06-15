# Tasks

## 1. Policy doc (`docs/ZERO_COPY.md`)

- [x] Define borrow / mutable view / share / materialize / copy-ok marker.
- [x] Document the default ownership model (borrowed bitstreams, owned
      reconstruction workspaces, borrowed plane/frame views, disjoint mutable
      regions for parallel work, move/share reference updates, one lookahead
      materialization point, output serialization as a copy boundary).
- [x] Document banned patterns and the allowed-copy boundaries.
- [x] Specify the `splot-copy-ok:` marker grammar and the vague-marker rejects.
- [x] Document the gate scope (which crates/patterns are scanned) and the
      `zerocopy` allowed surface + IVF pattern.
- [x] Cross-reference `docs/CONCURRENCY.md` and `docs/ARCHITECTURE.md`.

## 2. `splot-recon` view-first ownership

- [x] Add `PlaneRef<'a, T>` / `PlaneMut<'a, T>` and `FrameRef<'a, T>` /
      `FrameMut<'a, T>` that borrow existing storage, validate geometry, and
      never allocate or copy on construction.
- [x] Add `SharedFrame<T>` (Arc-backed, `.share()` only, no `Clone` derive, no
      mutable access, no `make_mut`).
- [x] Remove `Clone` from `Plane`, `FramePlanes`, `DecodedFrame`,
      `ReferenceFrameStore`, `CurrentFrameWorkspace`, `CurrentFramePlane`; keep
      `Clone`/`Copy` on small metadata and borrow iterators.
- [x] Add owned-type `.as_ref()` / `.as_mut()` view accessors that expose
      borrowed views without copying.
- [x] Add specific `splot-copy-ok:` markers to every remaining intentional copy.
- [x] Rewrite the one test that relied on `DecodedFrame: Clone`.

## 3. `xtask check-zero-copy-policy`

- [x] Add `xtask/src/zero_copy.rs` with a pure `evaluate_*` core and an IO
      `check_*` wrapper (mirror `concurrency_policy.rs`).
- [x] Implement dependency checks (`zerocopy` placement + banned alt-crates) and
      source checks (media-name `Clone`, suspicious `.clone()`, unmarked sample
      copies, `make_mut`, `unsafe`/`transmute`/`from_raw_parts`, unmarked
      `read_from_bytes`, `include!` bypass) with `splot-copy-ok:` marker support.
- [x] Add the `CheckZeroCopyPolicy` task variant and wire it into `run_ci()`.
- [x] Add accept/reject unit tests for every rule plus a real-repo-passes test.

## 4. (optional) `zerocopy` IVF wire

- [x] If a clean use site exists: add the workspace `zerocopy` dep
      (`default-features = false, features = ["derive"]`), a private IVF wire
      struct in `splot-core`, and parse-by-borrow + validate-into-strong-type,
      preserving all current IVF error kinds/offsets/messages.
- [x] Not taken (a clean IVF use site existed, so `zerocopy` was wired above).
      Its approved locations are also recorded in `THIRD-PARTY-NOTICES.md` §12.

## 5. Matrix and docs

- [x] Add/update `docs/IMPLEMENTATION-MATRIX.toml` row
      `INFRA-ZERO-COPY-MEDIA-POLICY` with proof.
- [x] Regenerate `docs/FEATURE-STATUS.md` with `cargo xtask feature-status
      --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Update `docs/ARCHITECTURE.md` (zero-copy ownership subsection +
      `zerocopy` dependency-direction) and `docs/CODE_REVIEW.md` (zero-copy
      checklist). Update `docs/references/THIRD-PARTY-NOTICES.md` if `zerocopy`
      is added or approved-future.

## 6. Tests and proof

- [x] Add view construction-without-allocation tests.
- [x] Add `*Mut` exclusivity + row-access tests.
- [x] Add `FrameRef`/`FrameMut` plane-presence/geometry validation tests.
- [x] Add owned-type borrowed-view-without-copy tests.
- [x] Add `SharedFrame::share()` pointer-identity tests.
- [x] Add reference-store move/share-without-`T: Clone` tests.
- [x] Add gate accept/reject fixtures for every rule.
- [x] Add IVF wire parsing + invalid magic/version/length/fourcc + preserved
      error tests (only if `zerocopy` is actually used).
- [x] Add proof commands to the matrix row.

## 7. Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-zero-copy-policy`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
