// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The user-facing worker-thread-count policy ([`ThreadCount`]).
use core::fmt;
use core::num::NonZeroUsize;
use core::str::FromStr;

use crate::error::ThreadCountParseError;

/// How many worker threads a codec context should use.
///
/// `Auto` resolves to the host parallelism once per pool creation (never inside
/// hot loops). `Fixed(n)` requires `n > 0` (enforced by the `NonZeroUsize`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ThreadCount {
    /// Resolve the worker count from the host at pool-creation time.
    #[default]
    Auto,
    /// Use exactly this many worker threads.
    Fixed(NonZeroUsize),
}

impl ThreadCount {
    /// Builds a [`ThreadCount`] from a raw count, mapping `0` to [`ThreadCount::Auto`].
    #[must_use]
    pub fn from_count_or_auto(count: usize) -> Self {
        match NonZeroUsize::new(count) {
            Some(count) => Self::Fixed(count),
            None => Self::Auto,
        }
    }

    /// Resolves to a concrete non-zero worker count.
    ///
    /// `Auto` uses [`std::thread::available_parallelism`], falling back to a
    /// single worker when the host count is unavailable.
    #[must_use]
    pub fn resolve(self) -> NonZeroUsize {
        match self {
            Self::Auto => std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN),
            Self::Fixed(count) => count,
        }
    }
}

impl fmt::Display for ThreadCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Fixed(count) => write!(f, "{count}"),
        }
    }
}

impl FromStr for ThreadCount {
    type Err = ThreadCountParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ThreadCountParseError::Empty);
        }
        if trimmed.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        match trimmed.parse::<usize>() {
            Ok(count) => Ok(Self::from_count_or_auto(count)),
            Err(_) => Err(ThreadCountParseError::Invalid {
                input: trimmed.to_owned(),
            }),
        }
    }
}

impl From<NonZeroUsize> for ThreadCount {
    fn from(count: NonZeroUsize) -> Self {
        Self::Fixed(count)
    }
}

impl From<usize> for ThreadCount {
    fn from(count: usize) -> Self {
        Self::from_count_or_auto(count)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    #[test]
    fn parses_auto_case_insensitively() {
        assert_eq!("auto".parse::<ThreadCount>().unwrap(), ThreadCount::Auto);
        assert_eq!("AUTO".parse::<ThreadCount>().unwrap(), ThreadCount::Auto);
        assert_eq!("Auto".parse::<ThreadCount>().unwrap(), ThreadCount::Auto);
    }

    #[test]
    fn parses_zero_as_auto() {
        assert_eq!("0".parse::<ThreadCount>().unwrap(), ThreadCount::Auto);
    }

    #[test]
    fn parses_positive_integer_as_fixed() {
        assert_eq!(
            "8".parse::<ThreadCount>().unwrap(),
            ThreadCount::Fixed(nz(8))
        );
    }

    #[test]
    fn empty_and_whitespace_are_empty_error() {
        assert_eq!(
            "".parse::<ThreadCount>().unwrap_err(),
            ThreadCountParseError::Empty
        );
        assert_eq!(
            "   ".parse::<ThreadCount>().unwrap_err(),
            ThreadCountParseError::Empty
        );
    }

    #[test]
    fn non_numeric_inputs_are_invalid_error() {
        for input in ["-1", "x", "3.5"] {
            assert_eq!(
                input.parse::<ThreadCount>().unwrap_err(),
                ThreadCountParseError::Invalid {
                    input: input.to_owned(),
                },
                "expected Invalid for {input:?}",
            );
        }
    }

    #[test]
    fn display_renders_auto_and_fixed() {
        assert_eq!(ThreadCount::Auto.to_string(), "auto");
        assert_eq!(ThreadCount::Fixed(nz(4)).to_string(), "4");
    }

    #[test]
    fn resolve_returns_expected_counts() {
        assert_eq!(ThreadCount::Fixed(nz(3)).resolve(), nz(3));
        assert!(ThreadCount::Auto.resolve().get() >= 1);
    }

    #[test]
    fn from_usize_maps_zero_to_auto() {
        assert_eq!(ThreadCount::from(0usize), ThreadCount::Auto);
        assert_eq!(ThreadCount::from(4usize), ThreadCount::Fixed(nz(4)));
    }

    #[test]
    fn default_is_auto() {
        assert_eq!(ThreadCount::default(), ThreadCount::Auto);
    }
}
