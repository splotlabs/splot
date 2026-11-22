// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Compile-time layout of CDF rows in one contiguous `u16` store.
//!
//! Rows of one length are adjacent, so a whole-table operation is one loop
//! per row length, whatever the number of syntax elements.

/// The shortest row: one of a two-symbol element (§ 8.2.6).
pub(super) const MIN_ROW_LEN: usize = 3;
/// The longest row: one of an eight-symbol element.
pub(super) const MAX_ROW_LEN: usize = 9;
const ROW_LENS: usize = MAX_ROW_LEN - MIN_ROW_LEN + 1;

/// Store offsets by row length: the rows of length `len` occupy
/// `bounds[len - MIN_ROW_LEN]..bounds[len - MIN_ROW_LEN + 1]`.
pub(super) type RowLenBounds = [usize; ROW_LENS + 1];

/// The rows of one syntax element in a store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RowSpan {
    pub(super) offset: usize,
    pub(super) row_len: usize,
    pub(super) rows: usize,
}

impl RowSpan {
    pub(super) const fn end(self) -> usize {
        self.offset + self.rows * self.row_len
    }
}

/// Where each of `E` syntax elements lives in a store of `len` entries.
#[derive(Clone, Copy, Debug)]
pub(super) struct CdfLayout<const E: usize> {
    pub(super) spans: [RowSpan; E],
    pub(super) bounds: RowLenBounds,
    pub(super) len: usize,
}

/// Lays out elements given as `(row_len, rows)`, by row length and then in
/// declaration order.
pub(super) const fn cdf_layout<const E: usize>(shapes: [(usize, usize); E]) -> CdfLayout<E> {
    let mut spans = [RowSpan {
        offset: 0,
        row_len: 0,
        rows: 0,
    }; E];
    let mut bounds = [0; ROW_LENS + 1];
    let mut offset = 0;
    let mut placed = 0;
    let mut row_len = MIN_ROW_LEN;
    while row_len <= MAX_ROW_LEN {
        let mut element = 0;
        while element < E {
            let (len, rows) = shapes[element];
            if len == row_len {
                spans[element] = RowSpan {
                    offset,
                    row_len,
                    rows,
                };
                offset += rows * len;
                placed += 1;
            }
            element += 1;
        }
        bounds[row_len + 1 - MIN_ROW_LEN] = offset;
        row_len += 1;
    }
    assert!(placed == E, "a CDF row length has no symbol arity");
    CdfLayout {
        spans,
        bounds,
        len: offset,
    }
}

/// Copies an element's default rows into a store under construction.
pub(super) const fn set_rows(store: &mut [u16], span: RowSpan, rows: &[u16]) {
    let (_, tail) = store.split_at_mut(span.offset);
    let (target, _) = tail.split_at_mut(span.rows * span.row_len);
    target.copy_from_slice(rows);
}

/// The rows of length `N` in a store laid out with `bounds`.
pub(super) fn rows_of_len<'a, const N: usize>(
    store: &'a [u16],
    bounds: &RowLenBounds,
) -> &'a [[u16; N]] {
    let (start, end) = (bounds[N - MIN_ROW_LEN], bounds[N + 1 - MIN_ROW_LEN]);
    store[start..end].as_chunks().0
}

/// [`rows_of_len`] for writing.
pub(super) fn rows_of_len_mut<'a, const N: usize>(
    store: &'a mut [u16],
    bounds: &RowLenBounds,
) -> &'a mut [[u16; N]] {
    let (start, end) = (bounds[N - MIN_ROW_LEN], bounds[N + 1 - MIN_ROW_LEN]);
    store[start..end].as_chunks_mut().0
}

/// Expands `$visit!(len)` for every row length a store can hold.
macro_rules! for_each_cdf_row_len {
    ($visit:ident) => {
        $visit!(3);
        $visit!(4);
        $visit!(5);
        $visit!(6);
        $visit!(7);
        $visit!(8);
        $visit!(9);
    };
}

const _: () = {
    let mut visited = 0;
    macro_rules! count {
        ($len:literal) => {
            assert!(MIN_ROW_LEN <= $len && $len <= MAX_ROW_LEN);
            visited += 1;
        };
    }
    for_each_cdf_row_len!(count);
    assert!(visited == ROW_LENS);
};

/// The array type of an element: its row nested in one array per context
/// dimension, outermost first.
macro_rules! cdf_rows_type {
    ($len:tt;) => { [u16; $len] };
    ($len:tt; [$dim:expr] $([$rest:expr])*) => { [cdf_rows_type!($len; $([$rest])*); $dim] };
}

