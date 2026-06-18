## Context

The `dc_sign` CDF bank (`TileDcSignCdf[ptype][isHidden][ctx]`) is wired but its
§8.3.2 `ctx` (08-parsing-process.md lines 1448-1487) was deferred. `dc_sign_ctx`
derives it, completing the selection. It is the simpler of the two sign contexts
(the other, `idtx_sign`, needs the `QuantSign[]` buffer).

## Decisions

- **Caller-provided DC-context slices.** The spec reads `AboveDcContext[plane]` /
  `LeftDcContext[plane]`, whose lengths are `MiCols` / `MiRows`. The function takes
  those plane slices directly, so the spec `x4 + k < MiCols` / `y4 + k < MiRows`
  bounds become the slice-length bounds — no separate `max` parameters, consistent
  with the caller-resolves convention. DC-sign values are `0` / `1` / `2`, hence
  `&[u8]`.

- **`break`, not skip-and-continue.** The spec loops `k` in `0..w4` and skips when
  `x4 + k >= MiCols`. Since `idx = x4 + k` is monotonic, once it leaves the slice
  every later `k` is also out of range, so `break` is equivalent to skipping the
  remainder and additionally bounds the loop to the slice length — a pathological
  `w4` / `h4` cannot spin (the bug a `usize::MAX` totality test caught).

- **Total / panic-free `const fn`.** `idx` uses `saturating_add`; the slice bound
  guards the read; the loop is bounded. `dcSign` is `isize` (it can go negative);
  the returned `ctx` is `usize` (`0` / `1` / `2`). A module-level `const`
  spec-contract check is the non-test consumer (so no `#[allow(dead_code)]`).

- **Completes the `dc_sign` selection.** Together with the `dc_sign` CDF bank, the
  §8.3.2 `dc_sign` element is fully selectable; it remains loaded-but-unread until
  the coeffs() loop supplies the DC-context buffers.

## Risks / Trade-offs

- **Sign-vote / bound fidelity** is the main risk (sign 1 vs 2 direction, the
  per-axis bound, the `<0` / `>0` / `else` mapping). Mitigated by tests pinning the
  netting (above vs left, positive / negative / zero), the position offset, the
  out-of-slice (max-bound) skip, and pathological-geometry totality (which also
  guards against the loop-spin regression).
