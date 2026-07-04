## Why

The local decoder mission stream has advanced to a coarse pre-tile unsupported-tool gate even though several enabled sequence tools only need active-use checks on the observed path. The next decoder brick should consume the safe inactive syntax in spec order and fail closed only when the stream actually selects unsupported active MRL or transform-tool behavior.

## What Changes

- Add `DECODE-ACTIVE-INTRA-TOOL-FRONTIER` as the next partial local decoder mission decoder row.
- Wire generated AV2 §9.3 MRL CDF rows into the tile CDF selector/lifecycle subset.
- Consume AV2 §5.20.5.5 `mrl_index` and `mrl_sec_index` when `enable_mrls` is set and luma mode is directional.
- Admit enabled-but-inactive intra/transform sequence tools in selectable Wiener NS LR transform-record derivation, while rejecting active nonzero MRL and nonzero residual branches that would require unsupported transform-type/CCTX/IST syntax.
- Keep the local decoder mission runtime probe fail-closed at the next true unsupported frontier; do not emit decoded output until it can be proven against AVM/dav2d.

## Capabilities

### New Capabilities
- `active-intra-tool-frontier`: fail-closed active-use admission for local decoder mission selectable transform records with MRL syntax consumption.

### Modified Capabilities
- `decoder-support`: add the corresponding partial support row and matrix evidence requirement.

## Impact

- Affects `splot-decode` tile CDF rows, CDF lifecycle, general intra mode parsing, selectable Wiener NS LR transform-record derivation, focused tests, local decoder mission CLI probe expectation, and generated support/status docs.
- No new dependencies, public API changes, encoder work, broad AV2 transform-type implementation, reconstruction/output claim, reference refresh claim, or AVM/dav2d byte-equality claim.
