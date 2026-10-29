-- Never edit a migration once it has shipped; add a new file instead.
-- Window state the user sets with the mouse or a key and expects back at the
-- next launch (S11.1): one row per setting, the value as text. `config.toml`
-- is the user's and is never rewritten, so what a drag changes lives here.

CREATE TABLE setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
