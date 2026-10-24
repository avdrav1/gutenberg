//! The desktop's font. Omarchy's `omarchy-font-set` writes a `monospace`
//! alias into fontconfig, and every app on the desktop resolves that alias;
//! so does this.

use std::process::{Command, Stdio};

/// The family fontconfig gives for `monospace`, such as "JetBrainsMono Nerd
/// Font". `None` without fontconfig or when it names nothing.
pub fn monospace() -> Option<String> {
    let output = Command::new("fc-match")
        .args(["--format=%{family[0]}", "monospace"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    family(&output.stdout)
}

/// The family in `fc-match`'s output, trimmed; `None` if there is none.
fn family(output: &[u8]) -> Option<String> {
    let name = std::str::from_utf8(output).ok()?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_is_the_trimmed_output_or_none() {
        assert_eq!(
            family(b"JetBrainsMono Nerd Font\n").as_deref(),
            Some("JetBrainsMono Nerd Font")
        );
        assert_eq!(family(b"  \n"), None);
        assert_eq!(family(b"\xff\xfe"), None);
    }
}
