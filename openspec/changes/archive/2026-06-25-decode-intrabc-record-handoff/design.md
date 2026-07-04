## Context

PR #487 moves the local decoder mission probe past the active FSC residual handoff and
the observed empty-transform fallback. The next live stop is
`unsupported_wienerns_lr_selectable_transform_records_intrabc` at byte offset
110 while parsing the key-frame Wiener NS LR selectable transform-record path.

The stop happens in the AV2 §5.20.5.3 intra frame mode-info prelude: for a
small luma/shared block, `allow_intrabc` permits `use_intrabc S()`, and the
local stream signals `use_intrabc = 1`. The current handoff only admits zero.
When `use_intrabc` is active, §5.20.5.3 sets the block to an inter-like IntrABC
mode, assigns luma/chroma defaults, calls §5.20.5.4 `read_intrabc_info()`, and
then later transform-size and residual syntax derive `is_inter = 1` even though
this is still inside an intra frame.

## Goals / Non-Goals

**Goals:**

- Consume the observed `use_intrabc = 1` mode-info path in spec order for the
  local decoder mission selectable-transform record handoff.
- Retain narrow IntrABC metadata needed by transform-size and residual handoff:
  `is_inter`, default luma/chroma prediction modes, `fsc_mode = 0`,
  `CwpIdx = CWP_EQUAL`, selected `RefMvIdx`, MV precision, and the block MV facts
  that can be parsed without producing samples.
- Advance the local probe to the next structured unsupported-feature diagnostic
  after IntrABC mode-info syntax is consumed.
- Preserve fail-closed behavior before decoded samples, IntrABC current-frame
  prediction, reconstruction, loop-restoration filtering/output, reference
  refresh, and byte-equality claims.

**Non-Goals:**

- Broad IntrABC support outside the local decoder mission path.
- Current-frame block-copy prediction or decoded `CurrFrame` population.
- Full §7.12.2 block-vector candidate modeling if the observed path can be
  retained with a narrower verified subset.
- Any encoder, public API, CLI option, dependency graph, or oracle-invocation
  change.

## Decisions

1. Keep IntrABC parsing local to the selectable-transform handoff first.

   The failing runtime path is a syntax/metadata handoff used to derive live LR
   tx-skip records. Wiring IntrABC into the broader general-intra mode API would
   imply reconstruction semantics the runtime cannot yet satisfy. A local helper
   can parse the observed §5.20.5.4 symbols, return typed facts for the handoff,
   and still reject before output.

2. Treat active IntrABC as `is_inter = 1` only for downstream syntax context.

   AV2 §5.20.5.3 sets `is_inter = 1` on the `use_intrabc` branch. The selectable
   transform reader and coefficient handoff must use that fact for §5.20.6 and
   §5.20.7.27 context selection, but this does not mean the runtime may produce
   inter or IntrABC predicted samples.

3. Consume only observed, spec-grounded IntrABC sub-branches.

   The first implementation should parse the `intrabc_mode`, DRL loop,
   optional `intrabc_precision`, and any observed gated flags in §5.20.5.4. It
   should reject unobserved active BAWP/morph-pred, unsupported block-vector
   derivations, or any branch whose context cannot be proven from current state.

4. Keep the old zero-IntrABC behavior as a regression boundary.

   Existing selectable-transform tests and the previous local decoder mission frontier prove
   non-IntrABC prelude ordering, CDEF, delta-Q, FSC, transform partitioning, and
   residual handoff. This change must not alter the syntax order for
   `use_intrabc = 0`.

## Risks / Trade-offs

- Wrong §5.20.5.4 read order could satisfy bit-count checks while corrupting
  decoded metadata. Mitigation: add symbol-sequence unit tests for the active
  IntrABC helper and verify with the local ignored local decoder mission probe.
- Reusing ordinary intra luma/chroma mode decoders on the IntrABC branch would
  read syntax the spec does not read. Mitigation: return IntrABC defaults rather
  than calling those mode decoders when `use_intrabc = 1`.
- Treating IntrABC as fully decoded after syntax consumption would risk
  confident-wrong output. Mitigation: keep the runtime on
  `decode/unsupported-feature` after metadata retention and update matrices to
  state that samples/output remain unclaimed.
- A stacked branch depends on #487. Mitigation: keep the new work isolated on a
  follow-on branch and sync/rebase after #487 is reviewed and merged.
