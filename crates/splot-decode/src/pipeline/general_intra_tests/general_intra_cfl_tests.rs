// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::{assert_hash, decode_eight};

const CFL_444_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-cfl-444-intra-64x64-q255.ivf"
);
const CFL_422_FIXTURE: &[u8] = include_bytes!(
    "../../../../../tests/conformance/vectors/valid/syn-cfl-422-intra-64x64-q128.ivf"
);
const CFL_420_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-intra-128x128.ivf");
const MHCCP_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-mhccp-intra-128x128.ivf");
const CCTX_444_FIXTURE: &[u8] =
    include_bytes!("../../../../../tests/conformance/vectors/valid/syn-cctx-444-intra-128x128.ivf");

#[test]
fn chroma_pair_paths_decode_to_oracles() {
    for (fixture, expected_hash) in [
        (
            CFL_444_FIXTURE,
            "075641becbde523eb5b6dbd542ae0df161b24f5a36710fc4cb6497dad85455bb",
        ),
        (
            CFL_422_FIXTURE,
            "8f9809413ac43dd23e7045d9c95ef27ec392beb38b5582edd71c61bea7e298f1",
        ),
        (
            CFL_420_FIXTURE,
            "294d914f3d6c61339876b7d91c97a1762011b3afae36850b91f89a04e850c6ce",
        ),
        (
            MHCCP_FIXTURE,
            "4d95ed1b9a1ae7188b06a3b991355e1822ca456dcb9e58aafebb6dfc05a7258a",
        ),
        (
            CCTX_444_FIXTURE,
            "61854be209ee4caceec47baf4bd5d2b796e296d5451f89973f0e2e519a524a05",
        ),
    ] {
        assert_hash(&decode_eight(fixture), expected_hash);
    }
}
