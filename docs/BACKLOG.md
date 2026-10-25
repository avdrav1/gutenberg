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
| S3.8 | Key hints: a bar along the bottom with the keys for what's on screen, drawn from `commands::ALL`, and a hint in the empty article pane; `hints = false` under `[keys]` in `config.toml` hides the bar | A new user sees how to move, open, search and get help without pressing a key; the bar changes in search and reader view (the palette, help and Add sources keep their own hint lines) | done |

### E4 Theme sync

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S4.1 | Read `colors.toml` from both Omarchy theme paths and map to UI tokens | Each stock Omarchy theme renders with readable contrast; screenshots in `docs/themes/` | done |
| S4.2 | Font from `fc-match monospace`; size from config | Changing the font with `omarchy-font-set` changes the app | done |
| S4.3 | Hook drop-in and inotify fallback reload the theme live (and the font, on the `font-set` hook: S4.2 points iced's sans-serif and monospace families at the fontconfig family once at launch, so a live change has to reset them and clear cosmic-text's font-match cache and iced's text caches) | `omarchy theme set` changes the app within 1 s with no restart | done |
| S4.4 | Built-in fallback theme when no Omarchy is found | Runs on stock Ubuntu with a sane look | done |
| S4.5 | Light mode from the theme's `mode` key: a key the file leaves out comes from the built-in theme for its mode (Catppuccin Latte or Tokyo Night), and without a `mode`, as in the older terminal layout, from its background's lightness. (The panes were already the theme's own backgrounds since S4.1, and derived panes mix toward the foreground, so they go darker on a light theme.) | Light themes don't show dark panes | done |

### E5 Keyword alerts

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S5.1 | Add a keyword source; query Google News RSS search | "Cory Booker" returns stories as a source; the catalog's keyword presets become addable on the Add sources screen | done |
| S5.2 | Add Bing News RSS as a provider; merge and dedupe by guid, canonical URL and headline. Google News links (`news.google.com/rss/articles/…`) reach the publisher by script, so they're resolved through the signed `batchexecute` request their page carries, 20 per alert per refresh (two requests each). GDELT moved to S5.5: its API answered every request with 429 on 2026-10-24 | The same story from two providers appears once | done |
| S5.3 | Query syntax: quotes, `-exclude`, `site:`, language and region. Language and region are `lang:fr` and `region:CA` in the query, so presets, the CLI and the Add sources field take them with no new settings. Checked live on 2026-10-24: Google honors quotes, `-word`, `site:` and `OR`, and redirects any `hl`/`gl`/`ceid` to its nearest edition; Bing honors quotes, `site:` and `mkt` (even markets outside its documented list), ignores `-word`, and returns nothing for `OR`. So exclusions are also applied to every provider's results, and Bing sits out a query with `OR` | Passed through to each provider that supports it | done |
| S5.4 | Group near-duplicate headlines into one story with a source count. Across every source; `article.story_id` (migration 0004) set after each refresh. Shingles are single words (alphabetic, two letters or more, no stop words, at least three per headline), within 3 days, at 0.5: on real feeds and alerts, pairs at 0.5 or more were the same story and below it different stories about the same person began to pair. A folder's count is still the sum of its feeds', so a story in two of its feeds counts twice there | Jaccard over title shingles; "5 sources" badge | done |
| S5.5 | Add GDELT's doc API (`api.gdeltproject.org/api/v2/doc/doc?mode=artlist&format=json`) as a third provider. It asks for one request every 5 s per client and answered 429 to every request on 2026-10-24, even a minute apart, so: per-host spacing of 5 s for its host, capture a real response as a fixture before writing the parser, and treat a 429 as that provider sitting out the refresh. Give it a `provider_query` and edition the way S5.3 did for the other two: it reads quotes, `-word` and `OR` (in parentheses), and takes language and country as `sourcelang:` and `sourcecountry:` | The same story from three providers appears once Blocked 2026-10-24: the API answered 429 to the first request from this machine again, hours after the last, and no fixture can be captured | blocked |

### E6 MCP sources

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S6.1 | Add an MCP server by command (stdio) or URL (streamable HTTP) with `rmcp`. Added with `gutenberg --mcp`; the tools a server lists are kept in `source.config` and shown when it's chosen. A command runs in its own process group, ended after each use (rmcp only ends the process it started, on a task that may never run). rmcp's HTTP client is reqwest 0.13, so the binary carries two reqwest versions until the feed client moves to 0.13 | Lists the server's tools; connection errors are one line in the source pane | done |
| S6.2 | Pick a tool and an argument template; call it on refresh. Picked in the window (the tools list from S6.1: `j`/`k`, Enter edits a one-line JSON field prefilled from the tool's input schema; `T` shows the tools again) or with `--tool`/`--args`. Templates fill `{{today}}` and `{{since}}` (UTC days). Results read as a list of items with a title and URL at any depth in `structuredContent` or a JSON text block, else one article keyed on its content; a list of nothing is nothing new. Checked live on 2026-10-24: the Hugging Face Daily Papers preset's arguments no longer fit `hf_fs` (fixed); Exa's keyless server first answered with a rate-limit notice as an ordinary result, then with its results as `Title:`/`URL:`/`Published:` text blocks, which read as one article until S6.3 | The catalog's keyless HTTP presets (Hugging Face Daily Papers, Exa News Search) show results as articles; the catalog's MCP servers become addable on the Add sources screen | done |
| S6.3 | Map tool results to articles: JSON array, feed document, Markdown with links, or plain text. Read in this order (`sources/src/mcp/articles.rs`, text shapes in `articles/text.rs`): a feed document (RSS, Atom or JSON Feed, in a text block or an embedded `resource`); a JSON list (S6.2); labelled records (`Title:`, `URL:`, `Published:`, `Author:`, then a multi-line `Highlights:`, as Exa's `web_search_exa` writes them, parted by `---`); Markdown tables, whose `key=value, …` cells (Hugging Face's `hf_fs` Details column) give a field each; Markdown sections, a heading with one link under it (Hugging Face's `hub_repo_search`); lists of Markdown links. Sections and links take two articles at least. Then a notice: an answer under 600 characters about a rate limit or quota is a failure, not an article (Exa's keyless server sent one unmarked on 2026-10-24). Checked live on 2026-10-24: Exa News Search gives 20 articles with links and dates, up from one; Hugging Face trending papers still give 10. The feed-document test wraps existing feed fixtures, since no keyless server answers with a feed | Fixtures for each shape; unknown shapes become one article | done |
| S6.4 | Secrets for HTTP servers from the system keyring or an env var. A server's settings name its key (`secret`, like `JINA_API_KEY`); each refresh takes the environment variable, else the Secret Service keyring item `application=gutenberg key=<NAME>` (`secret-service` crate, async on tokio; a refresh never asks to unlock it). A command gets it as that variable, a URL as `Authorization: Bearer` (every catalog server with a key takes a bearer; one wanting another header would need a setting). Entered with `K` in the window (dots; saved off the update loop, since the keyring may prompt) or `gutenberg --key NAME` from stdin; `--mcp … --secret NAME` names it. Checked live on 2026-10-24: a test key saved with `--key` read back with `secret-tool`, and Jina answered "Authentication failed" with it against "Please set your API key" without, so the header arrived | No token stored in `config.toml` | review |

### E7 Desktop polish and release

| ID | Story | Acceptance | Status |
| --- | --- | --- | --- |
| S7.1 | `.desktop` file, icon, Walker entry, `gutenberg --unread` for Waybar | Appears in Walker; Waybar module shows the count | todo |
| S7.2 | GitHub release workflow and an install page on GitHub Pages with a `curl` one-liner | `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh \| sh` installs a checksum-verified, attested binary; `gutenberg --version` prints the release | done |
| S7.3 | Single-instance: a second launch focuses the first (the socket exists since S4.3: `core::ipc`, `$XDG_RUNTIME_DIR/gutenberg/socket`, one command per line, with `reload-theme`; a second window currently runs without it) | Unix socket; `gutenberg --reload-theme` and `--refresh` work from scripts | todo |
| S7.4 | Image cache on disk with size cap (also prunes `favicons/` files of feeds no longer in the store, which S2.6 never deletes; article images, held in memory only since S3.2, and reader pages since S3.3, are fetched again each time an article opens; a cached reader page goes into `article_fts.body` so search finds it) | Under 150 MB RAM with 10k articles | todo |
| S7.5 | Crash-free week: panic handler writes a report; fuzz the parsers (also a CI job that launches the release tarball on `ubuntu:24.04` under Xvfb with a session bus and `xdg-desktop-portal-gtk`, as S4.4 did by hand, and fails on a crash or a blank window) | `cargo fuzz` runs on feed, OPML, HTML and reader-extraction inputs in CI | todo |

### E8 Later

| ID | Story | Status |
| --- | --- | --- |
| S8.1 | macOS build with native font and menu bar | todo |
| S8.2 | Windows build | todo |
| S8.3 | Cronkite bridge: send articles to Cronkite for ranking over MCP | todo |
| S8.4 | Rules: auto-star, auto-read, mute by keyword | todo |
| S8.5 | Sync via a user-chosen folder (Syncthing-friendly) | todo |

