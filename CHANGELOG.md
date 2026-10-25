# Changelog

## Unreleased

- MCP servers that need a key, such as Jina, GitHub, Brave and Tavily, can have one now. Choose the server and press `K` to type or paste it, or run `gutenberg --key JINA_API_KEY` in a terminal. The key is kept in your system keyring (GNOME Keyring or KWallet), never in Gutenberg's files, and a server on the web gets it too. An environment variable of the same name still comes first. A server that's missing its key, or turns it down, says so under its name. For a server you add by hand, name its key with `--secret`: `gutenberg --mcp <URL> --tool <name> --secret MY_API_KEY`.

## 0.18.0 — 2026-10-24

- An MCP server can now be a source of articles. Choose it in the sources pane, move to a tool with `j` and `k`, press Enter, and type its arguments as JSON. Each refresh then calls that tool, and every result with a title and a link becomes an article. Write `{{today}}` or `{{since}}` in an argument to get today's date or the date of the last call. Press `T` to pick another tool. From a terminal: `gutenberg --mcp <command or URL> --tool <name> --args '<json>'`.
- The MCP servers on **Add sources** can now be checked like feeds, and each comes with its tool and arguments filled in. A server marked "needs a key" that runs on your machine reads it from your environment, such as `BRAVE_API_KEY`. Keys for servers on the web come in a later version.
- MCP servers that answer in text now give one article per result too: Exa's search results, Hugging Face's tables and repository lists, Markdown with a link per heading or per line, and RSS, Atom or JSON feeds. A server that answers with a rate-limit or quota notice shows it as the server's error, one line under its name, instead of as an article.

## 0.17.0 — 2026-10-24

- Add an MCP server as a source: `gutenberg --mcp '<command>'` for one that runs on your machine, or `gutenberg --mcp <url>` for one on the web. Gutenberg connects, lists the server's tools, and shows them when you choose the server in the sources pane. If it can't reach a server, one line under the server's name says why, such as a missing API key or a refused connection, and Gutenberg tries again later. Reading articles from a tool comes next.

## 0.16.0 — 2026-10-24

- Keyword alerts can search another country's news: add `lang:fr` for French news from France, `region:CA` for Canada in English, or both, as in `Carney lang:fr region:CA`. Words after a dash (`-opinion`, `-"op-ed"`, `-site:example.com`) now keep those stories out of Bing's results too, which ignored them, and a query with `OR` is asked of Google News alone, since Bing finds nothing for it.
- When several outlets run the same story, from your feeds or a keyword alert, the list shows it once, with its newest headline and a count such as "8 sources". Opening it marks the whole story read, and the article pane lists the other outlets as links. Unread counts count stories. Articles you already have are grouped at the first refresh after updating.

## 0.15.0 — 2026-10-24

- A light Omarchy theme that leaves some colors out now gets light defaults for them, not dark ones. The theme's `mode` decides; a theme without one is judged by its background.
- Keyword alerts now search Bing News as well as Google News, and a story both carry shows up once. Google News results move from Google's link to the publisher's own, 20 per alert at each refresh, so `v` opens the article itself. If one search fails, the alert still updates from the other.

## 0.14.0 — 2026-10-24

- A bar along the bottom of the window shows the keys for what you're doing: moving and opening in the list, `v` and `s` with an article open, Enter and Esc while you search. Before you open anything, the article pane says how. To hide the bar, set `hints = false` under `[keys]` in `~/.config/gutenberg/config.toml`.

## 0.13.0 — 2026-10-24

- https://gutenbergapp.com now shows what Gutenberg looks like: a short clip at the top, a screenshot of each feature, the app in eight Omarchy themes, and a fifty-second tour set to music.

## 0.12.0 — 2026-10-24

- Without Omarchy, on Ubuntu or any other desktop, Gutenberg follows your light or dark setting. It uses Catppuccin Latte when you prefer light, Tokyo Night otherwise, and switches when you change the setting.
- Scrollbars and checkboxes now take the theme's colors too, instead of a fixed light gray.

## 0.11.0 — 2026-10-24

- An open window now follows `omarchy theme set` and `omarchy font set`. The colors change within a fraction of a second and the font right after, with no restart. Reinstall with the curl one-liner to add the two small Omarchy hooks that make this immediate. Without them, Gutenberg notices the change on its own.
- `gutenberg --reload-theme` makes an open window read the theme and font again, for scripts.

## 0.10.0 — 2026-10-24

- Gutenberg now takes its colors from your Omarchy theme, light themes included, instead of always looking like Tokyo Night. Where a theme's dim text or link color would be hard to read, it is brightened (or darkened) just enough to read. Themes from before Omarchy 4 work too. Changing the theme still needs a restart for now.
- Text is in your desktop's monospace font, the one `omarchy font set` chooses, like the terminal and Waybar; after changing it, restart Gutenberg to see it. Set the text size with `size` under `[font]` in `~/.config/gutenberg/config.toml` (8 to 32, default 14); the whole window scales with it, and edits apply as you save.
- **Keyword alerts.** A news search becomes a source: Gutenberg asks Google News for it on every refresh and lists what it finds, headline first, with the publisher beside the source.
  - On **Add sources**, the keyword presets in each category can now be checked like feeds.
  - The new **Keyword alerts** page takes your own query: press `n`, type something like `"Cory Booker"`, and press Enter.
  - From a terminal: `gutenberg --keyword '"Cory Booker"' [--folder <name>]`.
  - Quotes, `-word` and `site:` work the way Google News reads them.

## 0.9.0 — 2026-10-23

- The top of the left pane has four views with their counts: All articles, Unread, Starred and Today (published since midnight). Click one to see it across every feed, or press `u`, `S`, `t` or `A` to narrow the folder or feed you're reading. `M` marks read only what the view shows.

## 0.8.0 — 2026-10-23

- Press `/` to search every article you have. Type a few letters and the list shows matching titles, authors and summaries as you type, newest first; `Enter` lets you move through the results, `Esc` brings your list back. The first launch after updating builds the search index for articles you already have, which takes a moment on a large collection.

## 0.7.0 — 2026-10-23

- New keys, also in the `:` palette and `?` help: `s` stars an article (a ★ shows in the list), `m` marks it unread or read again, `M` marks everything in the chosen folder or feed read, `O` opens the article in your browser, and `y` copies its link.

## 0.6.0 — 2026-10-23

- Press `:` for the command palette: type a few letters of what you want to do (`rv` for reader view) and press Enter. Press `?` for a list of every key.

## 0.5.0 — 2026-10-23

- The article pane shows a story the way the feed wrote it: headings, bold and italic text, lists, code, quotes and pictures, instead of one block of plain text. Click a link to open it in your browser. Long lines and big pictures fit the pane's width.
- Press `v` to read the whole article instead of the feed's short summary. Gutenberg loads the page and keeps only the story, laid out in the app's own style. Press `v` again to go back. If a page can't be read, the summary stays and a line says why.

## 0.4.0 — 2026-10-22

- Each feed in the left pane shows its site's icon. Gutenberg fetches it once, the first time it refreshes the feed, and keeps it in `~/.local/share/gutenberg/favicons/`; after that it never asks the site again.

## 0.3.1 — 2026-10-22

- The install page now lives at https://gutenbergapp.com: `curl -fsSL https://gutenbergapp.com/install.sh | sh`. The old address keeps working.

## 0.3.0 — 2026-10-22

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
