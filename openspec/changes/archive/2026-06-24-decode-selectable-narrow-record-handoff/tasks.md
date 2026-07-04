## 1. Runtime Handoff

- [x] 1.1 Add a narrow-luma selectable-record path that consumes partition
  syntax, records the actual leaf extent when consumed partition geometry is
  empty, and admits luma-only chroma-offset leaves.
- [x] 1.2 Preserve zero-width/zero-height transform-record rejection outside the
  supported luma-only narrow fallback.
- [x] 1.3 Retain skipped luma residuals with `skip_flag = true` and `eob = 0`
  when `all_zero` is decoded.

## 2. Tests And Evidence

- [x] 2.1 Add focused `splot-decode` tests for narrow-luma actual extents,
  luma-only chroma-offset admission, general zero-geometry rejection, and
  skipped-record retention.
- [x] 2.2 Update the local decoder mission CLI ignored test to expect the active-MRL
  frontier instead of the empty-transform frontier.
- [x] 2.3 Update implementation/support matrices and regenerate decoder support
  status.

## 3. Validation

- [x] 3.1 Run focused `splot-decode` and `splot-cli` tests, including the local
  local decoder mission probe.
- [x] 3.2 Run `openspec validate --all --no-interactive`.
- [x] 3.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`,
  and `cargo xtask ci`.
