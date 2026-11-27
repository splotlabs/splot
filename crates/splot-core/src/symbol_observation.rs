// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use std::sync::{Mutex, MutexGuard};

use super::{CDF_COUNT_SHIFT, CdfStorage};

/// The arithmetic coding operation that produced a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntropyCoding {
    /// One AV2 § 8.2.6 CDF-coded symbol.
    Cdf,
    /// One or more equiprobable AV2 § 8.2.3 bypass bits, most significant bit first.
    Bypass,
    /// A truncated unary value read through bypass coding.
    Unary,
}

/// The CDF row immediately before the decoder read the symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntropyCdf {
    /// Ascending Q15 cutoffs. Only the first `arity - 1` entries apply.
    pub cutoffs: [u16; 7],
    /// Number of symbols in the row.
    pub arity: u8,
    /// Index in the AV2 adaptation rate table.
    pub rate_index: u8,
    /// Number of adaptations, capped at 32.
    pub adaptation_count: u8,
    /// Whether this read adapts the row.
    pub updates: bool,
}

impl EntropyCdf {
    pub(super) fn new<T: CdfStorage>(cdf: &[T], arity: usize, updates: bool) -> Self {
        let mut cutoffs = [0; 7];
        for (cutoff, source) in cutoffs.iter_mut().zip(&cdf[..arity - 1]) {
            *cutoff = source.to_u32() as u16;
        }
        let state = cdf[arity - 1].to_u32();
        Self {
            cutoffs,
            arity: arity as u8,
            rate_index: (state & ((1 << CDF_COUNT_SHIFT) - 1)) as u8,
            adaptation_count: (state >> CDF_COUNT_SHIFT) as u8,
            updates,
        }
    }
}

/// An authoritative syntax read and its CDF selection context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntropySyntax {
    /// Stable syntax name within the evidence schema.
    pub key: &'static str,
    /// CDF selector arguments in schema order.
    pub context: &'static str,
    /// CDF selector values in the same order as the argument names.
    pub values: [u16; 3],
}

impl EntropySyntax {
    /// Identifies a syntax read and its CDF selector arguments.
    #[must_use]
    pub const fn new(key: &'static str, context: &'static str, values: [u16; 3]) -> Self {
        Self {
            key,
            context,
            values,
        }
    }
}

/// One captured range decoder operation within a selected scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntropySymbol {
    /// Zero-based ordinal within the scope. A multi-bit read advances by its bit count.
    pub ordinal: u64,
    /// Syntax identity and CDF selection arguments, when captured.
    pub syntax: Option<EntropySyntax>,
    pub(super) scope_tag: Option<[u32; 4]>,
    /// The decoded symbol or bypass value.
    pub value: u32,
    /// The coding operation.
    pub coding: EntropyCoding,
    /// Number of bypass bits. This is zero for CDF symbols.
    pub bypass_bits: u8,
    /// Pre-update probability state, available for CDF reads.
    pub cdf: Option<EntropyCdf>,
    /// The effective range before the read.
    pub range: u16,
    /// The chosen interval width before renormalization.
    pub interval: u16,
}

impl EntropySymbol {
    /// Returns the scope tag supplied by the caller for this read.
    #[must_use]
    pub const fn scope_tag(&self) -> Option<[u32; 4]> {
        self.scope_tag
    }

    /// Estimates the coding cost as `log2(range / interval)` for CDF reads.
    /// Each equiprobable bypass bit costs one bit. Returns `None` if a CDF
    /// interval is empty or exceeds the range. The estimate excludes termination and padding.
    #[must_use]
    pub fn estimated_bits(self) -> Option<f64> {
        if self.coding != EntropyCoding::Cdf {
            return Some(f64::from(self.bypass_bits));
        }
        (self.interval > 0 && self.interval <= self.range)
            .then(|| (f64::from(self.range) / f64::from(self.interval)).log2())
    }
}

/// Reason that a symbol observation stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EntropyCaptureStop {
    /// A record or byte limit prevented the next record.
    Limit,
    /// The cancellation predicate returned true.
    Cancelled,
    /// The capture could not reserve record storage.
    AllocationFailed,
}

/// Bounded entropy observations from one decoder scope.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct EntropyCapture {
    /// The record prefix in decode order.
    pub symbols: Vec<EntropySymbol>,
    /// Why capture stopped before the end of the scope, if applicable.
    pub stopped: Option<EntropyCaptureStop>,
    /// The observed scope's end ordinal. Counts units after a recording limit.
    pub end_ordinal: u64,
    next_syntax: Option<EntropySyntax>,
    scope_tag: Option<[u32; 4]>,
}

