// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 bitstream syntax observation and bounded bit-span recording.
//!
//! Every header parser in [`crate::headers`], [`crate::hls`], and [`crate::obu`]
//! runs on a single internal implementation generic over [`SyntaxObserver`].
//! Calling a standard `parse_*` entrypoint installs the zero-sized
//! [`NoopObserver`], which monomorphizes away every observation call on the
//! decode path. Calling a `*_observed` entrypoint passes `&mut dyn SyntaxObserver`
//! (such as [`SyntaxRecorder`]) to receive canonical AV2 v1.0.0 syntax names,
//! nesting scopes, descriptors, raw values, exact OBU-payload-relative
//! [`BitSpan`]s, conditional absence / inferred defaults, and located errors.
//!
//! # Example
//!
//! ```
//! use splot_core::bitio::BitReader;
//! use splot_core::headers::parse_buffer_removal_timing_observed;
//! use splot_core::span::{BitSpan, ByteOffset};
//! use splot_core::syntax::{SyntaxRecorder, SyntaxValue};
//!
//! let payload = [0x01, 0x23];
//! let mut reader = BitReader::new(&payload, ByteOffset::new(0));
//! let mut recorder = SyntaxRecorder::with_default_limits();
//! let _ = parse_buffer_removal_timing_observed(&mut reader, &mut recorder)?;
//! let trace = recorder.finish();
//! let first = trace.fields_read().next();
//! assert_eq!(
//!     first.map(|f| (f.name.as_ref(), f.span, f.value)),
//!     Some((
//!         "br_ops_dependent_flag",
//!         BitSpan::new(0, 1),
//!         SyntaxValue::Bool(false),
//!     ))
//! );
//! # Ok::<(), splot_core::error::Error>(())
//! ```

use core::fmt;
use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::span::BitSpan;

pub(crate) mod reader;

pub(crate) use reader::SyntaxReader;

/// AV2 v1.0.0 bitstream descriptor used to read a syntax element (§ 4.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SyntaxDescriptor {
    /// Fixed-width unsigned integer `f(n)` (AV2 § 4.11.2).
    F(u32),
    /// Unsigned variable-length code `uvlc()` (AV2 § 4.11.3).
    Uvlc,
    /// Signed variable-length code `svlc()` (AV2 § 4.11.4).
    Svlc,
    /// Unsigned LEB128 integer `leb128()` (AV2 § 4.11.6).
    Leb128,
    /// Signed fixed-width integer `su(n)` (AV2 § 4.11.7).
    Su(u32),
    /// Non-symmetric unsigned integer `ns(n)` (AV2 § 4.11.8).
    Ns(u32),
    /// Rice-Golomb unsigned integer `rg(n)` (AV2 § 4.11.10).
    Rg(u32),
    /// Truncated unary `tu(mx)` (for example, loop-restoration unit size shift).
    Tu(u32),
    /// Sub-exponential code `decode_subexp` (AV2 § 5.18.7.9).
    Subexp,
    /// Raw opaque byte sequence of the given length in bytes.
    Bytes(usize),
}

impl fmt::Display for SyntaxDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::F(n) => write!(f, "f({n})"),
            Self::Uvlc => f.write_str("uvlc()"),
            Self::Svlc => f.write_str("svlc()"),
            Self::Leb128 => f.write_str("leb128()"),
            Self::Su(n) => write!(f, "su({n})"),
            Self::Ns(n) => write!(f, "ns({n})"),
            Self::Rg(n) => write!(f, "rg({n})"),
            Self::Tu(mx) => write!(f, "tu({mx})"),
            Self::Subexp => f.write_str("subexp"),
            Self::Bytes(len) => write!(f, "bytes({len})"),
        }
    }
}

/// Raw value of an observed or inferred AV2 syntax element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SyntaxValue {
    /// Single-bit boolean flag (`f(1)`).
    Bool(bool),
    /// Unsigned integer value.
    Unsigned(u64),
    /// Signed integer value.
    Signed(i64),
    /// Opaque byte sequence represented by its length in bytes.
    Bytes {
        /// Number of bytes covered by the opaque field.
        len: usize,
    },
}

