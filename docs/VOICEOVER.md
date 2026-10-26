# Tour voiceover

The narration on the long trailer, `site/media/trailer.mp4` (62 seconds). `scripts/site-media.sh` speaks it with Kokoro-82M's `af_heart` voice, an Apache-2.0 model run on this machine (the one Cronkite's video uses), and lays each line where its scene's caption comes up. Change a line here and run the script again; only that line is spoken again.

Each line belongs to the scene with the caption in its row, which must match the trailer's caption exactly (`crates/ui/examples/site_media.rs`), or "Title card" or "Install card". The times are only a guide for reading the table; the trailer sets them. A line has about two and a half words a second of room, and the script warns when one runs into the next scene.

The music comes in at 0:15 and swells to full band on the theme montage at 0:48; it's ducked under the voice. The short trailer on the hero loops without sound and has no voiceover.

| Time | Caption | Voiceover |
|---|---|---|
| 0:00 | Title card | Gutenberg. A news reader for Omarchy. |
| 0:03 | Start with 232 hand-picked feeds, or bring your own OPML | Start with two hundred and thirty-two hand-picked feeds, or bring your own. |
| 0:09 | Keyword alerts turn a news search into a source | Or follow anything with a keyword alert. |
| 0:12 | Three panes, all keyboard: j and k to move, o to open | Three panes, all on the keyboard. Move with j and k, open with o. |
| 0:17 | s stars it for later | Star it for later. Folders keep the counts honest. |
| 0:21 | Eight outlets, one story: the list shows it once | When eight outlets run the same story, you see it once. |
| 0:24 | Feeds send a summary | Feeds send a summary. Press v for the whole story, even offline. |
| 0:29 | MCP servers are sources too: pick a tool, read its answers | M C P servers are sources too. Pick a tool, give it its arguments, and what it finds lands in your list, next to your feeds. |
| 0:39 | / searches every article you have, as you type | Search every article you have, as you type. |
| 0:43 | : runs any command by a few letters of its name | Run any command by name. Question mark shows every key. |
| 0:48 | Colors and font follow your Omarchy theme, live | And it wears your Omarchy theme. Change the theme or the font, and Gutenberg follows, live. No account, no tracking: everything stays on your machine. |
| 0:58 | Install card | Get it at gutenberg app dot com. |

Spell out what the voice should say rather than how it's written: "M C P", "gutenberg app dot com".
