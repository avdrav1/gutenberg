---
name: theme-check
description: Verify the app under every Omarchy theme, light and dark, and report contrast or layout bugs with screenshots. Use after any UI or theme change, or when asked to check themes.
---

# Theme check

Proves the UI follows the Omarchy theme and stays readable under all of them.

1. List installed themes: `ls /usr/share/omarchy/themes ~/.config/omarchy/themes 2>/dev/null`. Refresh `tests/fixtures/themes/<name>.toml` from any `colors.toml` that changed or is new, and note the omarchy version in that folder's README.
2. Render every theme without touching the desktop: `ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots --example theme_shots -- tests/fixtures/themes docs/themes`. It writes `docs/themes/<name>.png` and prints each theme's lowest contrast ratios after mapping. Halve the PNGs for git: `magick <f> -resize 50% -strip <f>`.
3. For each screenshot, check by eye and by number:
   - Contrast: text 4.5 or more, muted and links 3.0 or more, against the list, article pane and selected row (the printed figures; `crates/theme/tests/stock.rs` asserts them).
   - Layout: nothing clipped, no pane narrower than its minimum, selection visible, links distinguishable from body text without color alone (underline).
   - Light themes: `mode = "light"` must not show dark panes.
4. Write `docs/themes/README.md`: a table of theme, mode, the three ratios, which colors the app adjusted, and pass or fail with the failing check. Keep the screenshots.
5. Each failure becomes a story in `docs/BACKLOG.md` under E4 with status `todo`, or a fix in the current PR if the user is mid-story.

Don't fix bugs here unless asked; report them. Don't change the user's desktop theme: the renders don't need it.
