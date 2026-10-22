# Gutenberg PRD

Oct 17, 2026 · @Avniel Dravid

## Summary

**Gutenberg** is a native Linux news reader for Omarchy, written in Rust. It pulls stories from RSS and Atom feeds, from MCP servers, and from keyword alerts, and shows them in one keyboard-driven window that takes its colors and font from the active Omarchy theme. It starts in under 250 ms, works offline, and keeps everything in one SQLite file.

It is for Avniel first, then Omarchy users who want a reader that looks like the rest of their desktop and never phones home. Linux ships first. macOS and Windows follow once the Linux build is stable, since the Rust core and the UI toolkit are cross-platform.

In scope for 1.0: feeds, keyword alerts, MCP sources, a reader view, search, OPML import and export, and theme sync. Out of scope for 1.0: accounts, cloud sync, social features, and an AI summarizer (Cronkite's job).

## Problem and goals

Omarchy has no news reader that fits the desktop. The options are a browser tab (Feedly, Inoreader), a GTK app that ignores the theme (Liferea, NewsFlash), or a terminal reader with no images or reader view (newsboat). None of them takes sources from MCP servers, and keyword alerts still mean Google Alerts email.

Goals, in priority order:

1. **Belongs on the desktop.** Colors, font and spacing follow the Omarchy theme and change the moment the theme does. Keys follow Vim and Hyprland habits.
2. **Fast.** Cold start under 250 ms, 10,000 articles scroll at 60 fps, a refresh of 100 feeds finishes in under 10 s on a home connection.
3. **One inbox for three kinds of source.** Feeds, MCP tools and keyword alerts all land in the same list with the same keys.
4. **Private and local.** No account, no telemetry, one SQLite file the user can copy.

Success at 1.0: Avniel uses it daily instead of a browser, three Omarchy users install it from the site, and no issue is open about theme mismatch.

Non-goals for 1.0: AI summaries and ranking (Cronkite covers that; a later MCP bridge can connect the two), podcast playback, full web browsing inside the app.

## Users and use cases

| User | What they do | What they need |
| --- | --- | --- |
| Avniel, daily reader | Opens the app with a Hyprland keybind, triages 200 new items in 10 minutes, reads 10 | j/k, unread counts, a reader view that strips the page down, star and share |
| Omarchy tinkerer | Switches themes often, installs with a one-line `curl`, edits TOML by hand | Theme sync with no restart, a config file in `~/.config`, no GUI settings maze |
| Politics-tech watcher | Follows a few names and bills across outlets | Keyword alerts that dedupe across sources, grouped by story |
| MCP builder | Runs local MCP servers (Cronkite, a scraper, a Slack digest) | Add a server by command or URL, pick a tool, see its results as articles |
| Future macOS user | Same app on a MacBook | Cross-platform core; native look is a later concern |

## Requirements

### Sources

| Kind | Must | Notes |
| --- | --- | --- |
| RSS / Atom / JSON Feed | Add by URL or site URL (autodiscovery), folders, per-feed refresh interval, conditional GET (ETag, Last-Modified), favicon, OPML import and export | Parse with `feed-rs`. Treat a 429 or 5xx as backoff, never as an error the user must dismiss |
| Keyword alerts | A query string becomes a source. Results from Google News RSS search, Bing News RSS, and GDELT's doc API, merged and deduped by canonical URL and title similarity | The user sees one source per query, not three. Quoted phrases, `-exclude`, `site:` pass through |
| MCP servers | Add a server by command (stdio) or URL (streamable HTTP). List its tools. Pick a tool and a JSON argument template; each call's result is mapped to articles | Use `rmcp`, the official Rust SDK. A tool result is accepted as articles if it returns an array of objects with `title` and `url`, or a feed document, or Markdown with links; otherwise show it as one article |

### Reading

- Three panes: sources, list, article. Any pane can be hidden; the list alone is a valid layout for a narrow tile.
- Reader view: fetch the page, extract the body (a Rust port of Readability or `dom_smoothie`), render headings, paragraphs, images, code and blockquotes in the theme's font. Fall back to the feed's summary, then to "open in browser."
- Keys: `j`/`k` move, `o`/`Enter` open, `m` toggle read, `s` star, `/` search, `r` refresh, `gg`/`G`, `:` command palette, `?` help. Every action is reachable from the palette.
- Full-text search over title, summary and extracted body (SQLite FTS5). Results in under 50 ms for 100k articles.
- Mark read on scroll past, mark all read in a folder, unread-only filter, starred, and a "today" view.
- Open in the default browser (`xdg-open`), copy link, share via `wl-copy`.

### Theming and desktop

- Read the palette from Omarchy's `colors.toml` and the font from fontconfig. Apply on launch and within one second of a theme change, with no restart.
- Match the terminal: no title bar decoration under Hyprland, borders and radii from the theme, the same spacing scale as `omarchy-shell`.
- Respect light and dark modes from the theme's `mode` key.
- Desktop file, icon, and a Walker entry. Unread count exposed for Waybar via a small CLI (`gutenberg --unread`).
- Outside Omarchy: fall back to a built-in theme and the system monospace font.

### Performance and reliability

- Cold start under 250 ms to first paint with 10k articles in the store. (Was 200 ms; raised in S3.1 when 60 fps at 4K needed the GPU, whose setup is ~170 ms.)
- Refresh runs in the background on a tokio runtime; the UI never blocks.
- Storage: one SQLite file at `$XDG_DATA_HOME/gutenberg/gutenberg.db`, WAL mode, schema migrations on launch.
- Memory under 150 MB with 10k articles loaded and images cached to disk, not RAM.
- Config at `$XDG_CONFIG_HOME/gutenberg/config.toml`, hot-reloaded.

### Privacy

- No telemetry. No network calls except to the user's sources, favicons, and article pages for reader view. A `--offline` flag disables all of it.

## Architecture

(Architecture diagram: see the PRD doc on claude.ai.)

A Cargo workspace of six crates. Sources implement one trait (`fetch() -> Vec<Article>`), so a feed, a keyword query and an MCP tool look the same to the sync engine. The engine writes to SQLite; the UI subscribes to the store and redraws. Nothing in the UI awaits a socket.

| Choice | Pick | Why, and the alternative |
| --- | --- | --- |
| UI toolkit | iced 0.14 | Elm-style, rendered with wgpu on Vulkan (tiny-skia as the no-GPU fallback; it managed ~10 fps scrolling at 4K, see S3.1), runs on Wayland through winit; System76 builds the COSMIC desktop on it, so Wayland bugs get fixed. Alternative: gpui (Zed), faster text and lists but a moving API. Decide after the E1 spike |
| Async | tokio | Sync engine, HTTP, MCP transports all run on it; the UI thread stays free |
| HTTP | reqwest with rustls | Conditional GET, gzip, per-host semaphores |
| Feed parsing | feed-rs | Handles RSS 0.9 through 2.0, Atom and JSON Feed in one type |
| Reader view | dom\_smoothie or a Readability port | Pure Rust; no WebKit dependency |
| MCP | rmcp 3.x | The official SDK; client over stdio (child process) and streamable HTTP |
| Store | rusqlite with FTS5, WAL | One file, fast search, easy backup |
| Config | TOML with serde, watched by notify | Same habits as the rest of Omarchy |
| Theme | toml + fontconfig via `fc-match` | Read, don't template; apply tokens at runtime |

## Omarchy integration

Omarchy keeps the palette in one file and tells apps when it changes. The app reads that file and listens for the signal.

| What | Where | How the app uses it |
| --- | --- | --- |
| Palette | `colors.toml` in the current theme directory: `~/.local/state/omarchy/current/theme/` on current builds, `~/.config/omarchy/current/theme/` on older ones. Keys include `mode`, `accent`, `selection`, `muted`, four backgrounds, four foregrounds, and the standard and bright colors | Map keys to the app's tokens: `background` to the list, `lighter_background` to the article pane, `accent` to selection and links, `muted` to read items, `red`/`green` to error and success |
| Theme name | `~/.local/state/omarchy/current/theme.name` | Shown in `--version` and in bug reports |
| Change signal | `omarchy-theme-set` runs `omarchy-hook theme-set <name>`, which runs `~/.config/omarchy/hooks/theme-set` and every script in `theme-set.d/` | Install a drop-in `theme-set.d/gutenberg` that sends `gutenberg --reload-theme` (a Unix socket message). Also watch the theme directory with inotify as a fallback, since the hook may not be installed |
| Font | fontconfig is the canonical source; `omarchy-font-set` writes a `monospace` rule to `~/.config/fontconfig/fonts.conf` and runs the `font-set` hook | Resolve with `fc-match monospace` at launch and on the `font-set` hook. Default is CaskaydiaMono Nerd Font, so Nerd Font glyphs are safe for icons |
| Window chrome | Hyprland draws borders; `omarchy-shell` uses `shell.toml` for its own surfaces | No client-side decorations. Use the theme's border radius and the shell's spacing scale so the app matches Walker and the notifications |
| Install | A page on GitHub Pages with `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh \| sh`; binaries from GitHub Releases | `install.sh` picks the x86_64 or aarch64 tarball, checks it against `SHA256SUMS`, installs to `~/.local/bin`. CI attaches a build attestation (`gh attestation verify`). A `.desktop` file so Walker lists it comes in S7.1. No AUR package |
| Keybind | Users bind apps in `~/.config/hypr/bindings.conf` | Document `bindd = SUPER, N, News, exec, gutenberg` and offer to add it on first run |

Fact check before building: the theme path moved between Omarchy 2 and 3. Read `bin/omarchy-theme-set` in the installed version and check both paths at runtime.

## Backlog

Epics in build order. Each story is one PR, sized for a day or less. "Done" means merged, tested, and visible in the app. The same list lives in `docs/BACKLOG.md` in the repo, where the `next-story` skill reads it.

### E1 Skeleton (first week)

| ID | Story | Acceptance |
| --- | --- | --- |
| S1.1 | Cargo workspace with `core`, `store`, `sources`, `theme`, `ui`, `cli` crates | `cargo build` and `cargo test` pass in CI on Arch and Ubuntu |
| S1.2 | iced window opens under Hyprland with three empty panes and `q` quits | Window has no CSD; Hyprland tiles it; cold start under 100 ms (met on tiny-skia; superseded by the 250 ms budget when S3.1 moved to wgpu) |
| S1.3 | SQLite store with migrations and `Article`, `Source`, `Folder` tables | Migration runs on a fresh and an existing file; a test inserts 10k rows and lists them in under 20 ms |
| S1.4 | Config file loads with defaults and hot-reloads on change | Edit `config.toml`; the change applies within 1 s with no restart |

### E2 Feeds

| ID | Story | Acceptance |
| --- | --- | --- |
| S2.1 | Add a feed by URL; parse RSS, Atom and JSON Feed | 20 fixture feeds from `tests/fixtures/` parse; bad XML shows one line of error |
| S2.2 | Autodiscover a feed from a site URL | Pasting `https://example.com` finds `<link rel="alternate">`; falls back to `/feed`, `/rss`, `/atom.xml` |
| S2.3 | Background refresh with conditional GET and per-host rate limits | A second refresh of an unchanged feed sends `If-None-Match` and gets 304; 100 feeds finish in under 10 s |
| S2.4 | Folders, drag to reorder, unread counts per source and folder | Counts update as items are read; order persists |
| S2.5 | OPML import and export | Round-trips the user's Feedly export without losing a feed or folder |
| S2.6 | Favicons cached to disk | Shown in the source list; no network call after the first fetch |
| S2.7 | Onboarding: an Add sources screen with a verified catalog of feeds, keyword presets and MCP servers | A fresh install opens on Add sources with 12 starter feeds checked; Enter adds them and articles appear within 10 s; keyword and MCP entries show greyed out |

### E3 Reading

| ID | Story | Acceptance |
| --- | --- | --- |
| S3.1 | Article list with `j`/`k`, unread styling, mark read on open | 10k rows scroll at 60 fps; only visible rows render |
| S3.2 | Article pane renders the feed's HTML summary as styled text | Headings, links, lists, images, code, blockquotes; nothing escapes the pane |
| S3.3 | Reader view extracts the full page | Works on 9 of 10 outlets in `tests/reader_sites.txt`; falls back to the summary |
| S3.4 | Star, mark unread, mark folder read, open in browser, copy link | Each has a key and a palette entry |
| S3.5 | Command palette (`:`) and help (`?`) | Every action listed; fuzzy match; keys shown |
| S3.6 | Full-text search with FTS5 | `/` finds "Harrisburg" across 100k articles in under 50 ms |
| S3.7 | Views: all, unread, starred, today | Each a key and a palette entry; counts in the source pane |

### E4 Theme sync

| ID | Story | Acceptance |
| --- | --- | --- |
| S4.1 | Read `colors.toml` from both Omarchy theme paths and map to UI tokens | Each stock Omarchy theme renders with readable contrast; screenshots in `docs/themes/` |
| S4.2 | Font from `fc-match monospace`; size from config | Changing the font with `omarchy-font-set` changes the app |
| S4.3 | Hook drop-in and inotify fallback reload the theme live | `omarchy theme set` changes the app within 1 s with no restart |
| S4.4 | Built-in fallback theme when no Omarchy is found | Runs on stock Ubuntu with a sane look |
| S4.5 | Light mode from the theme's `mode` key | Light themes don't show dark panes |

### E5 Keyword alerts

| ID | Story | Acceptance |
| --- | --- | --- |
| S5.1 | Add a keyword source; query Google News RSS search | "Cory Booker" returns stories as a source; the catalog's keyword presets become addable on the Add sources screen |
| S5.2 | Add Bing News RSS and GDELT as providers; merge and dedupe | The same story from three providers appears once |
| S5.3 | Query syntax: quotes, `-exclude`, `site:`, language and region | Passed through to each provider that supports it |
| S5.4 | Group near-duplicate headlines into one story with a source count | Jaccard over title shingles; "5 sources" badge |

### E6 MCP sources

| ID | Story | Acceptance |
| --- | --- | --- |
| S6.1 | Add an MCP server by command (stdio) or URL (streamable HTTP) with `rmcp` | Lists the server's tools; connection errors are one line in the source pane |
| S6.2 | Pick a tool and an argument template; call it on refresh | A Cronkite or filesystem server's results show as articles; the catalog's MCP servers become addable on the Add sources screen |
| S6.3 | Map tool results to articles: JSON array, feed document, Markdown with links, or plain text | Fixtures for each shape; unknown shapes become one article |
| S6.4 | Secrets for HTTP servers from the system keyring or an env var | No token stored in `config.toml` |

### E7 Desktop polish and release

| ID | Story | Acceptance |
| --- | --- | --- |
| S7.1 | `.desktop` file, icon, Walker entry, `gutenberg --unread` for Waybar | Appears in Walker; Waybar module shows the count |
| S7.2 | GitHub release workflow and an install page on GitHub Pages with a `curl` one-liner | `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh \| sh` installs a checksum-verified, attested binary; `gutenberg --version` prints the release |
| S7.3 | Single-instance: a second launch focuses the first | Unix socket; `gutenberg --reload-theme` and `--refresh` work from scripts |
| S7.4 | Image cache on disk with size cap | Under 150 MB RAM with 10k articles |
| S7.5 | Crash-free week: panic handler writes a report; fuzz the parsers | `cargo fuzz` runs on feed and HTML inputs in CI |

### E8 Later

| ID | Story |
| --- | --- |
| S8.1 | macOS build with native font and menu bar |
| S8.2 | Windows build |
| S8.3 | Cronkite bridge: send articles to Cronkite for ranking over MCP |
| S8.4 | Rules: auto-star, auto-read, mute by keyword |
| S8.5 | Sync via a user-chosen folder (Syncthing-friendly) |

## Releases

| Release | Ships | Done when |
| --- | --- | --- |
| 0.1 Reader | E1, E2, E3.1 to E3.4 | Avniel reads his own OPML in it for a week |
| 0.2 Themed | E3.5 to E3.7, E4 | Every stock Omarchy theme has a screenshot and no contrast bug |
| 0.3 Alerts | E5, E7.1, E7.3 | Three keyword sources replace Google Alerts email |
| 0.4 MCP | E6 | Cronkite's output appears as a source |
| 1.0 | E7, docs, install site | Three outside users installed from the site; no open Blocker for two weeks |
| 1.x | E8 | macOS beta |

Cadence: tag a release whenever the `ship-it` skill's checks pass and at least one story is merged. Small releases, often.

## Risks and open questions

| Risk | Likelihood | Plan |
| --- | --- | --- |
| Rust GUI toolkits are young. iced's text rendering, scrolling of long lists, and image handling may fall short | Medium | Build S1.2 and S3.1 first as a spike. If iced can't do 10k rows at 60 fps, try gpui (Zed's toolkit) before writing more UI |
| Omarchy moves its theme paths or `colors.toml` keys between versions | Medium | Read both known paths; treat missing keys as fallbacks; pin the Omarchy version tested in the README |
| Google News RSS search changes or rate-limits | Medium | Three providers, so one can fail; cache results; back off on 429 |
| Reader view fails on paywalled or JS-heavy sites | High | Fall back to summary, then browser; never block the list on extraction |
| MCP tool results have no standard shape for articles | High | Define the four accepted shapes (S6.3) and document them for server authors |
| Scope creep toward a full RSS service | Medium | Non-goals are written down; anything needing a server goes to 1.x or Cronkite |

Open questions, each one decision:

- [ ] iced or gpui? Decide after the S1.2 and S3.1 spike.
- [ ] Final name and domain (see chat for candidates).
- [ ] Should keyword alerts be on by default with example queries, or empty?
- [ ] Does Cronkite become an MCP server, or does this app grow a ranker? Default: MCP server.

## Sources

- [omarchy-theme-set (basecamp/omarchy)](https://github.com/basecamp/omarchy/blob/master/bin/omarchy-theme-set): current theme directory, `theme.name`, the apps it reloads, and the `omarchy-hook theme-set` call
- [omarchy-hook](https://github.com/basecamp/omarchy/blob/master/bin/omarchy-hook): `~/.config/omarchy/hooks/<name>` and `<name>.d/` drop-ins
- [omarchy-font-set](https://github.com/basecamp/omarchy/blob/master/bin/omarchy-font-set): fontconfig as the canonical font source
- [tokyo-night colors.toml](https://raw.githubusercontent.com/basecamp/omarchy/master/themes/tokyo-night/colors.toml): the palette keys
- [How to create a custom Omarchy theme](https://instagit.com/basecamp/omarchy/how-to-create-custom-omarchy-theme/) and [Omarchy config layout](https://docs.docuwriter.ai/omarchy-user-docs/78801): secondary guides; they disagree on the theme path, which is why the app checks both
- [rmcp on docs.rs](https://docs.rs/crate/rmcp/3.4.1): the official Rust MCP SDK, stdio and streamable HTTP client transports
- [Rust GUI crates on lib.rs](https://www.lib.rs/gui): toolkit survey