/// An element's default rows as one `u16` slice.
macro_rules! cdf_rows_flat {
    ($rows:expr;) => { $rows.as_slice() };
    ($rows:expr; [$dim:expr] $([$rest:expr])*) => {
        cdf_rows_flat!($rows; $([$rest])*).as_flattened()
    };
}

/// An element's part of the store as its array type: one `as_chunks_mut`
/// per array level, whose lengths the result type fixes.
macro_rules! cdf_rows_view {
    ($flat:expr;) => { $flat.as_chunks_mut().0 };
    ($flat:expr; [$dim:expr] $([$rest:expr])*) => {
        cdf_rows_view!($flat; $([$rest])*).as_chunks_mut().0
    };
}

/// Declares a CDF table: a struct that keeps the rows of every listed
/// syntax element in one store, an accessor per element that returns its
/// rows as nested arrays, and the defaults the store starts from.
///
/// An element is `name[dim]..: row_len = default`, its context dimensions
/// outermost first and `row_len` the symbol count plus one. Fields after
/// `..` are stored beside the rows as declared.
macro_rules! cdf_rows {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident[$element:ident]($($arg:ident: $arg_ty:ty),*) {
            $($field:ident $([$dim:expr])*: $len:tt = $default:expr,)+
            ..
            $($sub_vis:vis $sub:ident: $sub_ty:ty = $sub_default:expr,)*
        }
    ) => {
        #[allow(non_camel_case_types, reason = "named as the accessors are")]
        #[derive(Clone, Copy)]
        enum $element {
            $($field),+
        }

        impl $element {
            const COUNT: usize = [$(Self::$field),+].len();
            const LAYOUT: $crate::bitstream::tile_payload::cdf::layout::CdfLayout<
                { $element::COUNT },
            > = $crate::bitstream::tile_payload::cdf::layout::cdf_layout([
                $(($len, 1 $(* $dim)*)),+
            ]);

            const fn span(self) -> $crate::bitstream::tile_payload::cdf::layout::RowSpan {
                Self::LAYOUT.spans[self as usize]
            }
        }

        $(#[$meta])*
        $vis struct $name {
            store: [u16; $element::LAYOUT.len],
            $($sub_vis $sub: $sub_ty,)*
        }

        impl $name {
            const BOUNDS: $crate::bitstream::tile_payload::cdf::layout::RowLenBounds =
                $element::LAYOUT.bounds;

            pub(crate) const fn from_defaults($($arg: $arg_ty),*) -> Self {
                let mut store = [0; $element::LAYOUT.len];
                $(
                    let rows: &cdf_rows_type!($len; $([$dim])*) = &$default;
                    $crate::bitstream::tile_payload::cdf::layout::set_rows(
                        &mut store,
                        $element::$field.span(),
                        cdf_rows_flat!(rows; $([$dim])*),
                    );
                )+
                Self {
                    store,
                    $($sub: $sub_default,)*
                }
            }

            $(
                #[inline]
                pub(crate) fn $field(&mut self) -> &mut cdf_rows_type!($len; $([$dim])*) {
                    const SPAN: $crate::bitstream::tile_payload::cdf::layout::RowSpan =
                        $element::$field.span();
                    let flat = &mut self.store[SPAN.offset..SPAN.end()];
                    &mut cdf_rows_view!(flat; $([$dim])*)[0]
                }
            )+
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_groups_rows_by_length_in_declaration_order() {
        let layout = cdf_layout([(4, 2), (3, 5), (9, 1), (4, 1)]);
        let offsets = layout.spans.map(|span| span.offset);
        assert_eq!(offsets, [15, 0, 27, 23]);
        assert_eq!(layout.bounds, [0, 15, 27, 27, 27, 27, 27, 36]);
        assert_eq!(layout.len, 36);

        let mut store: [u16; 36] = core::array::from_fn(|entry| entry as u16);
        let fours: &[[u16; 4]] = rows_of_len(&store, &layout.bounds);
        assert_eq!(
            fours,
            [[15, 16, 17, 18], [19, 20, 21, 22], [23, 24, 25, 26]]
        );
        set_rows(&mut store, layout.spans[3], &[1, 2, 3, 4]);
        assert_eq!(store[23..27], [1, 2, 3, 4]);
        assert_eq!(rows_of_len_mut::<9>(&mut store, &layout.bounds).len(), 1);
    }
}
