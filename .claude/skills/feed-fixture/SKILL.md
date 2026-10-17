---
name: feed-fixture
description: Capture a real feed, article page, or MCP tool result as an offline test fixture with a parser test. Use when adding a parser case, reproducing a feed bug, or when a test would otherwise need the network.
---

# Feed fixture

Tests never touch the network. This turns a live thing into a file and a test.

1. Ask for, or take from the request, the URL (feed or page) or the MCP server command and tool call.
2. Fetch it once, from the shell, not from test code: `curl -sL -A "Gutenberg/0.1" <url> -o tests/fixtures/<kind>/<slug>.<ext>`. Kinds: `feeds/`, `pages/`, `mcp/`. Slug = host and a word, like `nytimes-world.xml`. For MCP, run the server and save the JSON-RPC result to `tests/fixtures/mcp/<server>-<tool>.json`.
3. Strip what's private or heavy: tracking params in URLs, inline base64 images over 10 KB, scripts in pages. Keep the structure the bug depends on.
4. Add a `SOURCE` line at the top of a sidecar `<slug>.toml`: the URL, the date fetched, and what the fixture is for ("Atom feed with no updated element").
5. Write the test next to the parser: it loads the fixture and asserts the exact count of articles and the fields of the first one (title, url, published, summary length). For a page, assert the extracted body's first and last 40 characters. For MCP, assert the mapped article shape.
6. Run `cargo test <slug>` and make it pass. If it fails because the parser is wrong, that's the bug; fix it in the same PR or file a story.

Keep each fixture under 200 KB. Over that, trim entries; the point is the shape, not the volume.
