-- Never edit a migration once it has shipped; add a new file instead.
-- Desktop notifications (S10.1): one row per article a `notify` rule matched.
-- It's the once-per-story rule (a story with a row never notifies again), the
-- hourly budget's clock (pop-ups sent in the last hour) and the history.
--
-- `state` is 'sent' (shown, alone or in a summary), 'held' (over the budget
-- or in quiet hours, waiting for a summary) or 'seen' (the window had focus,
-- so the list showed it). Rows shown in one pop-up share `popup`;
-- `notification_id` is the desktop's id for it, to replace it later (S10.2).

CREATE TABLE notified (
    article_id      INTEGER PRIMARY KEY REFERENCES article(id) ON DELETE CASCADE,
    story_id        INTEGER NOT NULL,
    reason          TEXT NOT NULL,
    at              INTEGER NOT NULL,
    state           TEXT NOT NULL,
    sent_at         INTEGER,
    popup           INTEGER,
    notification_id INTEGER
);
CREATE INDEX notified_by_story ON notified(story_id);
CREATE INDEX notified_by_sent_at ON notified(sent_at) WHERE sent_at IS NOT NULL;
