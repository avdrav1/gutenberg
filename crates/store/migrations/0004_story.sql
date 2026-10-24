-- Never edit a migration once it has shipped; add a new file instead.
-- Story grouping (S5.4). An article's story is the id of the story's first
-- grouped article, so one alone is its own story. It stays NULL until the refresh
-- after the article is stored compares its headline with its neighbours'
-- (`core::story`); the articles already stored are compared at the first.

ALTER TABLE article ADD COLUMN story_id INTEGER;
CREATE INDEX article_by_story ON article(story_id);
CREATE INDEX article_ungrouped ON article(sort_at) WHERE story_id IS NULL;
