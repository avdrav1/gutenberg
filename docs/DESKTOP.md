# Gutenberg on the desktop

`install.sh` adds Gutenberg to the app launcher and, on Omarchy, a bar widget with the unread count. Nothing goes on the bar by itself.

## App launcher

`~/.local/share/applications/gutenberg.desktop` and the icon in `~/.local/share/icons/hicolor/scalable/apps/gutenberg.svg`. Omarchy's launcher (Super+Alt+Space, or Walker on older Omarchy) and other desktops' launchers list it as Gutenberg. The entry runs the binary by its full path, and its `StartupWMClass` is the window's app id, `gutenberg`.

## One window

Gutenberg keeps to one window. Launching it again, from the launcher, the bar widget or a terminal, brings the open window forward, switching to its workspace, and the new launch exits. On Hyprland this needs `misc:focus_on_activate`, which Omarchy turns on; elsewhere the window may only be marked as wanting attention.

## From scripts

The open window listens on `$XDG_RUNTIME_DIR/gutenberg/socket`:

```sh
gutenberg --refresh        # 12 sources: 2 updated (5 new articles), 10 unchanged, 0 failed
gutenberg --reload-theme   # read the Omarchy theme and font again
```

`--refresh` refreshes every source that's due and waits for it. With the window open, the window does it, and what failed shows under each source's name; with no window, the command does it and prints each failure. `--reload-theme` with no window open says so and exits 0. Omarchy's theme and font hooks run it.

The socket takes one command per line and answers with one line: `reload-theme`, `refresh` or `focus`, then `ok`, `ok <summary>`, `error <why>` or `unknown`.

## The unread count

```sh
gutenberg --unread          # 12
gutenberg --unread --json   # {"text":"12","tooltip":"12 unread stories in Gutenberg","class":"unread","alt":"unread"}
```

It's the number the Unread view shows, with a story from several outlets counted once. It's read from the store, so the window needn't be open, and takes a few milliseconds. With nothing unread, the JSON's `text` is empty and its `class` is `read`, so a bar can hide the module.

### Omarchy bar

`install.sh` puts a widget in `~/.config/omarchy/plugins/gutenberg.unread/`. To show it:

```sh
omarchy bar put gutenberg.unread
```

It shows a newspaper with the count, checked every 30 seconds. Click to open Gutenberg; right-click to count again.

### Waybar

```jsonc
"custom/gutenberg": {
  "exec": "~/.local/bin/gutenberg --unread --json",
  "return-type": "json",
  "interval": 30,
  "format": "{} ",
  "on-click": "~/.local/bin/gutenberg"
}
```

Add `"custom/gutenberg"` to a `modules-*` list, and style it with `#custom-gutenberg.unread` or `#custom-gutenberg.read`.

The Omarchy shell's own command modules take the same JSON: `{ "id": "gutenberg", "type": "command", "exec": "~/.local/bin/gutenberg --unread --json", "interval": 30, "onClick": "~/.local/bin/gutenberg" }` in `bar.layout` of `~/.config/omarchy/shell.json`.
