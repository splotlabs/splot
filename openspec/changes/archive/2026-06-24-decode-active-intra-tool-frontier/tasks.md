## 1. CDF and Mode Syntax

- [x] 1.1 Add MRL index and secondary-index CDF selectors, default rows, mutable selection, copy/average, and frame-end scaling tests.
- [x] 1.2 Extend general intra mode parsing to consume `mrl_index`/`mrl_sec_index` for directional luma modes and reject active nonzero MRL with a typed mode error.

## 2. Selectable Transform-Record Frontier

- [x] 2.1 Relax selectable pre-tile gates for enabled intra/transform tools and parsed CCSO filter state that are now handled by active-use or later filter/output checks.
- [x] 2.2 Add residual active-use gating so nonzero residuals under unsupported transform-type/CCTX/IST requirements stop before skipped syntax.
- [x] 2.3 Update focused selectable/MRL tests and the local decoder mission CLI probe expectation to the new frontier.

## 3. Tracking and Verification

- [x] 3.1 Add `DECODE-ACTIVE-INTRA-TOOL-FRONTIER` matrix/support rows and regenerate status docs.
- [x] 3.2 Run focused tests, `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask ci`.
