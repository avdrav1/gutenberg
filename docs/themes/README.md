# Themes

The window in every stock Omarchy theme (omarchy 4.0), rendered headlessly from `tests/fixtures/themes/<name>.toml`:

```sh
ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots \
    --example theme_shots -- tests/fixtures/themes docs/themes
```

To render what's installed instead, pass `/usr/share/omarchy/themes` as the first argument. Text is in the rendering machine's desktop font, as in the app (`fc-match monospace`; JetBrainsMono Nerd Font here).

## The checks

- **Text** is the foreground. It must reach 4.5:1 against the list background, the article pane and the selected row.
- **Muted** is used for read titles and meta lines, and **links** for underlined links and stars. Each must reach 3:1 against the same three backgrounds.
- Each figure below is the lowest of the three.

**Where the app adjusts a theme.** When a theme's own color falls short, the app moves it toward white (dark themes) or black (light themes). It stops as soon as the color reaches the ratio. Backgrounds, selection and borders are never adjusted. In the older layout, the article pane and the selected row are derived from the theme's background, foreground and accent.

**Muted.** Most themes' `dark_foreground` is meant for terminal comments, not text you read. In 20 of the 24 themes it falls short of 3:1, at anywhere from 1.0:1 to 2.99:1. In last-horizon and white it is the same color as one of the backgrounds. So muted is the color the app adjusts most often.

## Results

| Theme | Mode | Text | Muted | Links | Adjusted | Result |
|---|---|---|---|---|---|---|
| catppuccin | dark | 6.31 | 3.07 | 4.33 | muted | pass |
| catppuccin-latte | light | 5.17 | 3.06 | 3.18 | muted | pass |
| ethereal | dark | 9.16 | 3.28 | 3.78 | | pass |
| everforest | dark | 5.57 | 3.06 | 4.33 | muted | pass |
| flexoki-light | light | 11.98 | 3.00 | 4.09 | muted | pass |
| gruvbox | dark | 4.88 | 3.08 | 3.55 | muted | pass |
| hackerman | dark | 13.60 | 3.09 | 11.73 | | pass |
| kanagawa | dark | 8.16 | 3.07 | 8.16 | muted | pass |
| last-horizon | dark | 7.77 | 3.08 | 3.03 | muted, links | pass |
| lumon | dark | 8.51 | 3.05 | 6.21 | muted | pass |
| lupine | light | 10.44 | 3.07 | 3.27 | muted | pass |
| matte-black | dark | 7.72 | 3.07 | 5.63 | muted | pass |
| miasma | dark | 6.50 | 3.01 | 3.05 | muted, links | pass |
| nord | dark | 6.39 | 3.08 | 3.21 | muted | pass |
| osaka-jade | dark | 5.53 | 4.45 | 3.03 | links | pass |
| retro-82 | dark | 6.95 | 3.03 | 4.83 | muted | pass |
| ristretto | dark | 7.72 | 3.01 | 4.46 | muted | pass |
| rose-pine | light | 5.25 | 3.06 | 3.11 | muted, links | pass |
| solitude | dark | 6.89 | 3.09 | 3.00 | muted, links | pass |
| starship | dark | 10.42 | 3.95 | 5.52 | | pass |
| tokyo-night | dark | 6.36 | 3.14 | 5.33 | muted | pass |
| vantablack | dark | 17.40 | 3.17 | 5.24 | muted | pass |
| white | light | 11.54 | 3.06 | 3.11 | muted, links | pass |
| terminal-style-solitude | — | 8.40 | 3.07 | 3.42 | muted | pass |

`terminal-style-solitude` is a theme in the layout Omarchy used before 4.0, with terminal keys `color0` to `color15` and no `mode`. A theme set before the update keeps that layout until it is set again.

## Layout

All 24 screenshots were checked by eye:

- nothing is clipped;
- the selected row and the chosen folder are visible;
- links are underlined;
- the five light themes show light panes throughout.

`mode` itself isn't read yet (S4.5). The panes come out light because the theme's own backgrounds are light.
