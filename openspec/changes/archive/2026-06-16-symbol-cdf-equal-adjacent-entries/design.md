## Context

`validate_cdf` guards the generated-table indexing inside
`SymbolDecoder::read_symbol`. It currently enforces strictly increasing
cumulative entries (`value <= cdf[index - 1]` is an error). AV2 § 8.2.6 specifies
the symbol-read threshold loop and the CDF adaptation step but states no
strict-monotonicity precondition for `read_symbol`.

## Reachability evidence

The § 8.2.6 increment branch is `cdf[i] += ((1 << 15) - cdf[i]) >> rate`. For two
adjacent entries `a < b`, the gap shrinks by
`((1<<15)-a)>>rate - ((1<<15)-b)>>rate >= 0` each step and never overshoots
(the gap delta is bounded by the gap itself), so adjacent entries converge to
exactly equal. The same holds in the decrement branch near 0. A direct simulation
over real generated § 9.3 rows confirms it:

- `Default_Cctx_Type_Cdf` reaches `cdf[4] == cdf[5] == 32737` after ~140 symbol-0
  decodes.
- `Default_Amvd_Indices_Cdf[0]` and `Default_Cdef_Index_Minus1_With8_Cdf` reach
  equal adjacent entries similarly, for every symbol choice.

This is encoded as the `adaptation_can_equalize_adjacent_cumulative_entries`
test, which drives a real default row through the actual `update_cdf` until two
adjacent cumulative entries are equal.

## Decoding correctness with equal adjacent entries

In the threshold loop, two adjacent entries `cdf[s] == cdf[s + 1]` give the same
`f = (1 << 15) - cdf[symbol]`, but `pp` adds `Prob_Inc[N - 2][symbol]`, which is
strictly decreasing across `symbol` over the nonzero prefix. So `next_cur` still
strictly decreases and `new_range = prev - cur > 0` for every symbol bucket;
equal `cdf` entries do not collapse a bucket to zero width. For the 4-ary row
`[16384, 16384, 24576, 0, 0]`, symbol 1 owns the narrow-but-nonzero range
`[16448, 16480)` and decodes deterministically — covered by
`read_symbol_accepts_and_decodes_equal_adjacent_cumulative_entries`.

## Decision

Change the comparison to reject only a strict decrease:

```rust
if index > 0 && value < cdf[index - 1] { /* DecreasingCumulative */ }
```

Keep every other check (length, `[1, 32767]` probability range, adaptation-rate
index, capped use count) unchanged, and keep rejection-before-mutation behavior.

The error variant is renamed `NonIncreasingCumulative -> DecreasingCumulative`
so the typed error name stays honest: "non-increasing" literally means `<=`,
which is no longer the rejection condition. `read_symbol` arithmetic is not
touched.

## Non-goals

- No change to `read_symbol`/`read_bool`/`read_literal`/`exit_symbol` arithmetic.
- No § 8.3 CDF selection, Tile/Saved CDF banks, `decode_tile()`, reconstruction,
  or runtime decode behavior.
- No new validator diagnostic rule.
