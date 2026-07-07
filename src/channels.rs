//! Channel-spec parsing.
//!
//! Turns a human/agent-supplied `--channels` string like `"1"`, `"1,2"`, or
//! `"1-2,9-10"` into a sorted, de-duplicated set of **1-based** channel numbers.
//!
//! This module is pure `std` — no audio backend — so it is fully unit-testable
//! without any hardware, and it type-checks in a toolchain-only environment. The
//! *syntactic* validation lives here; validating the requested channels against a
//! specific device's real channel count happens later, once cpal can enumerate
//! the device (v0.2).

use std::error::Error;
use std::fmt;

/// Why a `--channels` spec could not be parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum ChannelSpecError {
    /// The spec was empty or all-whitespace.
    Empty,
    /// A token was not a positive integer (or a `A-B` range of them).
    NotANumber(String),
    /// Channels are 1-based; `0` is never valid.
    ZeroChannel,
    /// A range `A-B` had `A > B`.
    ReversedRange { start: u16, end: u16 },
}

impl fmt::Display for ChannelSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChannelSpecError::Empty => write!(f, "channel spec is empty"),
            ChannelSpecError::NotANumber(tok) => {
                write!(
                    f,
                    "invalid channel token '{tok}' (expected N or A-B, 1-based)"
                )
            }
            ChannelSpecError::ZeroChannel => {
                write!(f, "channel numbers are 1-based; 0 is not valid")
            }
            ChannelSpecError::ReversedRange { start, end } => {
                write!(f, "range {start}-{end} is reversed (start must be <= end)")
            }
        }
    }
}

impl Error for ChannelSpecError {}

/// Parse a channel spec into a sorted, unique vector of 1-based channel numbers.
///
/// Accepts comma-separated tokens, each either a single channel (`"3"`) or an
/// inclusive range (`"9-10"`). Surrounding whitespace is ignored. The result is
/// sorted ascending with duplicates removed, so `"2,1,1,2-3"` yields `[1, 2, 3]`.
pub fn parse_channel_spec(spec: &str) -> Result<Vec<u16>, ChannelSpecError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(ChannelSpecError::Empty);
    }

    let mut channels: Vec<u16> = Vec::new();
    for raw in trimmed.split(',') {
        let token = raw.trim();
        match token.split_once('-') {
            Some((start, end)) => {
                let start = parse_one(start)?;
                let end = parse_one(end)?;
                if start > end {
                    return Err(ChannelSpecError::ReversedRange { start, end });
                }
                channels.extend(start..=end);
            }
            None => channels.push(parse_one(token)?),
        }
    }

    channels.sort_unstable();
    channels.dedup();
    Ok(channels)
}

/// Parse a single 1-based channel number.
fn parse_one(s: &str) -> Result<u16, ChannelSpecError> {
    let s = s.trim();
    let n: u16 = s
        .parse()
        .map_err(|_| ChannelSpecError::NotANumber(s.to_string()))?;
    if n == 0 {
        return Err(ChannelSpecError::ZeroChannel);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_channel() {
        assert_eq!(parse_channel_spec("3"), Ok(vec![3]));
    }

    #[test]
    fn comma_list() {
        assert_eq!(parse_channel_spec("1,2"), Ok(vec![1, 2]));
    }

    #[test]
    fn ranges_and_lists_combine() {
        assert_eq!(parse_channel_spec("1-2,9-10"), Ok(vec![1, 2, 9, 10]));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_channel_spec("  1 , 2 "), Ok(vec![1, 2]));
    }

    #[test]
    fn sorts_and_dedups() {
        assert_eq!(parse_channel_spec("2,1,1,2-3"), Ok(vec![1, 2, 3]));
    }

    #[test]
    fn single_element_range() {
        assert_eq!(parse_channel_spec("5-5"), Ok(vec![5]));
    }

    #[test]
    fn empty_is_error() {
        assert_eq!(parse_channel_spec("   "), Err(ChannelSpecError::Empty));
    }

    #[test]
    fn zero_is_error() {
        assert_eq!(parse_channel_spec("0"), Err(ChannelSpecError::ZeroChannel));
        assert_eq!(
            parse_channel_spec("1-0"),
            Err(ChannelSpecError::ZeroChannel)
        );
    }

    #[test]
    fn reversed_range_is_error() {
        assert_eq!(
            parse_channel_spec("5-3"),
            Err(ChannelSpecError::ReversedRange { start: 5, end: 3 })
        );
    }

    #[test]
    fn non_numeric_is_error() {
        assert!(matches!(
            parse_channel_spec("x"),
            Err(ChannelSpecError::NotANumber(_))
        ));
        assert!(matches!(
            parse_channel_spec("1,,2"),
            Err(ChannelSpecError::NotANumber(_))
        ));
    }
}
