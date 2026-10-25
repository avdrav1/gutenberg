# Gutenberg: working notes for coding agents

Native Linux news reader for Omarchy, in Rust. Read `docs/PRD.md` for what we're building and `docs/BACKLOG.md` for what's next. Skills in `.claude/skills/` cover the repeatable jobs: `/next-story`, `/ship-it`, `/theme-check`, `/feed-fixture`.

## Layout

Cargo workspace. One crate per concern; the UI never touches the network.

| Crate | Owns |
|---|---|
| `crates/core` | sync engine, `Source` trait, dedupe and story grouping |
| `crates/store` | SQLite schema, migrations, FTS5 queries |
| `crates/sources` | `FeedSource`, `KeywordSource`, `McpSource` |
| `crates/theme` | Omarchy `colors.toml` → tokens, fontconfig, hook and inotify reload |
| `crates/ui` | iced window, panes, keys, command palette |
| `crates/cli` | the `gutenberg` binary: launches the UI, `--add <url>`, `--keyword <query>`, `--mcp <command or URL>` and `--import <file.opml>` write to the store directly, `--key <NAME>` saves an MCP server's key in the keyring, `--export` prints OPML, `--unread [--json]` reads the unread count from the store for a bar, or `--refresh`, `--reload-theme` over the Unix socket |

## Rules

- Every story is one PR from a branch named after its ID (`s2-3-conditional-get`). Never commit to `main`.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` before every commit. CI runs the same.
- No network in tests. Feeds, pages and MCP results come from `tests/fixtures/`. Add a fixture with `/feed-fixture`.
- The UI reads the store and subscribes to changes. It never awaits HTTP, a child process or a socket.
- Theme tokens come from `crates/theme`. No literal colors in `crates/ui`.
- Keys are documented in `docs/KEYS.md`. A new action gets a key, a palette entry and a line there in the same PR.
- A user-visible change gets a line in `CHANGELOG.md` under `Unreleased` in the same PR. Write it for a reader, not a developer.
- Config and data paths follow XDG. The app never writes outside `$XDG_CONFIG_HOME/gutenberg`, `$XDG_DATA_HOME/gutenberg`, `$XDG_CACHE_HOME/gutenberg` (article images and reader pages, under `[cache] megabytes`) and the window's socket, `$XDG_RUNTIME_DIR/gutenberg/socket`. Only `site/install.sh` writes elsewhere: the binary, the desktop entry and icon from `share/` in `~/.local/share`, and on Omarchy the two hooks and the bar widget in `~/.config/omarchy/plugins/gutenberg.unread`.

## Performance budgets

Cold start to first paint under 250 ms (Vulkan setup is ~170 ms of it; see S3.1). 10k articles scroll at 60 fps. Search under 50 ms at 100k articles. Under 150 MB of the window's own memory (`RssAnon`; the GPU driver's libraries add about 165 MB on NVIDIA) with 10k articles, measured by `GUTENBERG_MEMORY_BENCH=<articles>`, which opens that many in turn and reports. A story that breaks a budget isn't done; `cargo bench` has a benchmark for each of the rest.

## Omarchy facts

- Palette: `colors.toml` in `~/.local/state/omarchy/current/theme/` (Omarchy 3) or `~/.config/omarchy/current/theme/` (older). Check both.
- Theme change: `omarchy-hook theme-set <name>` runs `~/.config/omarchy/hooks/theme-set` and `theme-set.d/*`. `install.sh` installs `theme-set.d/gutenberg` and `font-set.d/gutenberg`, which run `gutenberg --reload-theme`. A window without them still follows the theme through inotify on the theme directory (`crates/theme/src/watch.rs`).
- Font: `fc-match monospace`; `omarchy-font-set` updates `~/.config/fontconfig/fonts.conf` and runs the `font-set` hook.
- No client-side decorations under Hyprland.