impl From<bool> for SyntaxValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<u8> for SyntaxValue {
    fn from(v: u8) -> Self {
        Self::Unsigned(u64::from(v))
    }
}

impl From<u32> for SyntaxValue {
    fn from(v: u32) -> Self {
        Self::Unsigned(u64::from(v))
    }
}

impl From<i32> for SyntaxValue {
    fn from(v: i32) -> Self {
        Self::Signed(i64::from(v))
    }
}

impl fmt::Display for SyntaxValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Bool(v) => write!(f, "{}", u8::from(v)),
            Self::Unsigned(v) => write!(f, "{v}"),
            Self::Signed(v) => write!(f, "{v}"),
            Self::Bytes { len } => write!(f, "<{len} bytes>"),
        }
    }
}

/// Nesting scope entered during syntax parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScopeKind {
    /// A named AV2 syntax structure (e.g. `sequence_header_obu`, `tile_info`).
    Structure {
        /// Canonical AV2 syntax structure name.
        name: &'static str,
        /// AV2 v1.0.0 specification section (e.g. `"5.4.1"`), if applicable.
        spec_section: Option<&'static str>,
    },
    /// One iteration of an indexed syntax loop.
    LoopItem {
        /// Loop variable or collection name (e.g. `"i"`, `"plane"`, `"ref"`).
        name: &'static str,
        /// Zero-based iteration index.
        index: usize,
    },
    /// A conditional syntax branch.
    Condition {
        /// Canonical AV2 condition expression.
        condition: &'static str,
        /// Whether the condition evaluated to `true`.
        taken: bool,
    },
}

/// Owned, serializable form of [`ScopeKind`] stored in a [`SyntaxTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RecordedScope {
    /// A named AV2 syntax structure.
    Structure {
        /// Canonical AV2 syntax structure name.
        name: Cow<'static, str>,
        /// AV2 v1.0.0 specification section, if applicable.
        spec_section: Option<Cow<'static, str>>,
    },
    /// One iteration of an indexed syntax loop.
    LoopItem {
        /// Loop variable or collection name.
        name: Cow<'static, str>,
        /// Zero-based iteration index.
        index: usize,
    },
    /// A conditional syntax branch.
    Condition {
        /// Canonical AV2 condition expression.
        condition: Cow<'static, str>,
        /// Whether the condition evaluated to `true`.
        taken: bool,
    },
}

impl From<ScopeKind> for RecordedScope {
    fn from(scope: ScopeKind) -> Self {
        match scope {
            ScopeKind::Structure { name, spec_section } => Self::Structure {
                name: Cow::Borrowed(name),
                spec_section: spec_section.map(Cow::Borrowed),
            },
            ScopeKind::LoopItem { name, index } => Self::LoopItem {
                name: Cow::Borrowed(name),
                index,
            },
            ScopeKind::Condition { condition, taken } => Self::Condition {
                condition: Cow::Borrowed(condition),
                taken,
            },
        }
    }
}

/// A syntax field read directly from the bitstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxFieldRead {
    /// Canonical AV2 v1.0.0 syntax element name.
    pub name: &'static str,
    /// Descriptor used to read the field.
    pub descriptor: SyntaxDescriptor,
    /// Raw value as read from the bitstream.
    pub value: SyntaxValue,
    /// Exact bit span, relative to the start of the observed payload.
    pub span: BitSpan,
}

/// Owned, serializable form of [`SyntaxFieldRead`] stored in a [`SyntaxTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordedFieldRead {
    /// Canonical AV2 v1.0.0 syntax element name.
    pub name: Cow<'static, str>,
    /// Descriptor used to read the field.
    pub descriptor: SyntaxDescriptor,
    /// Raw value as read from the bitstream.
    pub value: SyntaxValue,
    /// Exact bit span, relative to the start of the observed payload.
    pub span: BitSpan,
}

