-- Never edit a migration once it has shipped; add a new file instead.

CREATE TABLE folder (
  id       INTEGER PRIMARY KEY,
  name     TEXT NOT NULL,
  position INTEGER NOT NULL
);

CREATE TABLE source (
  id         INTEGER PRIMARY KEY,
  folder_id  INTEGER REFERENCES folder(id) ON DELETE SET NULL,
  kind       TEXT NOT NULL CHECK (kind IN ('feed', 'keyword', 'mcp')),
  title      TEXT NOT NULL,
  url        TEXT,
  config     TEXT NOT NULL DEFAULT '{}',
  position   INTEGER NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX source_feed_url ON source(url) WHERE kind = 'feed';

CREATE TABLE article (
  id           INTEGER PRIMARY KEY,
  source_id    INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  guid         TEXT NOT NULL,
  url          TEXT,
  title        TEXT NOT NULL,
  author       TEXT,
  summary_html TEXT,
  published_at INTEGER,
  fetched_at   INTEGER NOT NULL,
  sort_at      INTEGER NOT NULL,
  read         INTEGER NOT NULL DEFAULT 0,
  starred      INTEGER NOT NULL DEFAULT 0,
  UNIQUE (source_id, guid)
);
CREATE INDEX article_by_time   ON article(sort_at DESC, id DESC);
CREATE INDEX article_by_source ON article(source_id, sort_at DESC, id DESC);
CREATE INDEX article_unread    ON article(source_id) WHERE read = 0;
CREATE INDEX article_starred   ON article(sort_at DESC) WHERE starred = 1;
