# Design notes

## Ground truth (AVM)

`av2/decoder/decodetxb.c` reads `all_zero = read_symbol(txb_skip_cdf[...])` and
takes the no-coefficient branch (`if (all_zero) return 0;`) — the decoded symbol
*is* `all_zero`, and `all_zero == 1` is the skip. The frozen trace asserted `0`.

## Why the prescribed in-place "regenerate the fixture for the frozen trace" is
## infeasible, and the chosen shape

An empirical sweep with `avmenc` (revision `457cd5868`) showed that a real
conformant 64x64 intra frame of a flat input never produces an all-*planes* skip:
the luma block quantizes to an all-zero (skipped) transform at any sufficiently
high QP, but the chroma planes always carry a real coded residual. So no
AVM-producible bitstream matches the frozen tier's hand-traced "all planes
all-zero" symbol sequence, and the frozen partition frontier mis-parses a real
conformant stream (it computes a bogus oversized MI region). Correcting the frozen
`txb_skip` assertion to `all_zero == 1` also makes the frozen happy path
*untestable* without re-fabricating a circular payload (the retired payload decoded
the symbol to `0`, and no conformant all-planes-skip stream exists).

Therefore the conformant fixture is decoded by the **general intra path**, not the
frozen tier:

- The fixture is `avmenc --qp 210` with broad tools, intra DIP, and tx-partition
  disabled, so it satisfies the general path's admission gate (`base_q_idx != 255`,
  no DIP/IBP/MRLS/edge-filter, `tx_mode == Largest`, etc.) and its luma block is a
  clean `all_zero == 1` skip. splot decodes it byte-for-byte identically to
  `avmdec` and `dav2d` (raw md5 `f618317b…`, sha256 `92c4477c…`).
- The frozen minimal-tier code stays (per the standing decision) with its
  assertion corrected for honesty, but it is no longer reached by any committed
  conformant fixture. Its corrected polarity is proven by a legacy-rejection test
  that feeds the retired payload through the frozen trace and asserts the typed
  `UnexpectedSymbol { expected: 1, actual: 0 }` mismatch on the luma `txb_skip`.

## Test reshaping

- The frozen "accepts the minimal trace" / "rejects exit_symbol padding" frontier
  tests are replaced by one `block_symbol_trace_rejects_legacy_inverted_skip` test
  (the frozen trace now correctly rejects the inverted-polarity payload) plus the
  retained synthetic block-symbol unit tests. The two MI-state limit tests keep
  their original assertions by running against the embedded retired payload, whose
  partition geometry is unchanged by the symbol-value correction.
- `general_intra_tests::luma_skip_fixture_decodes_skip_branch_through_general_path`
  is the first conformant, oracle-anchored exercise of the `all_zero == 1` luma
  skip branch in the real decoder.
