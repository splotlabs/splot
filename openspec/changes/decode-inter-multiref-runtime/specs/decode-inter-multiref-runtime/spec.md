## ADDED Requirements

### Requirement: Decode a frame that selects among two valid decoded references
The decoder SHALL decode a three-frame stream (a key frame followed by two
single-reference inter frames) where the third frame selects among TWO valid
decoded references retained by the AV2 § 7.20 / § 7.23 reference frame update,
producing output byte-identical to avmdec and dav2d for the committed
`syn-3frame-multiref-64x64.ivf` fixture.

After decoding each frame, the decoder SHALL apply the § 7.23 reference frame
update for the slots named by `refresh_frame_flags`: a KEY or SWITCH frame SHALL
set `RefValid[i] = first` (only the first refreshed slot becomes valid), and an
inter frame SHALL set every refreshed slot `RefValid[i] = 1`; each refreshed slot
SHALL store `RefOrderHint`, `RefFrameWidth`/`RefFrameHeight`, `RefBaseQIdx`, and
the decoded frame. The third frame SHALL therefore see two valid reference slots
(the key in slot 0 and the retained inter frame in slot 1).

When the § 7.7 implicit reference-map derivation finds two valid slots, the
decoder SHALL run the real § 7.7 `get_ref_frames()` ranking — scoring each slot's
`RefBaseQIdx` — and derive `NumTotalRefs == 2` with `ref_frame_idx == [0, 1]`,
rather than stopping with `UnmodeledDerivation`. The `RefBaseQIdx` SHALL be
supplied through the reference-state view; a view that does NOT model `RefBaseQIdx`
SHALL still stop with `UnmodeledDerivation` for two valid slots (the historical
single-valid-slot behavior is unchanged).

For a `NumTotalRefs == 2` single-reference (non-compound) inter block, the decoder
SHALL read the § 5.20.7.12 `single_ref` symbol over `TileSingleRefCdf[ctx][0]`,
where `ctx` is the § 8.3.2 `single_ref` context derived from the neighbour
`count_refs` (the same derivation as `comp_ref`), and SHALL resolve the block's
reference as `ref_frame_idx[RefFrame[0]]`. The read SHALL be bit-exact: a wrong
`single_ref` value or context desynchronizes the § 8.2 arithmetic decoder and
fails the § 8.2.4 `exit_symbol()` backstop.

The decoder SHALL reject, with a structured `decode/unsupported-feature`
diagnostic BEFORE producing any output, every case outside the verified subset:
`NumTotalRefs > 2`, compound prediction / `reference_select`, more than two valid
reference slots, a fourth frame, a `single_ref` block that has a decoded neighbour
(the § 8.3.2 context is verified only for the no-neighbour context 1), and an inter
frame that would LOAD a prior ADAPTED frame's CDFs. The decoder does not model the
§ 7.23 cross-frame CDF save/load (every frame decodes from the default
`init_*_cdfs` state); a conformant decoder loads a prior frame's saved CDFs iff
`primary_ref_frame` names a real reference and `disable_cross_frame_cdf_init == 0`
(§ 5 :5426-5430), so an inter frame whose RESOLVED `primary_ref_frame` is a real
reference (`0..PRIMARY_REF_NONE`) with cross-frame CDF init enabled, when ANY prior
frame — the key OR an earlier inter — adapted (`disable_cdf_update == 0`), is
rejected before the tile entropy decode. `PRIMARY_REF_CHOOSE` (the unsignalled
placeholder splot does not resolve) is treated as no-load: every committed
broad-tools-off fixture carries `primary_ref_frame == PRIMARY_REF_CHOOSE` and
decodes bit-exact vs both oracles even with an adapted key, so for the admitted
subset CHOOSE does not become a desyncing load (resolving `PRIMARY_REF_CHOOSE` is a
named follow-on).

#### Scenario: the three-frame multi-reference fixture decodes bit-exact
- **WHEN** `splot decode syn-3frame-multiref-64x64.ivf --output-format raw` runs
- **THEN** the whole-stream raw output equals avmdec `--rawvideo --i420` and
  dav2d `--demuxer ivf --muxer yuv` byte-for-byte (md5
  `861078138ab514bd847ccfe22ac44fa1`, 18432 bytes)

#### Scenario: the third frame reads the retained inter frame, not the key
- **WHEN** the third frame's block decodes its § 5.20.7.12 `single_ref`
- **THEN** it selects `RefFrame[0] == 1` (the retained frame 1 in slot 1) and
  motion-compensates frame 1's samples (luma 160), NOT the key's (luma 100)
- **AND** the decoded third frame equals the decoded second frame and DIFFERS from
  the decoded key, so a wrong slot-0 selection would be falsifiable

#### Scenario: the § 7.7 two-valid-slot ranking is exact when RefBaseQIdx is modeled
- **WHEN** the implicit reference-map derivation runs over two valid slots with a
  reference-state view that models `RefBaseQIdx`
- **THEN** it derives `NumTotalRefs == 2` and `ref_frame_idx == [0, 1]` (the real
  § 7.7 ranking), not an `UnmodeledDerivation` stop
- **AND** a view that does NOT model `RefBaseQIdx` still stops with
  `UnmodeledDerivation` for two valid slots

#### Scenario: the § 7.23 update retains the second decoded frame
- **WHEN** an inter frame with `refresh_frame_flags` naming a fresh slot is decoded
- **THEN** the reference buffer marks that slot `RefValid` and stores the decoded
  frame plus its `RefOrderHint` / dims / `RefBaseQIdx`, so a later frame can
  reference it (two valid slots after key + one inter frame)

#### Scenario: cases outside the verified subset are rejected before output
- **WHEN** a stream presents more than two valid reference slots, `NumTotalRefs >
  2`, a compound reference, a fourth frame, a neighbour-having `single_ref` block,
  or an inter frame that would load a prior adapted frame's CDFs (the key or an
  earlier inter, via `primary_ref_frame != PRIMARY_REF_NONE` with cross-frame CDF
  init enabled)
- **THEN** the decoder emits a structured `decode/unsupported-feature` diagnostic
  and produces no output, never a confident-but-unverified frame

#### Scenario: existing fixtures are unchanged
- **WHEN** `splot decode` is given the existing inter and general-intra fixtures
- **THEN** each decodes to its previously-recorded bit-exact output
