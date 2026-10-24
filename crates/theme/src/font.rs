//! The desktop's font. Omarchy's `omarchy-font-set` writes a `monospace`
//! alias into fontconfig, and every app on the desktop resolves that alias;
//! so does this.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What fontconfig gives for `monospace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    /// Such as "JetBrainsMono Nerd Font".
    pub family: String,
    /// The file of its regular face, so a font installed after the app
    /// started can still be loaded.
    pub file: Option<PathBuf>,
}

/// `None` without fontconfig or when it names nothing.
pub fn monospace() -> Option<Face> {
    let output = Command::new("fc-match")
        .args(["--format=%{family[0]}\n%{file}", "monospace"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    face(&output.stdout)
}

/// The family and file in `fc-match`'s output, one per line, trimmed.
fn face(output: &[u8]) -> Option<Face> {
    let text = std::str::from_utf8(output).ok()?;
    let mut lines = text.lines().map(str::trim);
    let family = lines.next().filter(|f| !f.is_empty())?.to_owned();
    let file = lines.next().filter(|f| !f.is_empty()).map(PathBuf::from);
    Some(Face { family, file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_is_family_then_file() {
        assert_eq!(
            face(
                b"JetBrainsMono Nerd Font\n/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf"
            ),
            Some(Face {
                family: "JetBrainsMono Nerd Font".into(),
                file: Some("/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf".into()),
            })
        );
        assert_eq!(
            face(b" Iosevka \n"),
            Some(Face {
                family: "Iosevka".into(),
                file: None
            })
        );
        assert_eq!(face(b"  \n/x.ttf"), None);
        assert_eq!(face(b"\xff\xfe"), None);
    }
}