impl From<SyntaxFieldRead> for RecordedFieldRead {
    fn from(field: SyntaxFieldRead) -> Self {
        Self {
            name: Cow::Borrowed(field.name),
            descriptor: field.descriptor,
            value: field.value,
            span: field.span,
        }
    }
}

/// A syntax field absent from the bitstream because of a syntax condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxFieldAbsent {
    /// Canonical AV2 v1.0.0 syntax element name.
    pub name: &'static str,
    /// Canonical condition explaining why the field was not coded.
    pub condition: &'static str,
    /// Value inferred by AV2 v1.0.0 when absent, or `None` if no value is inferred.
    pub inferred: Option<SyntaxValue>,
}

/// Owned, serializable form of [`SyntaxFieldAbsent`] stored in a [`SyntaxTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordedFieldAbsent {
    /// Canonical AV2 v1.0.0 syntax element name.
    pub name: Cow<'static, str>,
    /// Canonical condition explaining why the field was not coded.
    pub condition: Cow<'static, str>,
    /// Value inferred by AV2 v1.0.0 when absent, or `None` if no value is inferred.
    pub inferred: Option<SyntaxValue>,
}

impl From<SyntaxFieldAbsent> for RecordedFieldAbsent {
    fn from(field: SyntaxFieldAbsent) -> Self {
        Self {
            name: Cow::Borrowed(field.name),
            condition: Cow::Borrowed(field.condition),
            inferred: field.inferred,
        }
    }
}

/// A parse error reported with its exact OBU-payload-relative bit position.
#[derive(Debug)]
pub struct SyntaxErrorRef<'a> {
    /// Zero-based bit offset from the start of the observed payload where the error occurred.
    pub bit_offset: u64,
    /// Canonical AV2 syntax field being read or validated, if applicable.
    pub field: Option<&'static str>,
    /// Underlying typed parser error.
    pub error: &'a Error,
}

/// Owned, serializable form of [`SyntaxErrorRef`] stored in a [`SyntaxTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedSyntaxError {
    /// Zero-based bit offset from the start of the observed payload where the error occurred.
    pub bit_offset: u64,
    /// Canonical AV2 syntax field being read or validated, if applicable.
    pub field: Option<Cow<'static, str>>,
    /// Human-readable description formatted from [`Error`].
    pub message: String,
}

/// Observer receiving AV2 syntax structure, field, and error events during parsing.
///
/// Events arrive in exact bitstream parse order. Every [`Self::enter_scope`] is
/// matched by a corresponding [`Self::exit_scope`], even when parsing fails.
pub trait SyntaxObserver {
    /// Called when entering a syntax structure, loop iteration, or condition.
    fn enter_scope(&mut self, scope: ScopeKind);

    /// Called when leaving the most recently entered scope.
    fn exit_scope(&mut self);

    /// Called after a syntax element is read from the bitstream.
    fn on_field_read(&mut self, field: SyntaxFieldRead);

    /// Called when a conditional syntax element is absent from the bitstream.
    fn on_field_absent(&mut self, field: SyntaxFieldAbsent);

    /// Called when parsing fails, after all fields read before the error have
    /// been reported.
    fn on_error(&mut self, event: SyntaxErrorRef<'_>);
}

impl<T: SyntaxObserver + ?Sized> SyntaxObserver for &mut T {
    #[inline]
    fn enter_scope(&mut self, scope: ScopeKind) {
        (**self).enter_scope(scope);
    }

    #[inline]
    fn exit_scope(&mut self) {
        (**self).exit_scope();
    }

    #[inline]
    fn on_field_read(&mut self, field: SyntaxFieldRead) {
        (**self).on_field_read(field);
    }

    #[inline]
    fn on_field_absent(&mut self, field: SyntaxFieldAbsent) {
        (**self).on_field_absent(field);
    }