/// Record sink shared by an observing decoder and its temporary state copies.
pub struct EntropyObserver {
    capture: Mutex<EntropyCapture>,
    limit: usize,
    cancelled: Box<dyn Fn() -> bool + Send + Sync>,
}

impl std::fmt::Debug for EntropyObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntropyObserver").finish_non_exhaustive()
    }
}

impl EntropyObserver {
    /// Creates a sink bounded by the number and reserved storage of records.
    #[must_use]
    pub fn new(
        max_events: usize,
        max_bytes: usize,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            capture: Mutex::default(),
            limit: max_events.min(max_bytes / size_of::<EntropySymbol>()),
            cancelled: Box::new(cancelled),
        }
    }

    fn lock(&self) -> MutexGuard<'_, EntropyCapture> {
        self.capture
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Returns the observations after all decoder copies release this observer.
    #[must_use]
    pub fn into_capture(self) -> EntropyCapture {
        self.capture
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn annotate(&self, syntax: EntropySyntax) {
        self.lock().next_syntax = Some(syntax);
    }

    pub(super) fn replace_scope_tag(&self, tag: Option<[u32; 4]>) -> Option<[u32; 4]> {
        core::mem::replace(&mut self.lock().scope_tag, tag)
    }

    pub(super) fn push(&self, mut event: EntropySymbol) {
        let mut capture = self.lock();
        event.ordinal = capture.end_ordinal;
        event.syntax = capture.next_syntax.take();
        event.scope_tag = capture.scope_tag;
        capture.end_ordinal += u64::from(event.bypass_bits.max(1));
        if capture.stopped.is_some() {
            return;
        }
        capture.stopped = if (self.cancelled)() {
            Some(EntropyCaptureStop::Cancelled)
        } else if capture.symbols.len() == self.limit {
            Some(EntropyCaptureStop::Limit)
        } else {
            let current = capture.symbols.capacity();
            let next = current.saturating_mul(2).max(16).min(self.limit);
            if capture.symbols.len() != current
                || capture.symbols.try_reserve_exact(next - current).is_ok()
            {
                capture.symbols.push(event);
                None
            } else {
                Some(EntropyCaptureStop::AllocationFailed)
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdf_observation_precedes_each_adaptation() {
        use crate::symbol::SymbolDecoder;

        let mut decoder = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
        let observer = EntropyObserver::new(2, usize::MAX, || false);
        let mut row = [16384, 0, 0];
        decoder.with_entropy_observer(&observer, |scoped| {
            assert_eq!(scoped.read_trusted(&mut row), 0);
            assert_eq!(row, [17408, 1 << CDF_COUNT_SHIFT, 0]);
            let mut copied = scoped.clone();
            assert_eq!(copied.read_trusted(&mut row), 0);
            *scoped = copied;
        });

        let mut ordinary = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
        let mut ordinary_row = [16384, 0, 0];
        for _ in 0..2 {
            assert_eq!(ordinary.read_trusted(&mut ordinary_row), 0);
        }
        assert_eq!(decoder.checkpoint(), ordinary.checkpoint());
        assert_eq!(row, ordinary_row);
        assert_eq!(
            decoder.read_trusted(&mut row),
            ordinary.read_trusted(&mut ordinary_row)
        );
        assert_eq!(decoder.checkpoint(), ordinary.checkpoint());

        let capture = observer.into_capture();
        assert_eq!(capture.stopped, None);
        assert_eq!(capture.end_ordinal, 2);
        let first = capture.symbols[0].cdf.unwrap();
        let second = capture.symbols[1].cdf.unwrap();
        assert_eq!((first.cutoffs[0], first.adaptation_count), (16384, 0));
        assert_eq!((second.cutoffs[0], second.adaptation_count), (17408, 1));
    }

    #[test]
    fn entropy_estimate_uses_the_effective_interval() {
        let mut symbol = EntropySymbol {
            ordinal: 0,
            syntax: None,
            scope_tag: None,
            value: 0,
            coding: EntropyCoding::Cdf,
            bypass_bits: 0,
            cdf: None,
            range: 32768,
            interval: 8192,
        };
        assert_eq!(symbol.estimated_bits(), Some(2.0));
        symbol.interval = 32768;
        assert_eq!(symbol.estimated_bits(), Some(0.0));
        symbol.interval = 0;
        assert_eq!(symbol.estimated_bits(), None);
        symbol.interval = 65535;
        assert_eq!(symbol.estimated_bits(), None);
        symbol.coding = EntropyCoding::Unary;
        symbol.bypass_bits = 5;
        assert_eq!(symbol.estimated_bits(), Some(5.0));
    }
}
