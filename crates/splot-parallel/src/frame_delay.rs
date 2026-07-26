// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The user-facing frame-pipelining-depth policy ([`FrameDelay`]).
use core::fmt;
use core::num::NonZeroUsize;
use core::str::FromStr;

use crate::error::FrameDelayParseError;

/// How many frames a pipelined decoder may keep in flight at once.
///
/// `Auto` resolves to the pool's worker-thread count once per pipeline setup
/// (never inside hot loops). `Fixed(n)` requires `n > 0` (enforced by the
/// `NonZeroUsize`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FrameDelay {
    /// Resolve the pipelining depth from the worker-thread count.
    #[default]
    Auto,
    /// Keep at most this many frames in flight.
    Fixed(NonZeroUsize),
}

impl FrameDelay {
    /// Builds a [`FrameDelay`] from a raw count, mapping `0` to [`FrameDelay::Auto`].
    #[must_use]
    pub fn from_count_or_auto(count: usize) -> Self {
        match NonZeroUsize::new(count) {
            Some(count) => Self::Fixed(count),
            None => Self::Auto,
        }
    }

    /// Resolves to a concrete non-zero pipelining depth for a pool of `threads`
    /// workers.
    ///
    /// The depth is clamped to the worker-thread count: more in-flight frames
    /// than workers cannot buy additional concurrency, and the bound is what
    /// keeps a blocking pipeline driver from outrunning the pool. A resolved
    /// depth of 1 means serial decode.
    #[must_use]
    pub fn resolve(self, threads: NonZeroUsize) -> NonZeroUsize {
        match self {
            Self::Auto => threads,
            Self::Fixed(depth) => depth.min(threads),
        }
    }
}

impl fmt::Display for FrameDelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Fixed(depth) => write!(f, "{depth}"),
        }
    }
}

impl FromStr for FrameDelay {
    type Err = FrameDelayParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(FrameDelayParseError::Empty);
        }
        if trimmed.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }
        match trimmed.parse::<usize>() {
            Ok(depth) => Ok(Self::from_count_or_auto(depth)),
            Err(_) => Err(FrameDelayParseError::Invalid {
                input: trimmed.to_owned(),
            }),
        }
    }
}

impl From<NonZeroUsize> for FrameDelay {
    fn from(depth: NonZeroUsize) -> Self {
        Self::Fixed(depth)
    }
}

impl From<usize> for FrameDelay {
    fn from(depth: usize) -> Self {
        Self::from_count_or_auto(depth)
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
    fn parse_table_covers_auto_zero_and_positive_depths() {
        for input in ["auto", "AUTO", "Auto", "0", "  auto  "] {
            assert_eq!(
                input.parse::<FrameDelay>().unwrap(),
                FrameDelay::Auto,
                "expected Auto for {input:?}",
            );
        }
        assert_eq!("1".parse::<FrameDelay>().unwrap(), FrameDelay::Fixed(nz(1)));
        assert_eq!("7".parse::<FrameDelay>().unwrap(), FrameDelay::Fixed(nz(7)));
    }

    #[test]
    fn empty_and_whitespace_are_empty_error() {
        assert_eq!(
            "".parse::<FrameDelay>().unwrap_err(),
            FrameDelayParseError::Empty
        );
        assert_eq!(
            "   ".parse::<FrameDelay>().unwrap_err(),
            FrameDelayParseError::Empty
        );
    }

    #[test]
    fn non_numeric_inputs_are_invalid_error() {
        for input in ["-1", "x", "3.5"] {
            assert_eq!(
                input.parse::<FrameDelay>().unwrap_err(),
                FrameDelayParseError::Invalid {
                    input: input.to_owned(),
                },
                "expected Invalid for {input:?}",
            );
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        for delay in [FrameDelay::Auto, FrameDelay::Fixed(nz(3))] {
            assert_eq!(delay.to_string().parse::<FrameDelay>().unwrap(), delay);
        }
        assert_eq!(FrameDelay::Auto.to_string(), "auto");
        assert_eq!(FrameDelay::Fixed(nz(4)).to_string(), "4");
    }

    #[test]
    fn resolve_clamps_to_the_worker_count() {
        for (delay, threads, expected) in [
            (FrameDelay::Auto, 10, 10),
            (FrameDelay::Fixed(nz(4)), 10, 4),
            (FrameDelay::Fixed(nz(64)), 10, 10),
            (FrameDelay::Fixed(nz(1)), 1, 1),
        ] {
            assert_eq!(
                delay.resolve(nz(threads)),
                nz(expected),
                "expected {expected} for {delay} at {threads} worker(s)",
            );
        }
    }

    #[test]
    fn from_usize_maps_zero_to_auto() {
        assert_eq!(FrameDelay::from_count_or_auto(0), FrameDelay::Auto);
        assert_eq!(FrameDelay::from(0usize), FrameDelay::Auto);
        assert_eq!(FrameDelay::from(4usize), FrameDelay::Fixed(nz(4)));
        assert_eq!(FrameDelay::from(nz(4)), FrameDelay::Fixed(nz(4)));
    }

    #[test]
    fn default_is_auto() {
        assert_eq!(FrameDelay::default(), FrameDelay::Auto);
    }
}
