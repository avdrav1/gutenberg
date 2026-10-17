# Backlog

Source of truth for `/next-story`. Status: `todo`, `doing`, `review`, `done`, `blocked`. Stories run in order within an epic; epics run in order unless a story says otherwise.

Epics in build order. Each story is one PR, sized for a day or less. "Done" means merged, tested, and visible in the app. The same list lives in `docs/BACKLOG.md` in the repo, where the `next-story` skill reads it.

### E1 Skeleton (first week)

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S1.1 | Cargo workspace with `core`, `store`, `sources`, `theme`, `ui`, `cli` crates | `cargo build` and `cargo test` pass in CI on Arch and Ubuntu | review |
| S1.2 | iced window opens under Hyprland with three empty panes and `q` quits | Window has no CSD; Hyprland tiles it; cold start under 100 ms | review |
| S1.3 | SQLite store with migrations and `Article`, `Source`, `Folder` tables | Migration runs on a fresh and an existing file; a test inserts 10k rows and lists them in under 20 ms | todo |
| S1.4 | Config file loads with defaults and hot-reloads on change | Edit `config.toml`; the change applies within 1 s with no restart | todo |

### E2 Feeds

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S2.1 | Add a feed by URL; parse RSS, Atom and JSON Feed | 20 fixture feeds from `tests/fixtures/` parse; bad XML shows one line of error | todo |
| S2.2 | Autodiscover a feed from a site URL | Pasting `https://example.com` finds `<link rel="alternate">`; falls back to `/feed`, `/rss`, `/atom.xml` | todo |
| S2.3 | Background refresh with conditional GET and per-host rate limits | A second refresh of an unchanged feed sends `If-None-Match` and gets 304; 100 feeds finish in under 10 s | todo |
| S2.4 | Folders, drag to reorder, unread counts per source and folder | Counts update as items are read; order persists | todo |
| S2.5 | OPML import and export | Round-trips the user's Feedly export without losing a feed or folder | todo |
| S2.6 | Favicons cached to disk | Shown in the source list; no network call after the first fetch | todo |

### E3 Reading

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S3.1 | Article list with `j`/`k`, unread styling, mark read on open | 10k rows scroll at 60 fps on a 4K monitor; only visible rows render. Risk: S1.2 moved to iced's tiny-skia CPU renderer for cold start; if 4K scroll misses 60 fps, revisit wgpu with a deferred GPU init | todo |
| S3.2 | Article pane renders the feed's HTML summary as styled text | Headings, links, lists, images, code, blockquotes; nothing escapes the pane | todo |
| S3.3 | Reader view extracts the full page | Works on 9 of 10 outlets in `tests/reader_sites.txt`; falls back to the summary | todo |
| S3.4 | Star, mark unread, mark folder read, open in browser, copy link | Each has a key and a palette entry | todo |
| S3.5 | Command palette (`:`) and help (`?`) | Every action listed; fuzzy match; keys shown | todo |
| S3.6 | Full-text search with FTS5 | `/` finds "Harrisburg" across 100k articles in under 50 ms | todo |
| S3.7 | Views: all, unread, starred, today | Each a key and a palette entry; counts in the source pane | todo |

### E4 Theme sync

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S4.1 | Read `colors.toml` from both Omarchy theme paths and map to UI tokens | Each stock Omarchy theme renders with readable contrast; screenshots in `docs/themes/` | todo |
| S4.2 | Font from `fc-match monospace`; size from config | Changing the font with `omarchy-font-set` changes the app | todo |
| S4.3 | Hook drop-in and inotify fallback reload the theme live | `omarchy theme set` changes the app within 1 s with no restart | todo |
| S4.4 | Built-in fallback theme when no Omarchy is found | Runs on stock Ubuntu with a sane look | todo |
| S4.5 | Light mode from the theme's `mode` key | Light themes don't show dark panes | todo |

### E5 Keyword alerts

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S5.1 | Add a keyword source; query Google News RSS search | "Cory Booker" returns stories as a source | todo |
| S5.2 | Add Bing News RSS and GDELT as providers; merge and dedupe | The same story from three providers appears once | todo |
| S5.3 | Query syntax: quotes, `-exclude`, `site:`, language and region | Passed through to each provider that supports it | todo |
| S5.4 | Group near-duplicate headlines into one story with a source count | Jaccard over title shingles; "5 sources" badge | todo |

### E6 MCP sources

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S6.1 | Add an MCP server by command (stdio) or URL (streamable HTTP) with `rmcp` | Lists the server's tools; connection errors are one line in the source pane | todo |
| S6.2 | Pick a tool and an argument template; call it on refresh | A Cronkite or filesystem server's results show as articles | todo |
| S6.3 | Map tool results to articles: JSON array, feed document, Markdown with links, or plain text | Fixtures for each shape; unknown shapes become one article | todo |
| S6.4 | Secrets for HTTP servers from the system keyring or an env var | No token stored in `config.toml` | todo |

### E7 Desktop polish and release

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S7.1 | `.desktop` file, icon, Walker entry, `gutenberg --unread` for Waybar | Appears in Walker; Waybar module shows the count | todo |
| S7.2 | AUR packages `gutenberg` and `gutenberg-bin`, GitHub release workflow | `yay -S gutenberg-bin` installs a signed binary | todo |
| S7.3 | Single-instance: a second launch focuses the first | Unix socket; `gutenberg --reload-theme` and `--refresh` work from scripts | todo |
| S7.4 | Image cache on disk with size cap | Under 150 MB RAM with 10k articles | todo |
| S7.5 | Crash-free week: panic handler writes a report; fuzz the parsers | `cargo fuzz` runs on feed and HTML inputs in CI | todo |

### E8 Later

| ID | Story | Status |
| --- | --- | --- |
| S8.1 | macOS build with native font and menu bar | todo |
| S8.2 | Windows build | todo |
| S8.3 | Cronkite bridge: send articles to Cronkite for ranking over MCP | todo |
| S8.4 | Rules: auto-star, auto-read, mute by keyword | todo |
| S8.5 | Sync via a user-chosen folder (Syncthing-friendly) | todo |

