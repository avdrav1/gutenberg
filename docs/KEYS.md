# Keys

| Key | Action | Pane |
|---|---|---|
| `j` / `k` | next / previous item | list, sources |
| `gg` / `G` | first / last | list |
| `o` / `Enter` | open article | list |
| `v` | reader view: the full article from its page, or back to the feed's summary | list, article |
| `O` | open the selected article in the browser | list, article |
| `m` | mark the selected article unread, or read again | list |
| `M` | mark read: everything the list shows for the folder or feed chosen in the sources pane and the current view | any |
| `s` | star or unstar the selected article (★ in the list) | list, article |
| `y` | copy the selected article's link | list, article |
| `/` | search every article's title, author and summary as you type | any |
| `r` | refresh selected source; `R` all | any |
| `u` / `S` / `t` / `A` | view unread only / starred / today's (published since midnight) / all; keeps the chosen folder or feed | any |
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

## Add sources screen

Shown on first run, when the store has no sources, and opened with `a`. The left column lists the catalog's categories, then **Keyword alerts** and **MCP servers**.

| Key | Action |
|---|---|
| `j` / `k` / `↓` / `↑` | next / previous row |
| `l` / `h` / `→` / `←` | next / previous page |
| `Tab` / `Shift-Tab` | next / previous page |
| `Space` | check or uncheck the feed or keyword alert under the cursor; on the query field, start typing |
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