    #[inline]
    fn on_error(&mut self, event: SyntaxErrorRef<'_>) {
        (**self).on_error(event);
    }
}

/// Zero-sized observer that discards all events at zero runtime cost.
///
/// Standard `parse_*` entrypoints monomorphize internal parsers with this type
/// so no observer work survives optimization on the decode path.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopObserver;

impl SyntaxObserver for NoopObserver {
    #[inline(always)]
    fn enter_scope(&mut self, _scope: ScopeKind) {}

    #[inline(always)]
    fn exit_scope(&mut self) {}

    #[inline(always)]
    fn on_field_read(&mut self, _field: SyntaxFieldRead) {}

    #[inline(always)]
    fn on_field_absent(&mut self, _field: SyntaxFieldAbsent) {}

    #[inline(always)]
    fn on_error(&mut self, _event: SyntaxErrorRef<'_>) {}
}

/// Caller-configured bounds on recorded syntax output for hostile input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxLimits {
    /// Maximum number of recorded elements (read fields, absent fields, and
    /// scope entries).
    pub max_elements: usize,
    /// Maximum nesting depth of scopes recorded.
    pub max_depth: usize,
}

impl SyntaxLimits {
    /// Default element limit (`16_384`).
    pub const DEFAULT_MAX_ELEMENTS: usize = 16_384;
    /// Default nesting depth limit (`32`).
    pub const DEFAULT_MAX_DEPTH: usize = 32;

    /// Creates explicit bounds for [`SyntaxRecorder`].
    #[must_use]
    pub const fn new(max_elements: usize, max_depth: usize) -> Self {
        Self {
            max_elements,
            max_depth,
        }
    }
}

impl Default for SyntaxLimits {
    fn default() -> Self {
        Self {
            max_elements: Self::DEFAULT_MAX_ELEMENTS,
            max_depth: Self::DEFAULT_MAX_DEPTH,
        }
    }
}

/// One event in a recorded [`SyntaxTrace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SyntaxEvent {
    /// Entered a nested syntax scope.
    EnterScope(RecordedScope),
    /// Exited the most recently entered recorded scope.
    ExitScope,
    /// A syntax field was read from the bitstream.
    FieldRead(RecordedFieldRead),
    /// A syntax field was absent because of a condition.
    FieldAbsent(RecordedFieldAbsent),
}

/// Bounded, serializable trace collected by [`SyntaxRecorder`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxTrace {
    /// Recorded syntax events in parse order. Scope entries and exits are balanced.
    pub events: Vec<SyntaxEvent>,
    /// First parse error observed, if parsing failed before completion.
    pub error: Option<RecordedSyntaxError>,
    /// `true` when recording stopped early because [`SyntaxLimits::max_elements`]
    /// was reached.
    pub elements_truncated: bool,
    /// `true` when any scope or field deeper than [`SyntaxLimits::max_depth`]
    /// was omitted.
    pub depth_truncated: bool,
    /// Total number of elements observed (including those dropped by limits).
    pub total_observed: usize,
    /// Maximum nesting depth reached during parsing (including unrecorded depth).
    pub max_depth_reached: usize,
}

impl SyntaxTrace {
    /// Returns `true` if either element or depth truncation occurred.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.elements_truncated || self.depth_truncated
    }

    /// Iterates over all recorded fields read from the bitstream.
    pub fn fields_read(&self) -> impl Iterator<Item = &RecordedFieldRead> {
        self.events.iter().filter_map(|event| match event {
            SyntaxEvent::FieldRead(field) => Some(field),
            _ => None,
        })
    }

    /// Iterates over all recorded absent / inferred fields.
    pub fn fields_absent(&self) -> impl Iterator<Item = &RecordedFieldAbsent> {
        self.events.iter().filter_map(|event| match event {
            SyntaxEvent::FieldAbsent(field) => Some(field),
            _ => None,
        })
    }
}

