# Backlog

Source of truth for `/next-story`. Status: `todo`, `doing`, `review`, `done`, `blocked`. Stories run in order within an epic; epics run in order unless a story says otherwise.

Epics in build order. Each story is one PR, sized for a day or less. "Done" means merged, tested, and visible in the app. The same list lives in `docs/BACKLOG.md` in the repo, where the `next-story` skill reads it.

### E1 Skeleton (first week)

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S1.1 | Cargo workspace with `core`, `store`, `sources`, `theme`, `ui`, `cli` crates | `cargo build` and `cargo test` pass in CI on Arch and Ubuntu | done |
| S1.2 | iced window opens under Hyprland with three empty panes and `q` quits | Window has no CSD; Hyprland tiles it; cold start under 100 ms (met on tiny-skia; superseded by the 250 ms budget when S3.1 moved to wgpu) | done |
| S1.3 | SQLite store with migrations and `Article`, `Source`, `Folder` tables | Migration runs on a fresh and an existing file; a test inserts 10k rows and lists them in under 20 ms | done |
| S1.4 | Config file loads with defaults and hot-reloads on change | Edit `config.toml`; the change applies within 1 s with no restart | done |

### E2 Feeds

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S2.1 | Add a feed by URL; parse RSS, Atom and JSON Feed | 20 fixture feeds from `tests/fixtures/` parse; bad XML shows one line of error | done |
| S2.2 | Autodiscover a feed from a site URL | Pasting `https://example.com` finds `<link rel="alternate">`; falls back to `/feed`, `/rss`, `/atom.xml` | done |
| S2.3 | Background refresh with conditional GET and per-host rate limits | A second refresh of an unchanged feed sends `If-None-Match` and gets 304; 100 feeds finish in under 10 s | done |
| S2.4 | Folders, drag to reorder, unread counts per source and folder (after S3.1; folders from `--add --folder`, sources moved by drag) | Counts update as items are read; order persists | done |
| S2.5 | OPML import and export | Round-trips the user's Feedly export without losing a feed or folder | done |
| S2.6 | Favicons cached to disk | Shown in the source list; no network call after the first fetch | done |
| S2.7 | Onboarding: an Add sources screen with a verified catalog of feeds, keyword presets and MCP servers | A fresh install opens on Add sources with 12 starter feeds checked; Enter adds them and articles appear within 10 s; keyword and MCP entries show greyed out | done |

### E3 Reading

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S3.1 | Article list with `j`/`k`, unread styling, mark read on open | 10k rows scroll at 60 fps on a 4K monitor; only visible rows render. tiny-skia managed ~10 fps at 4K, so this moves to wgpu (Vulkan) and the cold-start budget to 250 ms | done |
| S3.2 | Article pane renders the feed's HTML summary as styled text | Headings, links, lists, images, code, blockquotes; nothing escapes the pane | done |
| S3.3 | Reader view extracts the full page (draw it with S3.2's `article::parse` blocks) | Works on 9 of 10 outlets in `tests/reader_sites.txt`; falls back to the summary | done |
| S3.5 | Command palette (`:`) and help (`?`) (before S3.4, so its actions land with palette entries) | Every action listed; fuzzy match; keys shown | done |
| S3.4 | Star, mark unread, mark folder read, open in browser, copy link (each a `list::Action`, a key in `action_for`, a row in `commands::ALL`; the test there holds them together) | Each has a key and a palette entry | done |
| S3.6 | Full-text search with FTS5 | `/` finds "Harrisburg" across 100k articles in under 50 ms | done |
| S3.7 | Views: all, unread, starred, today (a starred view lists what `s` stars since S3.4; `Filter` grows a view alongside its folder/feed part, and `mark_read_in` follows it) | Each a key and a palette entry; counts in the source pane | done |

### E4 Theme sync

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S4.1 | Read `colors.toml` from both Omarchy theme paths and map to UI tokens | Each stock Omarchy theme renders with readable contrast; screenshots in `docs/themes/` | doing |
| S4.2 | Font from `fc-match monospace`; size from config | Changing the font with `omarchy-font-set` changes the app | todo |
| S4.3 | Hook drop-in and inotify fallback reload the theme live | `omarchy theme set` changes the app within 1 s with no restart | todo |
| S4.4 | Built-in fallback theme when no Omarchy is found | Runs on stock Ubuntu with a sane look | todo |
| S4.5 | Light mode from the theme's `mode` key | Light themes don't show dark panes | todo |

### E5 Keyword alerts

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S5.1 | Add a keyword source; query Google News RSS search | "Cory Booker" returns stories as a source; the catalog's keyword presets become addable on the Add sources screen | todo |
| S5.2 | Add Bing News RSS and GDELT as providers; merge and dedupe | The same story from three providers appears once | todo |
| S5.3 | Query syntax: quotes, `-exclude`, `site:`, language and region | Passed through to each provider that supports it | todo |
| S5.4 | Group near-duplicate headlines into one story with a source count | Jaccard over title shingles; "5 sources" badge | todo |

### E6 MCP sources

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S6.1 | Add an MCP server by command (stdio) or URL (streamable HTTP) with `rmcp` | Lists the server's tools; connection errors are one line in the source pane | todo |
| S6.2 | Pick a tool and an argument template; call it on refresh | A Cronkite or filesystem server's results show as articles; the catalog's MCP servers become addable on the Add sources screen | todo |
| S6.3 | Map tool results to articles: JSON array, feed document, Markdown with links, or plain text | Fixtures for each shape; unknown shapes become one article | todo |
| S6.4 | Secrets for HTTP servers from the system keyring or an env var | No token stored in `config.toml` | todo |

### E7 Desktop polish and release

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S7.1 | `.desktop` file, icon, Walker entry, `gutenberg --unread` for Waybar | Appears in Walker; Waybar module shows the count | todo |
| S7.2 | GitHub release workflow and an install page on GitHub Pages with a `curl` one-liner | `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh \| sh` installs a checksum-verified, attested binary; `gutenberg --version` prints the release | done |
| S7.3 | Single-instance: a second launch focuses the first | Unix socket; `gutenberg --reload-theme` and `--refresh` work from scripts | todo |
| S7.4 | Image cache on disk with size cap (also prunes `favicons/` files of feeds no longer in the store, which S2.6 never deletes; article images, held in memory only since S3.2, and reader pages since S3.3, are fetched again each time an article opens; a cached reader page goes into `article_fts.body` so search finds it) | Under 150 MB RAM with 10k articles | todo |
| S7.5 | Crash-free week: panic handler writes a report; fuzz the parsers | `cargo fuzz` runs on feed, OPML, HTML and reader-extraction inputs in CI | todo |

### E8 Later

| ID | Story | Status |
| --- | --- | --- |
| S8.1 | macOS build with native font and menu bar | todo |
| S8.2 | Windows build | todo |
| S8.3 | Cronkite bridge: send articles to Cronkite for ranking over MCP | todo |
| S8.4 | Rules: auto-star, auto-read, mute by keyword | todo |
| S8.5 | Sync via a user-chosen folder (Syncthing-friendly) | todo |

