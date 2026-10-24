#!/usr/bin/env bash
# Rebuilds the site's screenshots and trailers from the real window, drawn
# headlessly over made-up sample articles (crates/ui/examples/site_media.rs).
# Needs cargo, ffmpeg and ImageMagick. Writes site/media/; commit what it
# writes, since the deploy serves site/ as it is.
#
#   scripts/site-media.sh [<music>]
#
# The music is "Beautiful" by avdrav (https://youtu.be/6Uzl3LIiVmo), any
# format ffmpeg reads; it isn't in the repository. Without it the trailers
# are silent.
set -euo pipefail

music=${1:-}
# Where the track turns from its quiet opening to full band, in seconds. The
# tour lands it on the theme montage; the hero loops from it.
rise=33.0

root=$(cd "$(dirname "$0")/.." && pwd)
here=$root/site/media
mkdir -p "$here"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

ICED_TEST_BACKEND=tiny-skia cargo run -q --release --manifest-path "$root/Cargo.toml" \
    -p gutenberg-ui --features shots --example site_media -- \
    "$root/tests/fixtures/themes" "$work"

font=$(fc-match -f '%{file}' 'monospace:bold')

# Screenshots: 1600 px wide WebP, plenty for a 2x display at page width.
for png in "$work"/shots/*.png; do
    name=$(basename "$png" .png)
    magick "$png" -resize 1600x -strip -quality 86 "$here/$name.webp"
done

# The concat demuxer needs every frame in one format: 8-bit RGB, as the
# renderer writes them.

# Captions sit in a pill over the bottom of the frame.
caption() { # <frame> <text> <out>
    magick "$1" \
        \( -size 2560x1440 xc:none -fill 'rgba(10,12,20,0.82)' \
           -draw "roundrectangle 330,1236 2230,1376 70,70" \) -composite \
        -font "$font" -pointsize 54 -fill '#f2f4ff' -gravity south \
        -annotate +0+95 "$2" -depth 8 "PNG24:$3"
}

card() { # <out> <title> <line> [<line>]
    magick -size 2560x1440 xc:'#1a1b26' -font "$font" -gravity center \
        -fill '#c0caf5' -pointsize 150 -annotate +0-110 "$2" \
        -fill '#8b93b8' -pointsize 58 -annotate +0+70 "$3" \
        -fill '#7aa2f7' -pointsize 50 -annotate +0+190 "${4:-}" -depth 8 "PNG24:$1"
}

# A timeline line is `<file> <seconds> <caption>`; the concat demuxer
# holds each frame for its seconds.
cut() { # <timeline> <list> <captions: yes|no>
    : > "$2"
    while read -r file seconds text; do
        frame="$work/frames/$file"
        if [[ $3 == yes && $text != - ]]; then
            caption "$frame" "$text" "$work/frames/c-$file"
            frame="$work/frames/c-$file"
        fi
        printf "file '%s'\nduration %s\n" "$frame" "$seconds" >> "$2"
    done < "$1"
    # The demuxer drops the last duration unless the file repeats.
    tail -n 2 "$2" | head -n 1 >> "$2"
}

encode() { # <list> <width> <crf> <seconds> <out> [<extra filter>]
    # -t: the repeated last frame would otherwise stay up twice as long.
    ffmpeg -loglevel error -y -f concat -safe 0 -i "$1" -t "$4" \
        -vf "fps=30,scale=$2:-2:flags=lanczos${6:+,$6},format=yuv420p" \
        -c:v libx264 -preset slow -crf "$3" -tune animation \
        -movflags +faststart -an "$5"
}

score() { # <video> <out> <ffmpeg input options> <audio filter>
    # shellcheck disable=SC2086 # the input options are several words
    ffmpeg -loglevel error -y -i "$1" $3 -i "$music" -map 0:v -map 1:a \
        -af "$4" -c:v copy -c:a aac -b:a 160k -shortest -movflags +faststart "$2"
}

# Short: the hero. No captions; loops, muted until the viewer asks.
cut "$work/short.txt" "$work/short.list" no
seconds() { awk '{ s += $2 } END { print s }' "$1"; }
encode "$work/short.list" 1280 26 "$(seconds "$work/short.txt")" "$here/trailer-short.mp4"
magick "$work/frames/0000.png" -resize 1600x -strip -quality 86 "$here/trailer-poster.webp"
if [[ -n $music ]]; then
    short=$(seconds "$work/short.txt")
    mv "$here/trailer-short.mp4" "$work/short.mp4"
    score "$work/short.mp4" "$here/trailer-short.mp4" "-ss $rise -t $short" \
        "afade=in:d=0.3,afade=out:st=$(awk "BEGIN { print $short - 0.6 }"):d=0.6"
fi

# Long: title card, captioned tour, install card.
card "$work/frames/title.png" "Gutenberg" "A news reader for Omarchy" "fast · keyboard-driven · follows your theme"
card "$work/frames/end.png" "Gutenberg" "curl -fsSL https://gutenbergapp.com/install.sh | sh" "gutenbergapp.com"
{
    echo "title.png 3.0 -"
    cat "$work/long.txt"
    echo "end.png 4.0 -"
} > "$work/long-all.txt"
cut "$work/long-all.txt" "$work/long.list" yes
total=$(seconds "$work/long-all.txt")
encode "$work/long.list" 1920 24 "$total" "$here/trailer.mp4" \
    "fade=in:st=0:d=0.6,fade=out:st=$(awk "BEGIN { print $total - 0.8 }"):d=0.8"
magick "$work/frames/title.png" -resize 1600x -strip -quality 86 "$here/trailer-long-poster.webp"
if [[ -n $music ]]; then
    # When the theme montage starts: the first frame captioned with it.
    montage=$(awk '/ Colors and font / { print s; exit } { s += $2 }' "$work/long-all.txt")
    delay=$(awk "BEGIN { d = $montage - $rise; print (d > 0 ? d : 0) * 1000 }")
    skip=$(awk "BEGIN { d = $rise - $montage; print (d > 0 ? d : 0) }")
    mv "$here/trailer.mp4" "$work/long.mp4"
    score "$work/long.mp4" "$here/trailer.mp4" "-ss $skip" \
        "adelay=$delay:all=1,apad,atrim=0:$total,afade=out:st=$(awk "BEGIN { print $total - 2.5 }"):d=2.5"
fi

du -h "$here"/*.mp4 "$here"/*.webp | sort -k2