/// Bounded [`SyntaxObserver`] that records events into a [`SyntaxTrace`].
///
/// Enforces [`SyntaxLimits::max_elements`] and [`SyntaxLimits::max_depth`] so
/// hostile or deeply nested inputs cannot cause unbounded allocation. Scope
/// entries and exits in the resulting [`SyntaxTrace`] are always balanced, and
/// any terminal parse error is still captured even if element recording hit its
/// limit.
#[derive(Debug, Clone)]
pub struct SyntaxRecorder {
    limits: SyntaxLimits,
    events: Vec<SyntaxEvent>,
    error: Option<RecordedSyntaxError>,
    recorded_elements: usize,
    total_observed: usize,
    actual_depth: usize,
    recorded_depth: usize,
    max_depth_reached: usize,
    elements_truncated: bool,
    depth_truncated: bool,
    scope_recorded_stack: Vec<bool>,
}

impl SyntaxRecorder {
    /// Creates a recorder with the given element and nesting limits.
    #[must_use]
    pub fn new(limits: SyntaxLimits) -> Self {
        Self {
            limits,
            events: Vec::new(),
            error: None,
            recorded_elements: 0,
            total_observed: 0,
            actual_depth: 0,
            recorded_depth: 0,
            max_depth_reached: 0,
            elements_truncated: false,
            depth_truncated: false,
            scope_recorded_stack: Vec::new(),
        }
    }

    /// Creates a recorder with [`SyntaxLimits::default`].
    #[must_use]
    pub fn with_default_limits() -> Self {
        Self::new(SyntaxLimits::default())
    }

    /// Consumes the recorder and returns the collected [`SyntaxTrace`].
    #[must_use]
    pub fn finish(self) -> SyntaxTrace {
        SyntaxTrace {
            events: self.events,
            error: self.error,
            elements_truncated: self.elements_truncated,
            depth_truncated: self.depth_truncated,
            total_observed: self.total_observed,
            max_depth_reached: self.max_depth_reached,
        }
    }

    /// Returns the number of elements recorded so far (excluding `ExitScope`).
    #[must_use]
    pub const fn recorded_elements(&self) -> usize {
        self.recorded_elements
    }

    fn can_record_element(&mut self) -> bool {
        self.total_observed = self.total_observed.saturating_add(1);
        if self.recorded_elements >= self.limits.max_elements {
            self.elements_truncated = true;
            return false;
        }
        if self.actual_depth > self.limits.max_depth {
            self.depth_truncated = true;
            return false;
        }
        true
    }

    fn record_leaf(&mut self, event: SyntaxEvent) {
        let parent_recorded = self.scope_recorded_stack.last().copied().unwrap_or(true);
        if !parent_recorded {
            self.total_observed = self.total_observed.saturating_add(1);
            if self.actual_depth > self.limits.max_depth {
                self.depth_truncated = true;
            }
            return;
        }
        if self.can_record_element() {
            self.recorded_elements = self.recorded_elements.saturating_add(1);
            self.events.push(event);
        }
    }
}

impl SyntaxObserver for SyntaxRecorder {
    fn enter_scope(&mut self, scope: ScopeKind) {
        self.actual_depth = self.actual_depth.saturating_add(1);
        if self.actual_depth > self.max_depth_reached {
            self.max_depth_reached = self.actual_depth;
        }
        let parent_recorded = self.scope_recorded_stack.last().copied().unwrap_or(true);
        let recorded = if !parent_recorded {
            self.total_observed = self.total_observed.saturating_add(1);
            if self.actual_depth > self.limits.max_depth {
                self.depth_truncated = true;
            }
            false
        } else if self.can_record_element() {
            self.recorded_elements = self.recorded_elements.saturating_add(1);
            self.recorded_depth = self.recorded_depth.saturating_add(1);
            self.events.push(SyntaxEvent::EnterScope(scope.into()));
            true
        } else {
            false
        };
        if self.scope_recorded_stack.len() <= self.limits.max_depth.saturating_add(1) {
            self.scope_recorded_stack.push(recorded);
        }
    }

