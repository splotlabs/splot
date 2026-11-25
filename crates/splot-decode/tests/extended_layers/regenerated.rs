// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn independently_encoded_layers_match_avm_across_reset_and_sparse_updates() {
    for (input, full_digest, component_digests) in [
        (
            include_bytes!(
                "../../../../tests/conformance/vectors/valid/syn-xlayer-independent-reset-10bit.obu"
            )
            .as_slice(),
            "64e3f689e23f99b8f2191a3baef6dab2d8d9c75a193bf59f1658407b6a7e73c0",
            [
                "895a1af44834b24d2fa4121df473b254aa78f04c3af9201fa10b2a56c6604aff",
                "34864578f19ed9a8a32591c6e3eb37334c35c6b6f097cbb63e4a38036cca5a26",
            ],
        ),
        (
            include_bytes!(
                "../../../../tests/conformance/vectors/valid/syn-xlayer-independent-reset-8bit.obu"
            )
            .as_slice(),
            "816f3401a23ab9ce43d7384dc7f4a5b3dee2f7e4d1518367ad51e182d2d1e108",
            [
                "15f1abf6e55b7a3b644ab81dcc00ee9648e1385559422b7565d180423492a648",
                "f33818e3c9463d2a72800333181eeb81014169ffc1e643b91c98e93cc9c1875b",
            ],
        ),
        (
            include_bytes!(
                "../../../../tests/conformance/vectors/valid/syn-xlayer-independent-sparse-8bit.obu"
            )
            .as_slice(),
            "82653d6b8266b055b89c6f366f4f4efa450a455d048602da73e0d4ac61f3ffa8",
            [
                "80f2b107ff4512837e25b724f380259077ad451f73e7cb2c046e64feee8062cc",
                "f33818e3c9463d2a72800333181eeb81014169ffc1e643b91c98e93cc9c1875b",
            ],
        ),
    ] {
        for threads in [1usize, 3, 12] {
            let pictures = stream_decode_with_settings(
                input,
                &DecoderSettings::default().with_threads(ThreadCount::from(threads)),
            )
            .unwrap();
            let raw = pictures_to_raw(&pictures.iter().collect::<Vec<_>>());
            assert_eq!(sha256_hex(&raw), full_digest);
            for (layer, digest) in [3, 7].into_iter().zip(component_digests) {
                let component = raw_for_xlayer(&pictures, layer);
                assert_eq!(sha256_hex(&component), digest);
                let mut selected = Vec::new();
                context(threads, 2)
                    .decode_raw_reader(
                        ChunkedReader::new(input, 7),
                        DecodeOptions::default().with_extended_layer_selection(
                            ExtendedLayerSelection::Target(ExtendedLayerId::from_bits(layer)),
                        ),
                        &mut selected,
                    )
                    .unwrap();
                assert_eq!(selected, component);
            }
        }
    }
}
