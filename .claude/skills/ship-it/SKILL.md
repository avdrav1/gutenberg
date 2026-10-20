---
name: ship-it
description: Merge reviewed work, cut a release, build and publish binaries and the install site, and verify the curl install. Use when asked to ship, release, cut a version, publish, or push to main.
---

# Ship it

Take what's on `main` plus any PRs the user names from "reviewed" to "installed on a user's machine." Do all of it without asking again, except where a step says to stop.

## Merge

1. `git fetch origin`. For each PR named (or every open PR labelled `ready`): check CI is green with `gh pr checks`, then `gh pr merge --squash --delete-branch`. A red PR is skipped and reported, never forced.
2. `git checkout main && git pull`. Run `cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo bench --no-run`. A failure here stops the release; fix on a branch and start again.
3. Set merged stories to `done` in `docs/BACKLOG.md` if the PR didn't.

## Version

- Read `CHANGELOG.md`. If `Unreleased` is empty, there is nothing to ship; say so and stop.
- Pick the version: a new epic completed or a schema change → minor; otherwise patch. Before 1.0, minor for anything a user would notice.
- On a branch `release-<version>`: move `Unreleased` under `## <version> — <date>`, set `version` in the workspace `Cargo.toml`, run `cargo update -w` so `Cargo.lock` follows, commit `Release <version>`, open a PR, and merge it once CI is green (`main` takes no direct commits).
- Tag the merge commit `v<version>` on `main` and push the tag. The workflow refuses a tag that doesn't match `Cargo.toml`.

## Build and publish

1. The `release` GitHub Actions workflow builds on the tag: `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, stripped, with `cargo auditable`. Watch it with `gh run watch`. If it fails, read the log, fix on a branch, delete the tag, and start over at Version.
2. The workflow attaches `gutenberg-<target>.tar.gz` for each target and a `SHA256SUMS` to the GitHub release, with notes from the changelog, and attests each tarball's build provenance. Check them with `gh release view v<version>` and `gh attestation verify <tarball> -R avdrav1/gutenberg`.
3. The same run deploys `site/` (the install page and `install.sh`) to GitHub Pages at https://avdrav1.github.io/gutenberg/. Check that the page and `install.sh` return 200.

## Verify

1. On an Omarchy machine: `curl -fsSL https://avdrav1.github.io/gutenberg/install.sh | sh`, then `~/.local/bin/gutenberg --version` prints the new version.
2. Launch under Hyprland; the window opens. Once theme sync ships (E4), run `omarchy theme set <another>` and confirm the app follows.
3. Once S7.1 ships, `gutenberg --unread` prints a number.

If any step fails on the installed package, yank the release (`gh release edit --draft`) and report.

## Report

Five lines: version, what's in it (from the changelog), the release link, whether the site deployed, whether the curl install check passed. Then the next story the backlog points at.
