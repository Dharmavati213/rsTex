#!/bin/sh
# Download the C side of pdfTeX and kpathsea at the pinned tag into refs/pinned-c/ and verify them
# against tools/pinned-c-refs.txt (git blob hashes).  These are read as a specification only: nothing
# is compiled or linked.  Same tag as tools/fetch_refs.sh; provenance is in docs/PORTING.md section 1.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tag=f26cc5ed05a1f784d1e694fe5b9cfc3ce992c03d   # tag texlive-2026.0
base=https://raw.githubusercontent.com/TeX-Live/texlive-source
cd "$root"
xargs -P 8 -L 1 sh -c '
    hash=$0 path=$1 dest=refs/pinned-c/$1
    if [ ! -f "$dest" ] || [ "$(git hash-object "$dest")" != "$hash" ]; then
        mkdir -p "$(dirname "$dest")"
        curl -sSf -m 60 -o "$dest" "'"$base/$tag"'/$path"
    fi
    if [ "$(git hash-object "$dest")" != "$hash" ]; then
        echo "hash mismatch: $dest" >&2
        exit 255
    fi
' < tools/pinned-c-refs.txt
echo "refs/pinned-c verified"