    fn exit_scope(&mut self) {
        let recorded = if self.actual_depth <= self.scope_recorded_stack.len() {
            self.scope_recorded_stack.pop().unwrap_or(false)
        } else {
            false
        };
        self.actual_depth = self.actual_depth.saturating_sub(1);
        if recorded {
            self.recorded_depth = self.recorded_depth.saturating_sub(1);
            self.events.push(SyntaxEvent::ExitScope);
        }
    }

    fn on_field_read(&mut self, field: SyntaxFieldRead) {
        self.record_leaf(SyntaxEvent::FieldRead(field.into()));
    }

    fn on_field_absent(&mut self, field: SyntaxFieldAbsent) {
        self.record_leaf(SyntaxEvent::FieldAbsent(field.into()));
    }

    fn on_error(&mut self, event: SyntaxErrorRef<'_>) {
        if self.error.is_none() {
            self.error = Some(RecordedSyntaxError {
                bit_offset: event.bit_offset,
                field: event.field.map(Cow::Borrowed),
                message: event.error.to_string(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::ByteOffset;

    #[test]
    fn recorder_enforces_max_elements_and_keeps_scopes_balanced() {
        let mut recorder = SyntaxRecorder::new(SyntaxLimits::new(2, 8));
        recorder.enter_scope(ScopeKind::Structure {
            name: "outer",
            spec_section: Some("5.4"),
        });
        recorder.on_field_read(SyntaxFieldRead {
            name: "a",
            descriptor: SyntaxDescriptor::F(1),
            value: SyntaxValue::Bool(true),
            span: BitSpan::new(0, 1),
        });
        recorder.on_field_read(SyntaxFieldRead {
            name: "b",
            descriptor: SyntaxDescriptor::F(3),
            value: SyntaxValue::Unsigned(5),
            span: BitSpan::new(1, 3),
        });
        recorder.on_error(SyntaxErrorRef {
            bit_offset: 4,
            field: Some("c"),
            error: &Error::UnexpectedEof {
                offset: ByteOffset::new(0),
                needed: 1,
            },
        });
        recorder.exit_scope();

        let trace = recorder.finish();
        assert!(trace.elements_truncated);
        assert!(!trace.depth_truncated);
        assert_eq!(trace.total_observed, 3);
        assert_eq!(trace.fields_read().count(), 1);
        assert_eq!(trace.events.len(), 3);
        assert!(matches!(trace.events[0], SyntaxEvent::EnterScope(_)));
        assert!(matches!(trace.events[1], SyntaxEvent::FieldRead(_)));
        assert!(matches!(trace.events[2], SyntaxEvent::ExitScope));
        let err = trace.error.as_ref().unwrap();
        assert_eq!(err.bit_offset, 4);
        assert_eq!(err.field.as_deref(), Some("c"));
    }

    #[test]
    fn recorder_enforces_max_depth_and_resumes_at_outer_scope() {
        let mut recorder = SyntaxRecorder::new(SyntaxLimits::new(16, 1));
        recorder.enter_scope(ScopeKind::Structure {
            name: "level1",
            spec_section: None,
        });
        recorder.enter_scope(ScopeKind::Structure {
            name: "level2",
            spec_section: None,
        });
        recorder.on_field_read(SyntaxFieldRead {
            name: "deep_field",
            descriptor: SyntaxDescriptor::F(1),
            value: SyntaxValue::Bool(false),
            span: BitSpan::new(0, 1),
        });
        recorder.exit_scope();
        recorder.on_field_read(SyntaxFieldRead {
            name: "outer_field",
            descriptor: SyntaxDescriptor::F(2),
            value: SyntaxValue::Unsigned(3),
            span: BitSpan::new(1, 2),
        });
        recorder.exit_scope();

        let trace = recorder.finish();
        assert!(trace.depth_truncated);
        assert!(!trace.elements_truncated);
        assert_eq!(trace.max_depth_reached, 2);
        let names: Vec<_> = trace.fields_read().map(|f| f.name.as_ref()).collect();
        assert_eq!(names, vec!["outer_field"]);
    }
}
