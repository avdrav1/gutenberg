#!/bin/sh
# Install Gutenberg, the news reader for Omarchy.
#
#   curl -fsSL https://gutenbergapp.com/install.sh | sh
#
# Downloads the release tarball for this machine from GitHub Releases,
# checks it against the release's SHA256SUMS, and installs the binary to
# ~/.local/bin. On Omarchy it also adds a theme-set and a font-set hook, so
# an open window follows `omarchy theme set` and `omarchy font set`.
# Nothing else on the system is touched.
#
# Environment:
#   GUTENBERG_VERSION      a release such as 0.1.0 (default: latest)
#   GUTENBERG_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   GUTENBERG_BASE_URL     where the tarballs are (for testing a build)

set -eu

REPO="avdrav1/gutenberg"

say() { printf '%s\n' "$*"; }
fail() { printf 'gutenberg install: %s\n' "$*" >&2; exit 1; }

# The whole script is read before anything runs, so a cut-off download
# can't execute half of it.
main() {
    os=$(uname -s)
    [ "$os" = "Linux" ] || fail "Linux only for now (this is $os)"

    case $(uname -m) in
        x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
        aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
        *) fail "no build for $(uname -m); x86_64 and aarch64 are supported" ;;
    esac

    if [ -n "${GUTENBERG_BASE_URL:-}" ]; then
        base="$GUTENBERG_BASE_URL"
    elif [ -n "${GUTENBERG_VERSION:-}" ]; then
        base="https://github.com/$REPO/releases/download/v${GUTENBERG_VERSION#v}"
    else
        base="https://github.com/$REPO/releases/latest/download"
    fi
    dir="${GUTENBERG_INSTALL_DIR:-$HOME/.local/bin}"
    tarball="gutenberg-$target.tar.gz"

    need tar
    if command -v curl >/dev/null 2>&1; then
        fetch() { curl -fsSL --proto '=https,http' --retry 2 -o "$2" "$1"; }
    elif command -v wget >/dev/null 2>&1; then
        fetch() { wget -q -O "$2" "$1"; }
    else
        fail "needs curl or wget"
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM

    say "Downloading $tarball"
    fetch "$base/$tarball" "$tmp/$tarball" || fail "could not download $base/$tarball"
    fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $base/SHA256SUMS"

    expected=$(awk -v f="$tarball" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
    [ -n "$expected" ] || fail "SHA256SUMS has no line for $tarball"
    actual=$(sha256 "$tmp/$tarball")
    [ "$expected" = "$actual" ] || fail "checksum mismatch for $tarball (expected $expected, got $actual)"
    say "Checksum OK"

    tar -xzf "$tmp/$tarball" -C "$tmp"
    [ -f "$tmp/gutenberg/gutenberg" ] || fail "$tarball has no gutenberg/gutenberg"

    mkdir -p "$dir"
    # Copy then rename, so a running gutenberg is never half overwritten.
    cp "$tmp/gutenberg/gutenberg" "$dir/.gutenberg.new"
    chmod 755 "$dir/.gutenberg.new"
    mv -f "$dir/.gutenberg.new" "$dir/gutenberg"

    version=$("$dir/gutenberg" --version 2>/dev/null || true)
    say "Installed ${version:-gutenberg} to $dir/gutenberg"
    install_hooks "$dir/gutenberg"
    case ":$PATH:" in
        *":$dir:"*) ;;
        *) say "Add $dir to your PATH to run it as 'gutenberg'." ;;
    esac
}

# Omarchy runs every file in ~/.config/omarchy/hooks/<name>.d/ after a theme
# or font change. Ours asks an open window to read them again; with no
# window open it does nothing.
install_hooks() {
    hooks="$HOME/.config/omarchy/hooks"
    [ -d "$HOME/.config/omarchy" ] || return 0
    for name in theme-set font-set; do
        mkdir -p "$hooks/$name.d"
        {
            printf '#!/bin/bash\n'
            printf '# Gutenberg: an open window takes the new %s. Installed by install.sh.\n' "${name%-set}"
            printf '%s --reload-theme >/dev/null 2>&1 || true\n' "$(quote "$1")"
        } > "$hooks/$name.d/gutenberg"
        chmod 755 "$hooks/$name.d/gutenberg"
    done
    say "Added Omarchy theme-set and font-set hooks in $hooks"
}

# $1 in single quotes for a shell script.
quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "needs $1"
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        fail "needs sha256sum or shasum to check the download"
    fi
}

main "$@"
