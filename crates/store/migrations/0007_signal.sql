-- Never edit a migration once it has shipped; add a new file instead.
-- Learned interest (S10.4): what you did with an article, for the score
-- that orders the list by interest. `opened_at` is when you first opened
-- it (in the window or the browser); `skipped_at` is when you marked it
-- read without opening it. Opening wins over a skip. Stars are read from
-- `article` itself. Scrolling past an article records nothing.

CREATE TABLE signal (
    article_id INTEGER PRIMARY KEY REFERENCES article(id) ON DELETE CASCADE,
    opened_at  INTEGER,
    skipped_at INTEGER
);
