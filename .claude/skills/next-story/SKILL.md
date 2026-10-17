---
name: next-story
description: Pick the next story from docs/BACKLOG.md, plan it, build it on a branch, test it, and open a PR. Use when asked for the next story, next epic, what's next, or to start a story by ID.
---

# Next story

Take one story from the backlog through to a pull request. One story per invocation.

## Pick

1. Read `docs/BACKLOG.md`. Stories are tables under epics, in build order, with a Status column.
2. If the user named an ID, take it. Otherwise take the first story whose status is `todo` and whose epic has no story `blocked`. Skip a story whose "Depends on" isn't `done`.
3. If the user asked for the next *epic*, list its stories and estimated days, and ask which to start, or start the first.
4. Set the story's status to `doing` in `docs/BACKLOG.md` and commit that alone on the new branch.

## Plan

Before code, write the plan in the PR description draft (`.github/PR_DRAFT.md`, not committed):

- The acceptance line from the backlog, verbatim.
- Files to touch, by crate. If more than two crates change, say why; a story usually lives in one.
- Tests to add: unit tests in the crate, a fixture under `tests/fixtures/` if a parser or a source is involved (`/feed-fixture`).
- The benchmark or budget it must respect, if any.
- What a user will see. If nothing, say so; then no changelog line.

Show the plan and ask before building only when the story is ambiguous or touches the schema (migrations can't be undone on users' machines). Otherwise build.

## Build

- Branch: `git checkout -b <id>-<slug>` from an up-to-date `main`.
- Follow `CLAUDE.md`: no network in tests, no literal colors in `ui`, keys documented, changelog line.
- Commit in small steps with messages that say what changed for the reader of `git log`.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` before each commit. For a UI story, launch the app (`cargo run`) under Hyprland and check the acceptance line by eye; take a screenshot with `grim` into `docs/screenshots/<id>.png`.
- If the story needs a schema change: add a migration file, never edit an old one; test it against a copy of a real database if one exists at `~/.local/share/gutenberg/gutenberg.db`.

## Finish

1. Check the acceptance line again against the running code. If it doesn't hold, the story isn't done; say what's missing.
2. Set the status to `review` in `docs/BACKLOG.md`.
3. Push the branch and open a PR with `gh pr create`: title `<id>: <story>`, body = the plan plus what changed and the screenshot.
4. Report in a few lines: the story, the PR link, anything learned that should change a later story (edit the backlog in the same PR).

Never merge. `/ship-it` does that.
