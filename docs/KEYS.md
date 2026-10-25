# Keys

The bar along the bottom of the window shows the keys for what's on screen; `hints = false` under `[keys]` in `config.toml` hides it. `?` lists them all.

| Key | Action | Pane |
|---|---|---|
| `j` / `k` | next / previous item | list, sources |
| `gg` / `G` | first / last | list |
| `o` / `Enter` | open article (a story's row opens its newest article and marks the whole story read) | list |
| `v` | reader view: the full article from its page, or back to the feed's summary | list, article |
| `O` | open the selected article in the browser | list, article |
| `m` | mark the selected article unread, or read again (a story: every article in it) | list |
| `M` | mark read: everything the list shows for the folder or feed chosen in the sources pane and the current view | any |
| `s` | star or unstar the selected article (★ in the list) | list, article |
| `y` | copy the selected article's link | list, article |
| `/` | search every article's title, author and summary as you type | any |
| `r` | refresh selected source; `R` all | any |
| `u` / `S` / `t` / `A` | view unread only / starred / today's (published since midnight) / all; keeps the chosen folder or feed | any |
| `T` | the chosen MCP server's tools, to pick the one its refresh calls; again for its articles | list |
| `K` | type the key the chosen MCP server needs (such as `JINA_API_KEY`); it's saved in your keyring | list |
| `a` | add sources (the catalog) | any |
| `Tab` / `Shift-Tab` | next / previous pane | any |
| `1` `2` `3` | toggle sources / list / article pane | any |
| `:` | command palette: find any action by name | any |
| `?` | help: every action and its keys | any |
| `q` | quit | any |

## Command palette

Opened with `:`. Type part of an action's name; letters only need to appear in order, so `rv` finds "Reader view".

| Key | Action |
|---|---|
| any letter | narrow the list |
| `Backspace` | delete the last letter |
| `↓` / `↑`, `Ctrl-n` / `Ctrl-p`, `Tab` / `Shift-Tab` | next / previous match |
| `Enter` | run the highlighted action (a click runs one too) |
| `Esc` | close without running anything |

Help (`?`) closes with `?`, `Esc` or `q`.

## Search

Opened with `/`. The list shows the newest 500 articles holding every word typed; words match their beginnings, so `harris` finds Harrisburg.

| Key | Action |
|---|---|
| any letter | narrow the results |
| `Backspace` | delete the last letter |
| `↓` / `↑` | move through the results while typing |
| `Enter` | stop typing; `j`/`k`, `o` and the rest work on the results |
| `/` | type in the search again |
| `Esc` | end the search; the folder or feed you were reading comes back |

## MCP server's tools

Shown in the list pane when you choose an MCP server that has no articles yet, and opened with `T`. The tool marked "calls on refresh" is the one each refresh calls.

| Key | Action |
|---|---|
| `j` / `k` / `↓` / `↑` | next / previous tool |
| `gg` / `G` | first / last tool |
| `o` / `Enter` | edit the arguments of the tool under the cursor |
| `Esc` / `T` | back to the server's articles |

While typing arguments (a JSON object; `{{today}}` and `{{since}}` in a string become today's date and the date of the last call that worked):

| Key | Action |
|---|---|
| any letter | type it |
| `Backspace` | delete the last letter |
| `Enter` | save: the server calls this tool with these arguments from the next refresh, within a second |
| `Esc` | stop typing without saving |

While typing a server's key (after `K`; it shows as dots):

| Key | Action |
|---|---|
| any letter | type it; pasting works too |
| `Backspace` | delete the last letter |
| `Enter` | save it in the system keyring; the server is reached with it within a second. An environment variable of the same name still comes first |
| `Esc` | stop typing without saving |

## Add sources screen

Shown on first run, when the store has no sources, and opened with `a`. The left column lists the catalog's categories, then **Keyword alerts** and **MCP servers**.

| Key | Action |
|---|---|
| `j` / `k` / `↓` / `↑` | next / previous row |
| `l` / `h` / `→` / `←` | next / previous page |
| `Tab` / `Shift-Tab` | next / previous page |
| `Space` | check or uncheck the feed, keyword alert or MCP server under the cursor; on the query field, start typing |
| `a` | check everything on the page, or uncheck it all |
| `n` | go to Keyword alerts and type a query |
| `Enter` | add what's checked |
| `Esc` | close without adding |
| `q` | quit |

While typing a keyword alert:

| Key | Action |
|---|---|
| any letter | type it, `j`, `a`, `n` and `q` included |
| `Backspace` | delete the last letter |
| `Enter` | check the query and stop typing; a preset's query checks the preset |
| `Esc` | stop typing and drop the query |
