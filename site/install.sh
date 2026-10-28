#!/bin/sh
# Install Gutenberg, the news reader for Omarchy.
#
#   curl -fsSL https://gutenbergapp.com/install.sh | sh
#
# Downloads the release tarball for this machine from GitHub Releases,
# checks it against the release's SHA256SUMS, and installs the binary to
# ~/.local/bin. On Linux it adds a desktop entry and an icon in ~/.local/share
# so app launchers list it; on Omarchy it also adds a theme-set and a font-set
# hook, so an open window follows `omarchy theme set` and `omarchy font set`,
# and a bar widget with the unread count (not placed on the bar; add it with
# `omarchy bar`). On macOS it puts Gutenberg.app in ~/Applications, checked
# the same way. Nothing else on the system is touched.
#
# Environment:
#   GUTENBERG_VERSION      a release such as 0.1.0 (default: latest)
#   GUTENBERG_INSTALL_DIR  where the binary goes (default: ~/.local/bin)
#   GUTENBERG_APP_DIR      macOS: where Gutenberg.app goes (default:
#                          ~/Applications; empty for no app)
#   GUTENBERG_BASE_URL     where the tarballs are (for testing a build)

set -eu

REPO="avdrav1/gutenberg"

say() { printf '%s\n' "$*"; }
fail() { printf 'gutenberg install: %s\n' "$*" >&2; exit 1; }

# The whole script is read before anything runs, so a cut-off download
# can't execute half of it.
main() {
    os=$(uname -s)
    case $os in
        Linux)
            case $(uname -m) in
                x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
                aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
                *) fail "no build for $(uname -m); x86_64 and aarch64 are supported" ;;
            esac
            ;;
        Darwin)
            arch=$(uname -m)
            # A terminal running under Rosetta says x86_64 on an Apple
            # silicon Mac; the native build is the one to install.
            [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = 1 ] && arch=arm64
            case $arch in
                arm64) target="aarch64-apple-darwin" ;;
                x86_64) target="x86_64-apple-darwin" ;;
                *) fail "no build for $arch; Apple silicon and Intel Macs are supported" ;;
            esac
            ;;
        *) fail "Linux and macOS only for now (this is $os)" ;;
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
    case $os in
        Linux)
            install_desktop "$dir/gutenberg" "$tmp/gutenberg/share"
            install_hooks "$dir/gutenberg"
            install_bar_widget "$dir/gutenberg" "$tmp/gutenberg/share"
            ;;
        Darwin) install_app "$base" "$target" "$tmp" ;;
    esac
    case ":$PATH:" in
        *":$dir:"*) ;;
        *) say "Add $dir to your PATH to run it as 'gutenberg'." ;;
    esac
}

# The desktop entry and icon, so launchers such as Walker list Gutenberg.
# Releases before 0.20.0 carry no share/; then there's nothing to add.
install_desktop() {
    [ -d "$2" ] || return 0
    data="${XDG_DATA_HOME:-$HOME/.local/share}"
    mkdir -p "$data/applications" "$data/icons/hicolor/scalable/apps"
    cp "$2/icons/hicolor/scalable/apps/gutenberg.svg" "$data/icons/hicolor/scalable/apps/gutenberg.svg"
    # The binary by its full path: launchers don't always have ~/.local/bin
    # on their PATH.
    sed -e "s|^Exec=gutenberg\$|Exec=$(desktop_quote "$1")|" \
        -e "s|^TryExec=gutenberg\$|TryExec=$(sed_escape "$1")|" \
        "$2/applications/gutenberg.desktop" > "$data/applications/gutenberg.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$data/applications" 2>/dev/null || true
    fi
    say "Added Gutenberg to your app launcher ($data/applications/gutenberg.desktop)"
}

# Gutenberg.app in ~/Applications, so Launchpad, Spotlight and the Dock
# have it. The zip is checked against SHA256SUMS like the tarball; a release
# without one (before 0.26.0) installs the binary alone. The app is signed
# ad hoc, not notarized: this download carries no quarantine flag, so it
# opens as it is, and nothing here touches com.apple.quarantine.
install_app() {
    app_dir="${GUTENBERG_APP_DIR-$HOME/Applications}"
    [ -n "$app_dir" ] || return 0
    zip="Gutenberg-$2.zip"
    expected=$(awk -v f="$zip" '$2 == f || $2 == "*" f { print $1 }' "$3/SHA256SUMS")
    [ -n "$expected" ] || return 0
    need ditto
    say "Downloading $zip"
    fetch "$1/$zip" "$3/$zip" || fail "could not download $1/$zip"
    actual=$(sha256 "$3/$zip")
    [ "$expected" = "$actual" ] || fail "checksum mismatch for $zip (expected $expected, got $actual)"
    say "Checksum OK"
    # Unpacked beside its final place and renamed in, as for the binary; an
    # open Gutenberg.app keeps running on its old bundle until it quits.
    staging="$app_dir/.Gutenberg.app.new"
    rm -rf "$staging"
    mkdir -p "$staging"
    ditto -x -k "$3/$zip" "$staging"
    [ -x "$staging/Gutenberg.app/Contents/MacOS/gutenberg" ] || fail "$zip has no Gutenberg.app"
    rm -rf "$app_dir/Gutenberg.app"
    mv "$staging/Gutenberg.app" "$app_dir/Gutenberg.app"
    rmdir "$staging"
    say "Installed Gutenberg.app to $app_dir"
    say "It isn't notarized. This copy opens as it is; one downloaded with a browser is held until you allow it in System Settings > Privacy & Security (Open Anyway)."
}

# The Omarchy bar widget, in the shell's plugin directory; the shell picks it
# up on its own. It isn't put on the bar: `omarchy bar` does that.
install_bar_widget() {
    plugin="$2/omarchy/plugins/gutenberg.unread"
    [ -d "$plugin" ] || return 0
    [ -d "$HOME/.config/omarchy" ] || return 0
    target="$HOME/.config/omarchy/plugins/gutenberg.unread"
    mkdir -p "$target/i18n"
    cp "$plugin/manifest.json" "$target/manifest.json"
    cp "$plugin/i18n/"*.json "$target/i18n/"
    # The binary's path as a QML string.
    path=$(printf '%s' "$1" | sed 's/[\\"]/\\&/g')
    sed "s|@GUTENBERG@|$(sed_escape "$path")|" "$plugin/Unread.qml" > "$target/Unread.qml"
    say "Added an unread count for the Omarchy bar; to show it: omarchy bar put gutenberg.unread"
}

# $1 as one argument of a desktop entry's Exec line.
desktop_quote() {
    case $1 in
        *[[:space:]\"\\\`\$]*) printf '"%s"' "$(printf '%s' "$1" | sed 's/[\\"`$]/\\&/g' | sed_escape_stdin)" ;;
        *) sed_escape "$1" ;;
    esac
}

# $1 safe on the right-hand side of a sed s||| command.
sed_escape() {
    printf '%s' "$1" | sed_escape_stdin
}

sed_escape_stdin() {
    sed 's/[|&\\]/\\&/g'
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
