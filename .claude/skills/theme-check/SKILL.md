---
name: theme-check
description: Verify the app under every Omarchy theme, light and dark, and report contrast or layout bugs with screenshots. Use after any UI or theme change, or when asked to check themes.
---

# Theme check

Proves the UI follows the Omarchy theme and stays readable under all of them.

1. List installed themes: `ls /usr/share/omarchy/themes ~/.config/omarchy/themes 2>/dev/null`. Remember the current one from `~/.local/state/omarchy/current/theme.name` so it can be restored.
2. Launch the app once (`cargo run --release &`). For each theme: `omarchy theme set <name>`, wait 1 s, screenshot the window with `grim -g "$(hyprctl activewindow -j | jq -r '.at[0],.at[1],.size[0],.size[1]' | paste -sd ' ' | awk '{print $1","$2" "$3"x"$4}')" docs/themes/<name>.png`.
3. For each screenshot, check by eye and by number:
   - Contrast: read the theme's `colors.toml` and compute WCAG contrast for list text on `background`, article text on `lighter_background`, and muted (read) text on `background`. Under 4.5 for body text or 3.0 for muted is a bug.
   - Layout: nothing clipped, no pane narrower than its minimum, selection visible, links distinguishable from body text without color alone (underline).
   - Light themes: `mode = "light"` must not show dark panes.
4. Restore the original theme: `omarchy theme set <saved>`.
5. Write `docs/themes/README.md`: a table of theme, mode, pass or fail, and the failing check. Keep the screenshots.
6. Each failure becomes a story in `docs/BACKLOG.md` under E4 with status `todo`, or a fix in the current PR if the user is mid-story.

Don't fix bugs here unless asked; report them. Never leave the user on a different theme than they started with.
