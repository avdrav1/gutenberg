-- Never edit a migration once it has shipped; add a new file instead.
-- Full-text search (S3.6). Contentless: the index holds words, not a second
-- copy of the text. `body` is the summary as plain text, which only Rust can
-- make, so Rust fills the index on insert; `migrate.rs` fills it once here
-- for the articles already stored.

CREATE VIRTUAL TABLE article_fts USING fts5(
  title,
  author,
  body,
  content = '',
  contentless_delete = 1,
  tokenize = 'unicode61 remove_diacritics 2'
);

-- Deleting a feed cascades to its articles; their words go with them.
CREATE TRIGGER article_fts_delete AFTER DELETE ON article BEGIN
  DELETE FROM article_fts WHERE rowid = old.id;
END;
