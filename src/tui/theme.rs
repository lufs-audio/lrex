//! The three standard themes locked in the cross-tool TUI style study
//! (`lufs-audio/bplate` `references/studies/tui-style-directions/`). Same
//! values as `bplate`'s own `src/tui/theme.rs` -- this is what makes it one
//! family rather than a per-tool palette; kept as a small duplicated module
//! rather than a shared crate dependency since neither tool publishes a
//! library the other could depend on today.
use clap::ValueEnum;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeKind {
    /// LUFS Audio's web identity -- teal/gold/rust, used only as accents.
    Lufs,
    /// Daniel's actual terminal palette. Verified against upstream
    /// catppuccin/{kde,lapce,kitty} theme files, not guessed.
    Catppuccin,
    /// The accessibility-focused standard: no hue anywhere, state is always
    /// a glyph plus a word, never color alone (WCAG 1.4.1).
    Mono,
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub ground: Color,
    pub ink: Color,
    pub dim: Color,
    pub dim2: Color,
    pub accent: Color,
    pub done: Color,
    pub running: Color,
    pub failed: Color,
}

impl Theme {
    pub fn from_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Lufs => Self::lufs(),
            ThemeKind::Catppuccin => Self::catppuccin_mocha(),
            ThemeKind::Mono => Self::mono(),
        }
    }

    pub fn lufs() -> Self {
        Self {
            ground: Color::Rgb(0x0b, 0x0b, 0x0b),
            ink: Color::Rgb(0xEC, 0xE9, 0xE1),
            dim: Color::Rgb(0x8a, 0x8a, 0x84),
            dim2: Color::Rgb(0x5f, 0x5f, 0x5a),
            accent: Color::Rgb(0x78, 0xBE, 0xBA),
            done: Color::Rgb(0x78, 0xBE, 0xBA),
            running: Color::Rgb(0xE7, 0xB2, 0x25),
            failed: Color::Rgb(0xD3, 0x52, 0x33),
        }
    }

    pub fn catppuccin_mocha() -> Self {
        Self {
            ground: Color::Rgb(0x1e, 0x1e, 0x2e),
            ink: Color::Rgb(0xcd, 0xd6, 0xf4),
            dim: Color::Rgb(0xa6, 0xad, 0xc8),
            dim2: Color::Rgb(0x6c, 0x70, 0x86),
            accent: Color::Rgb(0xcb, 0xa6, 0xf7),
            done: Color::Rgb(0xa6, 0xe3, 0xa1),
            running: Color::Rgb(0xf9, 0xe2, 0xaf),
            failed: Color::Rgb(0xf3, 0x8b, 0xa8),
        }
    }

    pub fn mono() -> Self {
        let ink = Color::Rgb(0xEC, 0xE9, 0xE1);
        Self {
            ground: Color::Rgb(0x0b, 0x0b, 0x0b),
            ink,
            dim: Color::Rgb(0x9e, 0x9e, 0x98),
            dim2: Color::Rgb(0x5f, 0x5f, 0x5a),
            accent: ink,
            done: ink,
            running: ink,
            failed: ink,
        }
    }

    pub fn is_mono(&self) -> bool {
        self.done == self.ink && self.running == self.ink && self.failed == self.ink
    }
}
