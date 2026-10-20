#!/usr/bin/env python3
"""Fill a Gutenberg store with N articles, for benchmarks.

    XDG_DATA_HOME=/tmp/bench gutenberg --refresh     # creates the store
    scripts/seed-articles.py /tmp/bench/gutenberg/gutenberg.db 10000

Adds one source and N articles with a summary each, a tenth of them read.
"""
import sqlite3
import sys

path, n = sys.argv[1], int(sys.argv[2])
db = sqlite3.connect(path)
cur = db.execute(
    "INSERT INTO source (kind, title, url, position, created_at) VALUES ('feed', 'Seeded', ?, 0, 0)",
    (f"https://seed.example/feed/{n}",),
)
source = cur.lastrowid
now = 1_792_500_000
db.executemany(
    "INSERT INTO article (source_id, guid, url, title, author, summary_html, published_at, fetched_at, sort_at, read)"
    " VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    (
        (
            source,
            f"seed-{i}",
            f"https://seed.example/{i}",
            f"Headline {i}: a story about the news of the day, long enough to clip in a narrow pane",
            "A. Writer",
            "<p>" + "Summary sentence. " * 12 + "</p>",
            now - i * 300,
            now,
            now - i * 300,
            int(i % 10 == 0),
        )
        for i in range(n)
    ),
)
db.commit()
print(f"seeded {n} articles into {path}")
