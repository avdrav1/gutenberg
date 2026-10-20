# Changelog

## Unreleased

## 0.1.0 — 2026-10-20

- Running `gutenberg` opens the main window with its three panes. Press `q` to quit.
- Pane widths can be set in `~/.config/gutenberg/config.toml` under `[layout]` (`sources`, `list`, `article`; 0 hides a pane). Edits apply while the app is open.
- Add a feed from the terminal with `gutenberg --add <feed-url>`. RSS, Atom and JSON Feed all work; a bad feed or address prints one line saying what went wrong.
- Install with one line: `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh | sh`. The script checks the download against the release checksums and installs to `~/.local/bin`.
