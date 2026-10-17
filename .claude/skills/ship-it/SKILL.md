---
name: ship-it
description: Merge reviewed work, cut a release, build and publish binaries and AUR packages, and verify the install. Use when asked to ship, release, cut a version, publish, or push to main.
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
- Move `Unreleased` under `## <version> — <date>`, set `version` in the workspace `Cargo.toml`, run `cargo update -w` so `Cargo.lock` follows, commit `Release <version>`, tag `v<version>`, push the tag.

## Build and publish

1. The `release` GitHub Actions workflow builds on the tag: `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, stripped, with `cargo auditable`. Watch it with `gh run watch`. If it fails, read the log, fix on a branch, delete the tag, and start over at Version.
2. The workflow attaches tarballs and a `SHA256SUMS` to the GitHub release. Check they are there with `gh release view v<version>`.
3. AUR: the workflow updates `gutenberg-bin` (PKGBUILD pointing at the tarball, `sha256sums` from the file) and `gutenberg` (source build from the tag), regenerates `.SRCINFO`, and pushes to the AUR. If the AUR SSH key isn't configured, say so and give the two commands to run by hand.

## Verify

1. On an Arch machine or in the `archlinux` container: `yay -S gutenberg-bin` (or `makepkg -si` from the AUR clone), then `gutenberg --version` prints the new version.
2. Launch under Hyprland; the window opens with the current theme. Run `omarchy theme set <another>` and confirm the app follows.
3. `gutenberg --unread` prints a number.

If any step fails on the installed package, yank the release (`gh release edit --draft`) and report.

## Report

Five lines: version, what's in it (from the changelog), the release link, whether AUR updated, whether the install check passed. Then the next story the backlog points at.
