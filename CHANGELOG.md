# Changelog

## Unreleased

- A fresh install opens on **Add sources**: 232 hand-picked, checked-working news feeds in 21 categories, from World and Science to Linux and Food. Twelve free starter feeds are already ticked, so pressing `Enter` gives you something to read within seconds. Paywalled outlets are labelled. Press `a` any time to open it again; feeds you already have show as added. Keyword alerts and MCP servers are listed too, greyed out until those features arrive.
- Feeds you add, from Add sources or `gutenberg --add`, are fetched within a second instead of at the next 30-minute refresh.
- Bring your subscriptions over from Feedly or any other reader: `gutenberg --import subscriptions.opml` adds every feed and folder in the file (feeds you already have are left alone), and the articles arrive with the next refresh. `gutenberg --export > subscriptions.opml` writes your feeds and folders back out. A feed filed in several Feedly categories lands in the first one.

## 0.2.0 — 2026-10-20

- The left pane lists your feeds with unread counts, grouped in folders. Click a feed or folder to see only its articles, and drag feeds to reorder them or move them between folders (drag folders to reorder them too). Counts go down as you read. Put a feed in a folder when you add it: `gutenberg --add <url> --folder Tech`.
- The middle pane lists your articles, newest first. `j`/`k` move, `gg`/`G` jump to the top or bottom, and `o` or `Enter` (or a click) opens one in the right pane and marks it read. Unread titles are bold. New articles from a refresh appear on their own.
- Feeds refresh in the background while the app is open: at launch, then every 30 minutes (`[refresh] minutes` in `config.toml`; 0 turns it off). `gutenberg --refresh` refreshes now and prints what changed. Unchanged feeds cost a single request, and a feed that's down is retried later instead of showing an error.
- `gutenberg --add` takes a site address too: it finds the feed the site links to, or tries `/feed`, `/rss` and `/atom.xml`.

## 0.1.0 — 2026-10-20

- Running `gutenberg` opens the main window with its three panes. Press `q` to quit.
- Pane widths can be set in `~/.config/gutenberg/config.toml` under `[layout]` (`sources`, `list`, `article`; 0 hides a pane). Edits apply while the app is open.
- Add a feed from the terminal with `gutenberg --add <feed-url>`. RSS, Atom and JSON Feed all work; a bad feed or address prints one line saying what went wrong.
- Install with one line: `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh | sh`. The script checks the download against the release checksums and installs to `~/.local/bin`.
