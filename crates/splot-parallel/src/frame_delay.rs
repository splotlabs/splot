// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The user-facing frame-pipelining-depth policy ([`FrameDelay`]).
use core::fmt;
use core::num::NonZeroUsize;
use core::str::FromStr;

use crate::error::{FrameDelayParseError, parse_auto_or_count};

/// How many frames a pipelined decoder may keep in flight at once.
///
/// `Auto` resolves once per pipeline setup (never inside hot loops) to the
/// worker-thread count. `Fixed(n)` requires `n > 0` (enforced by the
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
    /// `Auto` uses the worker count. `Fixed(n)` preserves the requested policy;
    /// a consumer may cap its effective in-flight work at the worker count
    /// because a larger queue cannot add execution concurrency. A resolved
    /// depth of 1 means one admitted frame.
    #[must_use]
    pub fn resolve(self, threads: NonZeroUsize) -> NonZeroUsize {
        match self {
            Self::Auto => threads,
            Self::Fixed(depth) => depth,
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
        Ok(Self::from_count_or_auto(parse_auto_or_count(s)?))
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
    fn parse_error_table_covers_empty_and_non_numeric_inputs() {
        let invalid = |input: &str| FrameDelayParseError::Invalid {
            input: input.to_owned(),
        };
        let cases = [
            ("", FrameDelayParseError::Empty),
            ("   ", FrameDelayParseError::Empty),
            ("-1", invalid("-1")),
            ("x", invalid("x")),
            ("3.5", invalid("3.5")),
        ];
        for (input, expected) in cases {
            assert_eq!(input.parse::<FrameDelay>().unwrap_err(), expected);
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
    fn resolve_maps_auto_to_worker_count_and_honors_fixed_depths() {
        for (delay, threads, expected) in [
            (FrameDelay::Auto, 10, 10),
            (FrameDelay::Auto, 4, 4),
            (FrameDelay::Auto, 3, 3),
            (FrameDelay::Auto, 2, 2),
            (FrameDelay::Auto, 1, 1),
            (FrameDelay::Fixed(nz(4)), 10, 4),
            (FrameDelay::Fixed(nz(64)), 10, 64),
            (FrameDelay::Fixed(nz(3)), 2, 3),
            (FrameDelay::Fixed(nz(1)), 2, 1),
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
