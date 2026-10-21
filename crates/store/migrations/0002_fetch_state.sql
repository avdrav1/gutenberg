-- Never edit a migration once it has shipped; add a new file instead.
-- Background refresh (S2.3): conditional GET validators and backoff.

ALTER TABLE source ADD COLUMN etag          TEXT;
ALTER TABLE source ADD COLUMN last_modified TEXT;
ALTER TABLE source ADD COLUMN checked_at    INTEGER;
ALTER TABLE source ADD COLUMN error         TEXT;
ALTER TABLE source ADD COLUMN failures      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE source ADD COLUMN retry_at      INTEGER;
