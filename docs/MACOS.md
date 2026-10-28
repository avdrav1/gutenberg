# Gutenberg on macOS

The same program as on Linux, built for Apple silicon and Intel. What differs is where things live, what stands in for Omarchy, and the first launch of an app nobody has notarized.

## Install

```sh
curl -fsSL https://gutenbergapp.com/install.sh | sh
```

The script puts the binary in `~/.local/bin/gutenberg` and `Gutenberg.app` in `~/Applications`, both checked against the release's `SHA256SUMS`. `GUTENBERG_INSTALL_DIR` moves the binary and `GUTENBERG_APP_DIR` the app; `GUTENBERG_APP_DIR=` (empty) installs the binary alone. A terminal running under Rosetta still gets the Apple silicon build. Both files have a build attestation: `gh attestation verify Gutenberg-aarch64-apple-darwin.zip -R avdrav1/gutenberg`.

`gutenberg --update` from the binary installs the next release the same way, app included. From inside `Gutenberg.app` it says to download the new app from the releases page instead, since swapping the executable alone would leave the bundle's `Info.plist` on the old version.

## The first open

The app and the binary are signed ad hoc (`codesign -s -`): there is no Apple Developer ID behind them, so nothing is notarized. What that means depends on how the file arrived:

- Installed by the script, or fetched with `curl`: it opens as it is. `curl` sets no quarantine flag, so Gatekeeper has nothing to assess.
- Downloaded with a browser from the releases page: macOS holds it the first time. Open System Settings › Privacy & Security, find the line about Gutenberg near the bottom and choose Open Anyway, then open it again. On macOS 15 and later, right-click › Open alone isn't enough.

The script never removes the quarantine flag on your behalf.

Each `--update` replaces the executable with one whose ad-hoc signature has no stable identity, so the keychain asks once more before it hands over MCP keys to the new build.

## Where things are

| | Linux | macOS |
| --- | --- | --- |
| Settings | `~/.config/gutenberg/config.toml` | `~/Library/Application Support/gutenberg/config.toml` |
| Articles, crash reports | `~/.local/share/gutenberg/` | `~/Library/Application Support/gutenberg/` |
| Pictures and reader pages | `~/.cache/gutenberg/` | `~/Library/Caches/gutenberg/` |
| The window's socket | `$XDG_RUNTIME_DIR/gutenberg/socket` | `$TMPDIR/gutenberg/socket` |
| MCP keys | Secret Service (`secret-tool`) | The login keychain |

The paths come from the `directories` crate, which maps the XDG directories to `~/Library` on macOS. `$TMPDIR` is the per-user `/var/folders/…/T/`, so the socket is private and gone at reboot.

An MCP key saved with `K` in the window or `gutenberg --key NAME` is a generic password with service `gutenberg` and account `NAME`:

```sh
security find-generic-password -s gutenberg -a NAME -w
```

## Theme and font

There is no Omarchy theme on a Mac, so the built-in light or dark theme applies and follows the system appearance as it changes. Text is in the system font (San Francisco) and code in Menlo; there is no fontconfig alias to follow, so `[font] size` in `config.toml` is the setting that applies. `gutenberg --reload-theme` works over the socket but has nothing beyond the appearance to reload.

The window has the system title bar. Keys and the command palette (`:`) are the whole interface, as on Linux: there is no native menu bar yet, because iced 0.14 has no API for one.

## Scripts and the menu bar

`gutenberg --unread`, `--unread --json`, `--refresh` and `--reload-theme` work as in [DESKTOP.md](DESKTOP.md). There is no bar widget; a menu-bar tool such as SwiftBar can run `gutenberg --unread` on a timer and `open -a Gutenberg` on click.

## Known gaps

- An app started from Finder or the Dock has launchd's `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), so an MCP server given as `npx …` or `uvx …` isn't found; use the command's full path (`/opt/homebrew/bin/npx`), or start Gutenberg from a terminal.
- Cold start and memory haven't been measured on a Mac. `GUTENBERG_MEMORY_BENCH` reports resident and peak memory there; the Linux budget's own-memory figure (`RssAnon`) has no macOS equivalent.
- No notarization, no Homebrew cask, no universal binary: two zips and two tarballs, one per architecture.
