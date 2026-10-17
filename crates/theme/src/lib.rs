//! See CLAUDE.md for what this crate owns.

/// An sRGB color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn hex(hex: u32) -> Self {
        Self {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        }
    }
}

/// The colors the UI draws with. Every color in `crates/ui` comes from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    /// Sources and article list.
    pub background: Rgb,
    /// Article pane.
    pub lighter_background: Rgb,
    pub foreground: Rgb,
    /// Lines between panes.
    pub border: Rgb,
}

impl Tokens {
    /// Used until `colors.toml` is read (S4.1) and when no Omarchy theme is found (S4.4).
    pub const fn fallback() -> Self {
        Self {
            background: Rgb::hex(0x1a1b26),
            lighter_background: Rgb::hex(0x24283b),
            foreground: Rgb::hex(0xa9b1d6),
            border: Rgb::hex(0x414868),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_splits_channels() {
        assert_eq!(
            Rgb::hex(0x1a1b26),
            Rgb {
                r: 0x1a,
                g: 0x1b,
                b: 0x26
            }
        );
    }
}
