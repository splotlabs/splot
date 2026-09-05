// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Typed errors for the AV2 bitstream writer (`ENC-BITSTREAM-WRITER`).
//!
//! The writer is the inverse of the [`crate::bitio::BitReader`] descriptors. A
//! [`WriteError`] is raised when a model value cannot be encoded by the requested
//! AV2 descriptor — for example a value too large for a fixed field, or a width
//! outside a descriptor's domain. These are *encoder-side* programming errors
//! (the caller asked for an impossible encoding), distinct from the parser's
//! conformance/EOF [`crate::error::Error`] variants, so the writer carries its own
//! self-contained error type and never touches the parser error model.

use thiserror::Error;

use crate::error::SymbolCdfErrorKind;

/// An AV2 bitstream-writer descriptor could not encode the requested value.
///
/// Every variant corresponds to a precondition of the matching
/// [`crate::bitio::BitReader`] descriptor: the writer rejects exactly the values
/// the reader could never have produced, so the round-trip property
/// `read(write(x)) == x` holds for every value the writer accepts.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum WriteError {
    /// A fixed-width write requested more bits than the descriptor allows
    /// (`f(n)`/`su(n)`/`rg(n)` accept `n <= 32`).
    #[error("bit width {requested} exceeds the maximum of {max}")]
    BitWidthTooLarge {
        /// The requested width, in bits.
        requested: u32,
        /// The maximum width the descriptor permits, in bits.
        max: u32,
    },

    /// A little-endian write requested more bytes than the descriptor allows
    /// (`le(n) -> u64` accepts `n <= 8`).
    #[error("byte width {requested} exceeds the maximum of {max}")]
    ByteWidthTooLarge {
        /// The requested width, in bytes.
        requested: u32,
        /// The maximum width the descriptor permits, in bytes.
        max: u32,
    },

    /// A descriptor that requires a positive width was given zero (e.g. `ns(0)`).
    #[error("the {descriptor} descriptor requires a width greater than zero")]
    ZeroWidth {
        /// The AV2 descriptor name (`"ns"`).
        descriptor: &'static str,
    },

    /// A value does not fit in the requested fixed field width.
    #[error("value {value} does not fit in {width_bits} bit(s)")]
    ValueTooWide {
        /// The offending value.
        value: u64,
        /// The field width that cannot hold it, in bits.
        width_bits: u32,
    },

    /// A value lies outside the range the descriptor can encode (`su(n)` signed
    /// range, `ns(n)` `0..n`, `uvlc`/`svlc` conformance bound, or `rg(n)` whose
    /// unary prefix would not terminate within 32 bits).
    #[error("the {descriptor} descriptor cannot encode value {value}")]
    ValueOutOfRange {
        /// The AV2 descriptor name (`"su"`, `"ns"`, `"uvlc"`, `"svlc"`, `"rg"`).
        descriptor: &'static str,
        /// The offending value, widened to `i64` so both signed and unsigned
        /// descriptors share one variant.
        value: i64,
    },

    /// `trailing_bits(0)` was requested. The parser rejects an empty trailing-bits
    /// field (AV2 v1.0.0 § 5.2.3,
    /// `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3`, always writes at least
    /// the `trailing_one_bit`), so the writer never produces one.
    #[error("trailing_bits requires at least one bit")]
    EmptyTrailingBits,

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `has_header_extension` flag
    /// disagrees with its `header_size_bytes` (the flag is `true` iff the header is
    /// two bytes). Such a header could never have been produced by the parser.
    #[error(
        "OBU header extension flag ({flag}) is inconsistent with header_size_bytes ({size_bytes})"
    )]
    InconsistentHeader {
        /// The header's `has_header_extension` flag.
        flag: bool,
        /// The header's `header_size_bytes`.
        size_bytes: u8,
    },

    /// A no-extension [`ObuHeader`](crate::obu::ObuHeader) carries layer ids that the
    /// AV2 v1.0.0 § 5.2.2 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2`)
    /// parser could never infer (it derives `obu_mlayer_id = 0` and `obu_xlayer_id`
    /// to `GLOBAL_XLAYER_ID` for the global-scope types or `0` otherwise). Such ids are unrepresentable without the extension byte, so the
    /// writer rejects them rather than silently dropping them.
    #[error(
        "no-extension OBU header has non-inferable layer ids (mlayer {embedded}, xlayer {extended})"
    )]
    NonInferableLayerIds {
        /// The header's `embedded_layer_id` (`obu_mlayer_id`).
        embedded: u8,
        /// The header's `extended_layer_id` (`obu_xlayer_id`).
        extended: u8,
    },

    /// An Annex B OBU's total byte count (`header_size_bytes + payload.len()`)
    /// exceeds the LEB128 `u32` size domain (AV2 v1.0.0 § 4.11.6,
    /// `docs/spec/av2/1.0.0/04-conventions.md#s-4-11-6`).
    #[error("OBU size {total} exceeds the u32 leb128 domain")]
    ObuTooLarge {
        /// The computed total byte count that does not fit in a `u32`.
        total: u64,
    },

    /// A byte-granular framer (e.g. `write_annexb_obu`) was given a writer that is
    /// not on a byte boundary; the bytes it emits would be mis-positioned. The error
    /// is returned before any byte is written.
    #[error("writer is not byte-aligned")]
    WriterNotByteAligned,

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `obu_type` is a non-canonical
    /// `ObuType::Reserved(raw)` whose raw value the § 5.2.2 parser maps to a
    /// different variant on reparse (e.g. `Reserved(1)` reparses as a named type), so
    /// writing it would break `read(write(x)) == x`. Rejected before any byte.
    #[error("non-canonical obu_type with raw value {raw}")]
    NonCanonicalObuType {
        /// The header's `obu_type.raw()`.
        raw: u8,
    },

    /// A sequence-header value disagrees with its derived fields or presence flags (§ 5.4.1).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1`.
    #[error("non-canonical {what}: model value cannot be reproduced by the §5.4 parser")]
    NonCanonicalSequenceValue {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A [`SequenceHeader`](crate::headers::sequence::SequenceHeader) the AV2 v1.0.0
    /// § 5.4.2 (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2`) parser could not
    /// fully parse: `seq_tile_info_present_flag == 1` while `seq_level_idx` is a reserved
    /// (non-conformant) level with no defined `tile_params()` (§ 5.18.7.3) bit layout, so
    /// the parser left a bounded residual (`SequenceHeader::unimplemented_at` /
    /// `SequenceTileConfig::unimplemented_at`) and never modeled the tile bits or any
    /// payload after them. The un-modeled tail cannot be re-emitted, so the writer rejects
    /// the whole header before writing any bit rather than producing a truncated stream.
    #[error("sequence header is not fully parsed (stopped at {feature})")]
    UnwritableSequenceHeader {
        /// The owning Feature ID at which the parser stopped (e.g.
        /// `"AV2-5.4.2-SEQUENCE-TILE-CONFIG"`).
        feature: &'static str,
    },

    /// A frame-header value disagrees with its derived fields or presence flags (§ 5.18.2).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`.
    #[error("non-canonical {what}: frame-header value cannot be reproduced by the §5.18 parser")]
    NonCanonicalFrameHeader {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A tile-group structure or payload cannot be reproduced (§ 5.19 / § 5.20.1).
    /// Inverted or out-of-range tile ranges are also rejected under § 6.18, even
    /// when the parser preserves them.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1`.
    #[error(
        "non-canonical {what}: tile-group value cannot be reproduced by the §5.19/§5.20.1 parser"
    )]
    NonCanonicalTileGroup {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A metadata value disagrees with its payload, declared sizes or presence flags (§ 5.17).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-17`.
    #[error("non-canonical {what}: metadata value cannot be reproduced by the §5.17 parser")]
    NonCanonicalMetadata {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A buffer-removal-timing value disagrees with its counts or presence flags (§ 5.12).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-12`.
    #[error(
        "non-canonical {what}: buffer-removal-timing value cannot be reproduced by the §5.12 parser"
    )]
    NonCanonicalBufferRemovalTiming {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A multistream-decoder-operation value disagrees with its counts or presence flags (§ 5.6).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-6`.
    #[error(
        "non-canonical {what}: multistream-decoder-operation value cannot be reproduced by the §5.6 parser"
    )]
    NonCanonicalMsdo {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// An operating-point-set value disagrees with its layer maps, sizes or presence flags (§ 5.10 / § 5.11).
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-10`.
    #[error(
        "non-canonical {what}: operating-point-set value cannot be reproduced by the §5.10/§5.11 parser"
    )]
    NonCanonicalOperatingPointSet {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A multi-frame-header value disagrees with its derived fields (§ 5.7).
    /// Out-of-conformance IDs preserved by the parser are reproduced verbatim.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-7`.
    #[error(
        "non-canonical {what}: multi-frame-header value cannot be reproduced by the §5.7 parser"
    )]
    NonCanonicalMultiFrameHeader {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A content-interpretation value disagrees with its derived fields (§ 5.15).
    /// Reserved values preserved by the parser are reproduced verbatim.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-15`.
    #[error(
        "non-canonical {what}: content-interpretation value cannot be reproduced by the §5.15 parser"
    )]
    NonCanonicalContentInterpretation {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// An atlas-segment value disagrees with its mode, counts or presence flags (§ 5.9).
    /// Descriptive segment IDs preserved by the parser are reproduced verbatim.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-9`.
    #[error("non-canonical {what}: atlas-segment value cannot be reproduced by the §5.9 parser")]
    NonCanonicalAtlasSegment {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A layer-configuration-record value disagrees with its scope, maps, sizes or presence flags (§ 5.8).
    /// Reserved fields and IDs are reproduced within their descriptor domains.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-8`.
    #[error(
        "non-canonical {what}: layer-config-record value cannot be reproduced by the §5.8 parser"
    )]
    NonCanonicalLayerConfigRecord {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A film-grain value cannot be reproduced (§ 5.14 / § 5.18.10.2).
    /// Widths absent from the model are canonicalized; semantic equality does not
    /// imply byte equality.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-14`, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-2`.
    #[error("non-canonical {what}: film-grain value cannot be reproduced by the §5.14 parser")]
    NonCanonicalFilmGrain {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A quantizer-matrix value cannot be reproduced (§ 5.13 / § 5.4.11).
    /// Coefficients are emitted without optional wire compression; semantic
    /// equality does not imply byte equality.
    /// See `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-13`, `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-4-11`.
    #[error(
        "non-canonical {what}: quantizer-matrix value cannot be reproduced by the §5.13 parser"
    )]
    NonCanonicalQuantizationMatrix {
        /// Stable label for the offending field.
        what: &'static str,
    },

    /// A caller-supplied CDF row cannot be used by the AV2 § 8.2 symbol encoder.
    #[error("invalid symbol CDF: {kind}")]
    InvalidSymbolCdf {
        /// Specific CDF-row violation.
        kind: SymbolCdfErrorKind,
    },

    /// The requested symbol is outside the supplied CDF row's arity.
    #[error("symbol {symbol} is outside the {symbols}-symbol CDF row")]
    SymbolOutOfRange {
        /// Requested symbol value.
        symbol: u8,
        /// Number of symbols represented by the CDF row.
        symbols: usize,
    },

    /// A symbol-encoder arithmetic interval collapsed before renormalization.
    #[error("symbol encoder arithmetic interval collapsed")]
    SymbolArithmeticRange,

    /// A symbol encoder payload would exceed its configured byte limit.
    #[error("symbol encoder payload would require {requested} byte(s), exceeding limit {limit}")]
    SymbolOutputTooLarge {
        /// Required output bytes.
        requested: usize,
        /// Configured maximum output bytes.
        limit: usize,
    },

    /// A symbol encoder stream would exceed its configured operation count limit.
    #[error(
        "symbol encoder stream would require {requested} operation(s), exceeding limit {limit}"
    )]
    SymbolOperationLimit {
        /// Required primitive operation count.
        requested: usize,
        /// Configured maximum primitive operation count.
        limit: usize,
    },

    /// A valid final symbol payload could not be constructed for the committed operations.
    #[error("symbol encoder could not construct a valid finalized payload")]
    SymbolFinalizationFailed,

    /// An [`ObuHeader`](crate::obu::ObuHeader)'s `obu_type` does not select the
    /// [`ParsedObu`](crate::obu::ParsedObu) payload variant it was paired with in the complete-OBU
    /// writer (e.g. a `SequenceHeader` header with a `Padding` payload). The § 5.2.1 OBU dispatch
    /// routes a single `obu_type` to exactly one payload syntax, so such a pair could never have come
    /// from parsing one OBU; writing it would reparse as the header's type and break
    /// `read(write(x)) == x`. Rejected before any bit is written.
    #[error("OBU header type does not select the {payload} payload")]
    ObuTypePayloadMismatch {
        /// The mispaired payload's syntax name ([`ParsedObu::syntax_name`](crate::obu::ParsedObu::syntax_name)).
        payload: &'static str,
    },
}

/// Result alias for [`crate::write::BitWriter`] operations.
pub type WriteResult<T> = core::result::Result<T, WriteError>;
