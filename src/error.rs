//! Exit-code taxonomy and the typed error that carries it.
//!
//! The taxonomy is the machine-facing control-flow signal (see `CONTRACT.md` and
//! `AGENTS.md`): a supervising agent branches on the exit code. Every command
//! path funnels through [`ExitError`] so the mapping to an exit code lives in
//! exactly one place.

use std::fmt;

/// Exit-code taxonomy (see `CONTRACT.md`). Stable across versions.
pub mod code {
    /// Take captured AND verified.
    pub const OK: u8 = 0;
    /// Requested audio device or MIDI port unavailable.
    pub const DEVICE_UNAVAILABLE: u8 = 3;
    /// Requested format (rate / bit-depth / channels) unsupported by the device.
    pub const FORMAT_UNSUPPORTED: u8 = 4;
    /// Capture ran but FAILED the verification contract (xrun, truncation, ...).
    pub const CONTRACT_VIOLATION: u8 = 5;
    /// Interrupted before a valid take was produced.
    pub const INTERRUPTED: u8 = 6;
    /// Not implemented yet — the honest-failure sentinel. Used when a
    /// requested capability exists in the surface but its backend does not
    /// (e.g. `--app` isolated capture without a PipeWire-native capture path).
    pub const NOT_IMPLEMENTED: u8 = 70;
}

/// A command failure that knows its own exit code.
#[derive(Debug)]
pub enum ExitError {
    /// Requested audio device or MIDI port unavailable (exit 3).
    DeviceUnavailable(String),
    /// Requested rate / bit-depth / channels unsupported by the device (exit 4).
    FormatUnsupported(String),
    /// Capture ran but failed the verification contract (exit 5).
    ContractViolation(String),
    /// Interrupted before a valid take (exit 6).
    Interrupted(String),
    /// A requested capability is not implemented in this build/environment
    /// (exit 70) — the honest-failure sentinel. Nothing reports success it has
    /// not earned.
    NotImplemented(String),
    /// Any other error — bad usage falls out as a generic failure (exit 2 via clap,
    /// or 1 here). Carries an [`anyhow::Error`] for context.
    Other(anyhow::Error),
}

impl ExitError {
    /// The stable exit code for this failure class.
    pub fn code(&self) -> u8 {
        match self {
            ExitError::DeviceUnavailable(_) => code::DEVICE_UNAVAILABLE,
            ExitError::FormatUnsupported(_) => code::FORMAT_UNSUPPORTED,
            ExitError::ContractViolation(_) => code::CONTRACT_VIOLATION,
            ExitError::Interrupted(_) => code::INTERRUPTED,
            ExitError::NotImplemented(_) => code::NOT_IMPLEMENTED,
            ExitError::Other(_) => 1,
        }
    }

    /// Machine-readable error kind for `--json` reporting.
    pub fn kind(&self) -> &'static str {
        match self {
            ExitError::DeviceUnavailable(_) => "device_unavailable",
            ExitError::FormatUnsupported(_) => "format_unsupported",
            ExitError::ContractViolation(_) => "contract_violation",
            ExitError::Interrupted(_) => "interrupted",
            ExitError::NotImplemented(_) => "not_implemented",
            ExitError::Other(_) => "error",
        }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExitError::DeviceUnavailable(m)
            | ExitError::FormatUnsupported(m)
            | ExitError::ContractViolation(m)
            | ExitError::Interrupted(m)
            | ExitError::NotImplemented(m) => write!(f, "{m}"),
            ExitError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for ExitError {}

impl From<anyhow::Error> for ExitError {
    fn from(e: anyhow::Error) -> Self {
        ExitError::Other(e)
    }
}

/// Convenience alias for command handlers.
pub type Result<T> = std::result::Result<T, ExitError>;
